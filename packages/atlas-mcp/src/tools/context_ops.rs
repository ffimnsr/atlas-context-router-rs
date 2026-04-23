use anyhow::{Context, Result};
use atlas_adapters::derive_content_db_path;
use atlas_core::SearchQuery;
use atlas_core::model::{ChangeType, ChangedFile, ContextIntent, ContextRequest, ContextTarget};
use atlas_engine::{BuildOptions, UpdateOptions, UpdateTarget, build_graph, update_graph};
use atlas_repo::{CanonicalRepoPath, DiffTarget, changed_files, find_repo_root};
use atlas_review::{ContextEngine, query_parser};
use atlas_search::semantic as sem;
use atlas_store_sqlite::{BuildFinishStats, GraphBuildState, Store};
use camino::Utf8Path;
use serde::Serialize;
use std::collections::BTreeSet;

use super::shared::{
    bool_arg, error_message, error_suggestions, open_store, parse_mcp_intent, str_arg,
    string_array_arg, tool_result_value, u64_arg,
};
use crate::context::{package_context_result, package_impact};

#[derive(Clone, Copy)]
enum ChangeSourceMode {
    ExplicitFiles,
    BaseRef,
    Staged,
    WorkingTree,
}

impl ChangeSourceMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::ExplicitFiles => "explicit_files",
            Self::BaseRef => "base_ref",
            Self::Staged => "staged",
            Self::WorkingTree => "working_tree",
        }
    }
}

struct ResolvedChangeSource {
    mode: ChangeSourceMode,
    files: Vec<String>,
    changes: Vec<ChangedFile>,
    deleted_files: Vec<String>,
    base: Option<String>,
    staged: bool,
    working_tree: bool,
}

fn normalize_explicit_files(files: Vec<String>) -> Result<Vec<String>> {
    files
        .into_iter()
        .map(|path| {
            CanonicalRepoPath::from_repo_relative(&path)
                .with_context(|| format!("invalid explicit file path '{path}'"))
                .map(|path| path.as_str().to_owned())
        })
        .collect()
}

fn resolve_diff_target(
    base: Option<String>,
    staged: bool,
    working_tree: bool,
) -> (ChangeSourceMode, DiffTarget) {
    if staged {
        (ChangeSourceMode::Staged, DiffTarget::Staged)
    } else if let Some(base_ref) = base {
        (ChangeSourceMode::BaseRef, DiffTarget::BaseRef(base_ref))
    } else {
        let _ = working_tree;
        (ChangeSourceMode::WorkingTree, DiffTarget::WorkingTree)
    }
}

fn resolve_change_source(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    allow_explicit_files: bool,
) -> Result<ResolvedChangeSource> {
    let base = str_arg(args, "base")?.map(str::to_owned);
    let staged = bool_arg(args, "staged").unwrap_or(false);
    let working_tree = bool_arg(args, "working_tree").unwrap_or(false);
    let files = if allow_explicit_files {
        string_array_arg(args, "files")?
    } else {
        Vec::new()
    };

    if !files.is_empty() && (base.is_some() || staged || working_tree) {
        return Err(anyhow::anyhow!(
            "ambiguous change source: provide either files or one of base/staged/working_tree"
        ));
    }
    if staged && working_tree {
        return Err(anyhow::anyhow!(
            "ambiguous change source: staged and working_tree cannot be combined"
        ));
    }
    if base.is_some() && staged {
        return Err(anyhow::anyhow!(
            "ambiguous change source: base and staged cannot be combined"
        ));
    }
    if base.is_some() && working_tree {
        return Err(anyhow::anyhow!(
            "ambiguous change source: base and working_tree cannot be combined"
        ));
    }

    if !files.is_empty() {
        let files = normalize_explicit_files(files)?;
        return Ok(ResolvedChangeSource {
            mode: ChangeSourceMode::ExplicitFiles,
            files,
            changes: Vec::new(),
            deleted_files: Vec::new(),
            base,
            staged,
            working_tree,
        });
    }

    let repo_root_path =
        find_repo_root(Utf8Path::new(repo_root)).context("cannot find git repo root")?;
    let repo_root_path = repo_root_path.as_path();

    let (mode, diff_target) = resolve_diff_target(base.clone(), staged, working_tree);
    let changes =
        changed_files(repo_root_path, &diff_target).context("cannot detect changed files")?;
    let files: Vec<String> = changes
        .iter()
        .filter(|cf| cf.change_type != ChangeType::Deleted)
        .map(|cf| cf.path.clone())
        .collect();
    let deleted_files: Vec<String> = changes
        .iter()
        .filter(|cf| cf.change_type == ChangeType::Deleted)
        .map(|cf| cf.path.clone())
        .collect();

    Ok(ResolvedChangeSource {
        mode,
        files,
        changes,
        deleted_files,
        base,
        staged,
        working_tree,
    })
}

fn inject_change_source_metadata(
    response: &mut serde_json::Value,
    resolved: &ResolvedChangeSource,
) {
    response["atlas_change_source"] = serde_json::json!({
        "mode": resolved.mode.as_str(),
        "resolved_files": &resolved.files,
        "deleted_files": &resolved.deleted_files,
        "base": &resolved.base,
        "staged": resolved.staged,
        "working_tree": resolved.working_tree,
    });
}

pub(super) fn tool_get_impact_radius(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let resolved = resolve_change_source(args, repo_root, true)?;
    let max_depth = u64_arg(args, "max_depth").unwrap_or(5) as u32;
    let max_nodes = u64_arg(args, "max_nodes").unwrap_or(200) as usize;

    let store = open_store(db_path)?;
    let file_refs: Vec<&str> = resolved.files.iter().map(String::as_str).collect();
    let result = store
        .impact_radius(&file_refs, max_depth, max_nodes)
        .context("impact_radius query failed")?;

    let packaged = package_impact(&result, &resolved.files);
    let mut response = tool_result_value(&packaged, output_format)?;
    inject_change_source_metadata(&mut response, &resolved);
    Ok(response)
}

pub(super) fn tool_get_review_context(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let resolved = resolve_change_source(args, repo_root, true)?;
    let max_depth = u64_arg(args, "max_depth").unwrap_or(3) as u32;
    let max_nodes = u64_arg(args, "max_nodes").unwrap_or(200) as usize;

    let store = open_store(db_path)?;
    let engine = ContextEngine::new(&store);
    let request = ContextRequest {
        intent: ContextIntent::Review,
        target: ContextTarget::ChangedFiles {
            paths: resolved.files.clone(),
        },
        max_nodes: Some(max_nodes),
        depth: Some(max_depth),
        ..ContextRequest::default()
    };
    let result = engine.build(&request).context("context engine failed")?;
    let packaged = package_context_result(&result);
    let mut response = tool_result_value(&packaged, output_format)?;
    inject_change_source_metadata(&mut response, &resolved);
    Ok(response)
}

pub(super) fn tool_detect_changes(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let resolved = resolve_change_source(args, repo_root, false)?;
    let changes = &resolved.changes;
    let store_opt = Store::open(db_path).ok();

    #[derive(Serialize)]
    struct ChangedEntry<'a> {
        path: &'a str,
        change_type: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        old_path: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        node_count: Option<usize>,
    }

    let entries: Vec<ChangedEntry<'_>> = changes
        .iter()
        .map(|cf| {
            let node_count = store_opt
                .as_ref()
                .and_then(|s| s.nodes_by_file(&cf.path).ok())
                .map(|ns| ns.len());
            ChangedEntry {
                path: &cf.path,
                change_type: match cf.change_type {
                    atlas_core::ChangeType::Added => "added",
                    atlas_core::ChangeType::Modified => "modified",
                    atlas_core::ChangeType::Deleted => "deleted",
                    atlas_core::ChangeType::Renamed => "renamed",
                    atlas_core::ChangeType::Copied => "copied",
                },
                old_path: cf.old_path.as_deref(),
                node_count,
            }
        })
        .collect();

    let mut response = tool_result_value(&entries, output_format)?;
    inject_change_source_metadata(&mut response, &resolved);
    Ok(response)
}

pub(super) fn tool_build_or_update_graph(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let mode = str_arg(args, "mode")?.unwrap_or("build");
    let repo_root_path =
        find_repo_root(Utf8Path::new(repo_root)).context("cannot find git repo root")?;
    let repo_root_str = repo_root_path.as_str();

    fn build_status_json(db_path: &str, repo_root: &str) -> serde_json::Value {
        let Ok(store) = Store::open(db_path) else {
            return serde_json::Value::Null;
        };
        let Ok(Some(bs)) = store.get_build_status(repo_root) else {
            return serde_json::Value::Null;
        };
        let state_str = match bs.state {
            GraphBuildState::Building => "building",
            GraphBuildState::Built => "built",
            GraphBuildState::BuildFailed => "build_failed",
        };
        serde_json::json!({
            "state": state_str,
            "files_discovered": bs.files_discovered,
            "files_processed": bs.files_processed,
            "files_failed": bs.files_failed,
            "nodes_written": bs.nodes_written,
            "edges_written": bs.edges_written,
            "last_built_at": bs.last_built_at,
            "last_error": bs.last_error,
        })
    }

    if mode == "update" {
        let base = str_arg(args, "base")?.map(str::to_owned);
        let staged = bool_arg(args, "staged").unwrap_or(false);
        let files = string_array_arg(args, "files")?;

        let target = if !files.is_empty() {
            UpdateTarget::Files(files)
        } else if staged {
            UpdateTarget::Staged
        } else if let Some(b) = base {
            UpdateTarget::BaseRef(b)
        } else {
            UpdateTarget::WorkingTree
        };

        let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root))
            .unwrap_or_default();

        if let Ok(s) = Store::open(db_path) {
            let _ = s.begin_build(repo_root_str);
        }

        let update_result = update_graph(
            repo_root_path.as_path(),
            db_path,
            &UpdateOptions {
                fail_fast: false,
                batch_size: config.parse_batch_size(),
                target,
            },
        );

        if let Ok(s) = Store::open(db_path) {
            match &update_result {
                Ok(sum) => {
                    let _ = s.finish_build(
                        repo_root_str,
                        BuildFinishStats {
                            files_discovered: (sum.parsed + sum.deleted + sum.renamed) as i64,
                            files_processed: sum.parsed as i64,
                            files_failed: sum.parse_errors as i64,
                            nodes_written: sum.nodes_updated as i64,
                            edges_written: sum.edges_updated as i64,
                        },
                    );
                }
                Err(e) => {
                    let _ = s.fail_build(repo_root_str, &e.to_string());
                }
            }
        }

        let summary = update_result?;
        tool_result_value(
            &serde_json::json!({
                "mode": "update",
                "deleted": summary.deleted,
                "renamed": summary.renamed,
                "parsed": summary.parsed,
                "skipped_unsupported": summary.skipped_unsupported,
                "parse_errors": summary.parse_errors,
                "nodes_updated": summary.nodes_updated,
                "edges_updated": summary.edges_updated,
                "elapsed_ms": summary.elapsed_ms,
                "build_status": build_status_json(db_path, repo_root_str),
            }),
            output_format,
        )
    } else {
        let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root))
            .unwrap_or_default();

        if let Ok(s) = Store::open(db_path) {
            let _ = s.begin_build(repo_root_str);
        }

        let build_result = build_graph(
            repo_root_path.as_path(),
            db_path,
            &BuildOptions {
                fail_fast: false,
                batch_size: config.parse_batch_size(),
            },
        );

        if let Ok(s) = Store::open(db_path) {
            match &build_result {
                Ok(sum) => {
                    let _ = s.finish_build(
                        repo_root_str,
                        BuildFinishStats {
                            files_discovered: sum.scanned as i64,
                            files_processed: sum.parsed as i64,
                            files_failed: sum.parse_errors as i64,
                            nodes_written: sum.nodes_inserted as i64,
                            edges_written: sum.edges_inserted as i64,
                        },
                    );
                }
                Err(e) => {
                    let _ = s.fail_build(repo_root_str, &e.to_string());
                }
            }
        }

        let summary = build_result?;
        tool_result_value(
            &serde_json::json!({
                "mode": "build",
                "scanned": summary.scanned,
                "skipped_unsupported": summary.skipped_unsupported,
                "skipped_unchanged": summary.skipped_unchanged,
                "parsed": summary.parsed,
                "parse_errors": summary.parse_errors,
                "nodes_inserted": summary.nodes_inserted,
                "edges_inserted": summary.edges_inserted,
                "elapsed_ms": summary.elapsed_ms,
                "build_status": build_status_json(db_path, repo_root_str),
            }),
            output_format,
        )
    }
}

pub(super) fn tool_get_minimal_context(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let base = str_arg(args, "base")?.map(str::to_owned);
    let staged = bool_arg(args, "staged").unwrap_or(false);
    let max_depth = u64_arg(args, "max_depth").unwrap_or(2) as u32;
    let max_nodes = u64_arg(args, "max_nodes").unwrap_or(50) as usize;

    let repo_root_path =
        find_repo_root(Utf8Path::new(repo_root)).context("cannot find git repo root")?;

    let diff_target = if staged {
        DiffTarget::Staged
    } else if let Some(ref b) = base {
        DiffTarget::BaseRef(b.clone())
    } else {
        DiffTarget::WorkingTree
    };

    let changes = changed_files(repo_root_path.as_path(), &diff_target)
        .context("cannot detect changed files")?;

    let changed_file_paths: Vec<String> = changes
        .iter()
        .filter(|cf| cf.change_type != atlas_core::ChangeType::Deleted)
        .map(|cf| cf.path.clone())
        .collect();

    let store = open_store(db_path)?;
    let file_refs: Vec<&str> = changed_file_paths.iter().map(String::as_str).collect();
    let impact = store
        .impact_radius(&file_refs, max_depth, max_nodes)
        .context("impact_radius failed")?;

    let packaged = package_impact(&impact, &changed_file_paths);

    #[derive(Serialize)]
    struct MinimalContext<'a> {
        changed_file_count: usize,
        deleted_file_count: usize,
        changed_files: Vec<&'a str>,
        impact: crate::context::PackagedImpact<'a>,
    }

    let deleted_count = changes
        .iter()
        .filter(|cf| cf.change_type == atlas_core::ChangeType::Deleted)
        .count();

    let ctx = MinimalContext {
        changed_file_count: changed_file_paths.len(),
        deleted_file_count: deleted_count,
        changed_files: changed_file_paths.iter().map(String::as_str).collect(),
        impact: packaged,
    };

    tool_result_value(&ctx, output_format)
}

pub(super) fn tool_explain_change(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let max_depth = u64_arg(args, "max_depth").unwrap_or(5) as u32;
    let max_nodes = u64_arg(args, "max_nodes").unwrap_or(200) as usize;

    let mut files = string_array_arg(args, "files")?;
    if files.is_empty() {
        let staged = bool_arg(args, "staged").unwrap_or(false);
        let base = str_arg(args, "base")?.map(str::to_owned);
        let repo_root_path =
            find_repo_root(Utf8Path::new(repo_root)).context("cannot find git repo root")?;
        let diff_target = if staged {
            DiffTarget::Staged
        } else if let Some(b) = base {
            DiffTarget::BaseRef(b)
        } else {
            DiffTarget::WorkingTree
        };
        let changes = changed_files(repo_root_path.as_path(), &diff_target)
            .context("cannot detect changed files")?;
        files = changes
            .into_iter()
            .filter(|cf| cf.change_type != atlas_core::ChangeType::Deleted)
            .map(|cf| cf.path)
            .collect();
    }

    if files.is_empty() {
        return tool_result_value(&atlas_review::empty_explain_change_summary(), output_format);
    }

    let store = open_store(db_path)?;
    let changes: Vec<atlas_core::model::ChangedFile> = files
        .iter()
        .cloned()
        .map(|path| atlas_core::model::ChangedFile {
            path,
            change_type: atlas_core::ChangeType::Modified,
            old_path: None,
        })
        .collect();
    let summary =
        atlas_review::build_explain_change_summary(&store, &changes, &files, max_depth, max_nodes)
            .context("explain_change summary generation failed")?;

    tool_result_value(&summary, output_format)
}

pub(super) fn tool_get_context(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    use atlas_contentstore::ContentStore;

    let query = str_arg(args, "query")?.map(str::to_owned);
    let file = str_arg(args, "file")?.map(str::to_owned);
    let files = string_array_arg(args, "files")?;
    let intent_override = str_arg(args, "intent")?.map(str::to_owned);
    let max_nodes = u64_arg(args, "max_nodes").map(|n| n as usize);
    let max_edges = u64_arg(args, "max_edges").map(|n| n as usize);
    let max_files = u64_arg(args, "max_files").map(|n| n as usize);
    let max_depth = u64_arg(args, "max_depth").map(|n| n as u32);
    let code_spans = bool_arg(args, "code_spans");
    let tests = bool_arg(args, "tests");
    let imports = bool_arg(args, "imports");
    let neighbors = bool_arg(args, "neighbors");
    let semantic = bool_arg(args, "semantic").unwrap_or(false);
    let include_saved_context = bool_arg(args, "include_saved_context").unwrap_or(false);
    let session_id = str_arg(args, "session_id")?.map(str::to_owned);

    let mut request = if !files.is_empty() {
        let intent = intent_override
            .as_deref()
            .map(parse_mcp_intent)
            .unwrap_or(ContextIntent::Review);
        ContextRequest {
            intent,
            target: ContextTarget::ChangedFiles { paths: files },
            ..ContextRequest::default()
        }
    } else if let Some(path) = file {
        let intent = intent_override
            .as_deref()
            .map(parse_mcp_intent)
            .unwrap_or(ContextIntent::File);
        ContextRequest {
            intent,
            target: ContextTarget::FilePath { path },
            ..ContextRequest::default()
        }
    } else if let Some(q) = query {
        let mut parsed = query_parser::parse_query(&q);
        if let Some(ref ov) = intent_override {
            parsed.intent = parse_mcp_intent(ov);
        }
        parsed
    } else {
        return Err(anyhow::anyhow!(
            "get_context requires one of: 'query', 'file', or 'files'"
        ));
    };

    if max_nodes.is_some() {
        request.max_nodes = max_nodes;
    }
    if max_edges.is_some() {
        request.max_edges = max_edges;
    }
    if max_files.is_some() {
        request.max_files = max_files;
    }
    if max_depth.is_some() {
        request.depth = max_depth;
    }
    if let Some(v) = code_spans {
        request.include_code_spans = v;
    }
    if let Some(v) = tests {
        request.include_tests = v;
    }
    if let Some(v) = imports {
        request.include_imports = v;
    }
    if let Some(v) = neighbors {
        request.include_neighbors = v;
    }
    request.include_saved_context = include_saved_context;
    request.session_id = session_id;

    let store = open_store(db_path)?;

    // --semantic: when target is a SymbolName, run graph-aware semantic search
    // first to resolve the best-matching qualified name, then build context
    // around the resolved node instead of doing a fuzzier name lookup.
    if semantic && let ContextTarget::SymbolName { ref name } = request.target {
        let sq = SearchQuery {
            text: name.clone(),
            limit: 5,
            graph_expand: true,
            graph_max_hops: 1,
            ..Default::default()
        };
        let hits = sem::context_boosted_search(&store, &sq, &[], &[]).unwrap_or_default();
        if let Some(top) = hits.into_iter().next() {
            request.target = ContextTarget::QualifiedName {
                qname: top.node.qualified_name,
            };
        }
    }

    let engine = ContextEngine::new(&store);

    let result = if include_saved_context {
        let content_db = derive_content_db_path(db_path);
        match ContentStore::open(&content_db) {
            Ok(mut cs) => {
                let _ = cs.migrate();
                let engine = engine.with_content_store(&cs);
                engine.build(&request).context("context engine failed")?
            }
            Err(_) => engine.build(&request).context("context engine failed")?,
        }
    } else {
        engine.build(&request).context("context engine failed")?
    };

    let packaged = package_context_result(&result);
    let mut response = tool_result_value(&packaged, output_format)?;
    let context_files: Vec<String> = match &request.target {
        ContextTarget::ChangedFiles { paths } => paths.clone(),
        ContextTarget::FilePath { path } => vec![path.clone()],
        _ => Vec::new(),
    }
    .into_iter()
    .chain(result.files.iter().map(|file| file.path.clone()))
    .chain(result.nodes.iter().map(|node| node.node.file_path.clone()))
    .collect::<BTreeSet<_>>()
    .into_iter()
    .collect();
    response["atlas_context_files"] = serde_json::json!(context_files);

    // Emit applied-controls metadata so agents can inspect what was included/excluded.
    let mut omitted: Vec<&str> = Vec::new();
    if !result.request.include_tests {
        omitted.push("tests");
    }
    if !result.request.include_code_spans {
        omitted.push("code_spans");
    }
    if !result.request.include_neighbors {
        omitted.push("neighbors");
    }
    response["atlas_detail_controls"] = serde_json::json!({
        "max_files": result.request.max_files,
        "max_nodes": result.request.max_nodes,
        "max_edges": result.request.max_edges,
        "code_spans": result.request.include_code_spans,
        "tests": result.request.include_tests,
        "imports": result.request.include_imports,
        "neighbors": result.request.include_neighbors,
        "semantic": semantic,
        "omitted_sections": omitted,
    });

    if result.nodes.is_empty() {
        response["atlas_error_code"] = serde_json::Value::String("node_not_found".to_owned());
        response["atlas_message"] =
            serde_json::Value::String(error_message("node_not_found").to_owned());
        response["atlas_suggestions"] = serde_json::json!(error_suggestions("node_not_found"));
        response["atlas_hint"] = serde_json::Value::String(
            "No graph nodes matched this request. Possible causes: \
             (1) the graph has not been built yet — run build_or_update_graph first; \
             (2) 'query' contained a natural-language phrase instead of a symbol name or \
             qualified name — try a short exact identifier (e.g. 'BalancesTab') or \
             use query_graph with regex for pattern matching; \
             (3) the file path is wrong or the file has no indexed symbols."
                .to_owned(),
        );
    }
    Ok(response)
}
