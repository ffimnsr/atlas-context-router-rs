use anyhow::{Context, Result};
use atlas_core::model::{ContextIntent, ContextRequest, ContextTarget};
use atlas_engine::{BuildOptions, UpdateOptions, UpdateTarget, build_graph, update_graph};
use atlas_repo::{DiffTarget, changed_files, find_repo_root};
use atlas_review::{ContextEngine, query_parser};
use atlas_store_sqlite::{BuildFinishStats, GraphBuildState, Store};
use camino::Utf8Path;
use serde::Serialize;

use crate::context::{package_context_result, package_impact};
use crate::session_tools::derive_content_db_path;

use super::shared::{
    bool_arg, error_message, error_suggestions, open_store, parse_mcp_intent, str_arg,
    string_array_arg, tool_result_value, u64_arg,
};

pub(super) fn tool_get_impact_radius(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let files = string_array_arg(args, "files")?;
    if files.is_empty() {
        return Err(anyhow::anyhow!("missing required argument: files"));
    }
    let max_depth = u64_arg(args, "max_depth").unwrap_or(5) as u32;
    let max_nodes = u64_arg(args, "max_nodes").unwrap_or(200) as usize;

    let store = open_store(db_path)?;
    let file_refs: Vec<&str> = files.iter().map(String::as_str).collect();
    let result = store
        .impact_radius(&file_refs, max_depth, max_nodes)
        .context("impact_radius query failed")?;

    let packaged = package_impact(&result, &files);
    tool_result_value(&packaged, output_format)
}

pub(super) fn tool_get_review_context(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let files = string_array_arg(args, "files")?;
    if files.is_empty() {
        return Err(anyhow::anyhow!("missing required argument: files"));
    }
    let max_depth = u64_arg(args, "max_depth").unwrap_or(3) as u32;
    let max_nodes = u64_arg(args, "max_nodes").unwrap_or(200) as usize;

    let store = open_store(db_path)?;
    let engine = ContextEngine::new(&store);
    let request = ContextRequest {
        intent: ContextIntent::Review,
        target: ContextTarget::ChangedFiles { paths: files },
        max_nodes: Some(max_nodes),
        depth: Some(max_depth),
        ..ContextRequest::default()
    };
    let result = engine.build(&request).context("context engine failed")?;
    let packaged = package_context_result(&result);
    tool_result_value(&packaged, output_format)
}

pub(super) fn tool_detect_changes(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let base = str_arg(args, "base")?.map(str::to_owned);
    let staged = bool_arg(args, "staged").unwrap_or(false);

    let repo_root_path =
        find_repo_root(Utf8Path::new(repo_root)).context("cannot find git repo root")?;

    let target = if staged {
        DiffTarget::Staged
    } else if let Some(ref b) = base {
        DiffTarget::BaseRef(b.clone())
    } else {
        DiffTarget::WorkingTree
    };

    let changes =
        changed_files(repo_root_path.as_path(), &target).context("cannot detect changed files")?;
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

    tool_result_value(&entries, output_format)
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
        return tool_result_value(
            &serde_json::json!({
                "risk_level": "low",
                "changed_file_count": 0,
                "changed_symbol_count": 0,
                "changed_by_kind": { "api_change": 0, "signature_change": 0, "internal_change": 0 },
                "changed_symbols": [],
                "impacted_file_count": 0,
                "impacted_node_count": 0,
                "boundary_violations": [],
                "test_impact": { "affected_test_count": 0, "uncovered_symbol_count": 0, "uncovered_symbols": [] },
                "summary": "No changed files detected."
            }),
            output_format,
        );
    }

    let store = open_store(db_path)?;
    let file_refs: Vec<&str> = files.iter().map(String::as_str).collect();
    let base_impact = store
        .impact_radius(&file_refs, max_depth, max_nodes)
        .context("impact_radius query failed")?;

    let advanced = atlas_impact::analyze(base_impact);
    let mut api_count = 0usize;
    let mut sig_count = 0usize;
    let mut internal_count = 0usize;

    #[derive(Serialize)]
    struct ChangedSymbol<'a> {
        qn: &'a str,
        kind: &'a str,
        file: &'a str,
        line: u32,
        change_kind: &'a str,
        lang: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        sig: Option<&'a str>,
    }

    let changed_symbols: Vec<ChangedSymbol<'_>> = advanced
        .scored_nodes
        .iter()
        .filter_map(|sn| sn.change_kind.map(|ck| (&sn.node, ck)))
        .map(|(n, ck)| {
            let ck_str = match ck {
                atlas_core::ChangeKind::ApiChange => {
                    api_count += 1;
                    "api_change"
                }
                atlas_core::ChangeKind::SignatureChange => {
                    sig_count += 1;
                    "signature_change"
                }
                atlas_core::ChangeKind::InternalChange => {
                    internal_count += 1;
                    "internal_change"
                }
            };
            ChangedSymbol {
                qn: &n.qualified_name,
                kind: n.kind.as_str(),
                file: &n.file_path,
                line: n.line_start,
                change_kind: ck_str,
                lang: &n.language,
                sig: n.params.as_deref(),
            }
        })
        .collect();

    let changed_symbol_count = changed_symbols.len();

    #[derive(Serialize)]
    struct BoundaryViolationCompact<'a> {
        kind: &'a str,
        description: &'a str,
        nodes: &'a [String],
    }

    let boundary_violations: Vec<BoundaryViolationCompact<'_>> = advanced
        .boundary_violations
        .iter()
        .map(|bv| BoundaryViolationCompact {
            kind: match bv.kind {
                atlas_core::BoundaryKind::CrossModule => "cross_module",
                atlas_core::BoundaryKind::CrossPackage => "cross_package",
            },
            description: &bv.description,
            nodes: &bv.nodes,
        })
        .collect();

    let affected_test_count = advanced.test_impact.affected_tests.len();
    let uncovered: Vec<&str> = advanced
        .test_impact
        .uncovered_changed_nodes
        .iter()
        .map(|n| n.qualified_name.as_str())
        .collect();
    let uncovered_count = uncovered.len();

    let risk_str = advanced.risk_level.to_string();
    let impacted_file_count = advanced.base.impacted_files.len();
    let impacted_node_count = advanced.base.impacted_nodes.len();

    let mut summary_parts: Vec<String> = Vec::new();
    summary_parts.push(format!("Risk: {}.", risk_str));
    if api_count > 0 {
        summary_parts.push(format!("{} api change(s).", api_count));
    }
    if sig_count > 0 {
        summary_parts.push(format!("{} signature change(s).", sig_count));
    }
    if internal_count > 0 {
        summary_parts.push(format!("{} internal change(s).", internal_count));
    }
    summary_parts.push(format!(
        "Affects {} file(s), {} node(s).",
        impacted_file_count, impacted_node_count
    ));
    if !boundary_violations.is_empty() {
        summary_parts.push(format!(
            "{} boundary violation(s).",
            boundary_violations.len()
        ));
    }
    if uncovered_count > 0 {
        summary_parts.push(format!(
            "{} changed symbol(s) lack test coverage.",
            uncovered_count
        ));
    }
    let summary = summary_parts.join(" ");

    #[derive(Serialize)]
    struct ExplainChangeResult<'a> {
        risk_level: &'a str,
        changed_file_count: usize,
        changed_symbol_count: usize,
        changed_by_kind: serde_json::Value,
        changed_symbols: Vec<ChangedSymbol<'a>>,
        impacted_file_count: usize,
        impacted_node_count: usize,
        boundary_violations: Vec<BoundaryViolationCompact<'a>>,
        test_impact: serde_json::Value,
        summary: &'a str,
    }

    let result = ExplainChangeResult {
        risk_level: &risk_str,
        changed_file_count: files.len(),
        changed_symbol_count,
        changed_by_kind: serde_json::json!({
            "api_change": api_count,
            "signature_change": sig_count,
            "internal_change": internal_count,
        }),
        changed_symbols,
        impacted_file_count,
        impacted_node_count,
        boundary_violations,
        test_impact: serde_json::json!({
            "affected_test_count": affected_test_count,
            "uncovered_symbol_count": uncovered_count,
            "uncovered_symbols": uncovered,
        }),
        summary: &summary,
    };

    tool_result_value(&result, output_format)
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
    let max_depth = u64_arg(args, "max_depth").map(|n| n as u32);
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
    if max_depth.is_some() {
        request.depth = max_depth;
    }
    request.include_saved_context = include_saved_context;
    request.session_id = session_id;

    let store = open_store(db_path)?;
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
