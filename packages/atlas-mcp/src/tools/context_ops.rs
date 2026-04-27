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
    bool_arg, error_message, error_suggestions, inject_budget_metadata, load_budget_policy,
    open_store, parse_mcp_intent, str_arg, string_array_arg, tool_result_value, u64_arg,
};
use crate::context::{enforce_mcp_response_budget, package_context_result, package_impact};
use crate::session_tools::{
    decision_hits_json, record_mcp_decision_best_effort, search_decisions_best_effort,
};

fn context_ranking_evidence_legend_json() -> serde_json::Value {
    atlas_core::context_ranking_evidence_legend()
}

fn context_decision_lookup_query(request: &ContextRequest) -> Option<String> {
    match &request.target {
        ContextTarget::QualifiedName { qname } => Some(qname.clone()),
        ContextTarget::SymbolName { name } => Some(name.clone()),
        ContextTarget::FilePath { path } => Some(path.clone()),
        ContextTarget::ChangedFiles { paths } => {
            let joined = paths.iter().take(3).cloned().collect::<Vec<_>>().join(" ");
            (!joined.is_empty()).then_some(joined)
        }
        ContextTarget::ChangedSymbols { qnames } => {
            let joined = qnames.iter().take(3).cloned().collect::<Vec<_>>().join(" ");
            (!joined.is_empty()).then_some(joined)
        }
        ContextTarget::EdgeQuerySeed { source_qname, .. } => Some(source_qname.clone()),
    }
}

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

fn metadata_reserve_bytes(value: &serde_json::Value) -> usize {
    serde_json::to_vec(value)
        .map(|bytes| bytes.len() + 256)
        .unwrap_or(256)
}

fn trim_context_response_metadata(response: &mut serde_json::Value, max_bytes: usize) {
    while serde_json::to_vec(response)
        .map(|bytes| bytes.len())
        .unwrap_or(0)
        > max_bytes
    {
        if response
            .get("atlas_context_ranking_evidence_legend")
            .is_some()
        {
            response
                .as_object_mut()
                .expect("response object")
                .remove("atlas_context_ranking_evidence_legend");
            continue;
        }

        let removed_context_file = response
            .get_mut("atlas_context_files")
            .and_then(serde_json::Value::as_array_mut)
            .is_some_and(|files| {
                if files.is_empty() {
                    false
                } else {
                    files.pop();
                    true
                }
            });
        if removed_context_file {
            continue;
        }

        let removed_omitted_section = response
            .get_mut("atlas_detail_controls")
            .and_then(serde_json::Value::as_object_mut)
            .and_then(|controls| controls.get_mut("omitted_sections"))
            .and_then(serde_json::Value::as_array_mut)
            .is_some_and(|sections| {
                if sections.is_empty() {
                    false
                } else {
                    sections.pop();
                    true
                }
            });
        if removed_omitted_section {
            continue;
        }

        if response.get("atlas_context_files").is_some() {
            response
                .as_object_mut()
                .expect("response object")
                .remove("atlas_context_files");
            continue;
        }

        if response.get("atlas_detail_controls").is_some() {
            response
                .as_object_mut()
                .expect("response object")
                .remove("atlas_detail_controls");
            continue;
        }

        let removed_change_source_file = response
            .get_mut("atlas_change_source")
            .and_then(serde_json::Value::as_object_mut)
            .and_then(|source| source.get_mut("resolved_files"))
            .and_then(serde_json::Value::as_array_mut)
            .is_some_and(|files| {
                if files.is_empty() {
                    false
                } else {
                    files.pop();
                    true
                }
            });
        if removed_change_source_file {
            continue;
        }

        let removed_deleted_file = response
            .get_mut("atlas_change_source")
            .and_then(serde_json::Value::as_object_mut)
            .and_then(|source| source.get_mut("deleted_files"))
            .and_then(serde_json::Value::as_array_mut)
            .is_some_and(|files| {
                if files.is_empty() {
                    false
                } else {
                    files.pop();
                    true
                }
            });
        if removed_deleted_file {
            continue;
        }

        if response.get("atlas_change_source").is_some() {
            response
                .as_object_mut()
                .expect("response object")
                .remove("atlas_change_source");
            continue;
        }

        break;
    }
}

fn ensure_final_response_budget(
    response: &mut serde_json::Value,
    max_bytes: usize,
) -> Result<usize> {
    trim_context_response_metadata(response, max_bytes);
    let emitted_bytes = serde_json::to_vec(response)
        .map(|bytes| bytes.len())
        .unwrap_or(0);
    if emitted_bytes > max_bytes {
        anyhow::bail!(
            "MCP response exceeds max_mcp_response_bytes after metadata trimming (emitted={emitted_bytes}, limit={max_bytes})"
        );
    }
    Ok(emitted_bytes)
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
    let policy = load_budget_policy(repo_root)?;
    let file_refs: Vec<&str> = resolved.files.iter().map(String::as_str).collect();
    let result = store
        .impact_radius(
            &file_refs,
            max_depth,
            max_nodes,
            policy.graph_traversal.edges.default_limit,
        )
        .context("impact_radius query failed")?;

    let packaged = package_impact(&result, &resolved.files);
    let mut response = tool_result_value(&packaged, output_format)?;
    inject_budget_metadata(&mut response, &result.budget);
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
    let token_budget = u64_arg(args, "token_budget").map(|n| n as usize);

    let store = open_store(db_path)?;
    let policy = load_budget_policy(repo_root)?;
    let engine = ContextEngine::new(&store).with_budget_policy(policy);
    let request = ContextRequest {
        intent: ContextIntent::Review,
        target: ContextTarget::ChangedFiles {
            paths: resolved.files.clone(),
        },
        max_nodes: Some(max_nodes),
        depth: Some(max_depth),
        token_budget,
        ..ContextRequest::default()
    };
    let result = engine.build(&request).context("context engine failed")?;
    let include_context_ranking_evidence = output_format == crate::output::OutputFormat::Json;
    let packaged = package_context_result(&result, include_context_ranking_evidence);
    let mut packaged_value = serde_json::to_value(&packaged)?;
    let response_budget_limit = policy
        .mcp_cli_payload_serialization
        .mcp_response_bytes
        .default_limit;
    let response_budget_limit =
        response_budget_limit.saturating_sub(metadata_reserve_bytes(&serde_json::json!({
            "atlas_change_source": {
                "mode": resolved.mode.as_str(),
                "resolved_files": &resolved.files,
                "deleted_files": &resolved.deleted_files,
                "base": &resolved.base,
                "staged": resolved.staged,
                "working_tree": resolved.working_tree,
            }
        })));
    let stage_budget = if let Some(response_budget) =
        enforce_mcp_response_budget(&mut packaged_value, output_format, response_budget_limit)?
    {
        result.budget.clone().merge(response_budget)
    } else {
        result.budget.clone()
    };
    let mut response = tool_result_value(&packaged_value, output_format)?;
    if include_context_ranking_evidence {
        response["atlas_context_ranking_evidence_legend"] = context_ranking_evidence_legend_json();
    }
    inject_budget_metadata(&mut response, &stage_budget);
    inject_change_source_metadata(&mut response, &resolved);
    let _ = ensure_final_response_budget(
        &mut response,
        policy
            .mcp_cli_payload_serialization
            .mcp_response_bytes
            .default_limit,
    )?;
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
                    ChangeType::Added => "added",
                    ChangeType::Modified => "modified",
                    ChangeType::Deleted => "deleted",
                    ChangeType::Renamed => "renamed",
                    ChangeType::Copied => "copied",
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
            GraphBuildState::Degraded => "degraded",
            GraphBuildState::BuildFailed => "build_failed",
        };
        serde_json::json!({
            "state": state_str,
            "files_discovered": bs.files_discovered,
            "files_processed": bs.files_processed,
            "files_accepted": bs.files_accepted,
            "files_skipped_by_byte_budget": bs.files_skipped_by_byte_budget,
            "files_failed": bs.files_failed,
            "bytes_accepted": bs.bytes_accepted,
            "bytes_skipped": bs.bytes_skipped,
            "nodes_written": bs.nodes_written,
            "edges_written": bs.edges_written,
            "budget_stop_reason": bs.budget_stop_reason,
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
        let build_budget = config.build_run_budget()?;

        if let Ok(s) = Store::open(db_path) {
            let _ = s.begin_build(repo_root_str);
        }

        crate::progress::report("detecting changed files", None);
        if crate::progress::is_canceled() {
            return Err(anyhow::anyhow!("canceled"));
        }
        crate::progress::report("updating graph", Some(10));

        let update_result = update_graph(
            repo_root_path.as_path(),
            db_path,
            &UpdateOptions {
                fail_fast: false,
                batch_size: config.parse_batch_size(),
                target,
                budget: build_budget,
            },
        );

        if let Ok(s) = Store::open(db_path) {
            match &update_result {
                Ok(sum) => {
                    let state =
                        if matches!(sum.budget.budget_status, atlas_core::BudgetStatus::Blocked) {
                            GraphBuildState::BuildFailed
                        } else if sum.budget.partial {
                            GraphBuildState::Degraded
                        } else {
                            GraphBuildState::Built
                        };
                    let _ = s.finish_build(
                        repo_root_str,
                        BuildFinishStats {
                            state,
                            files_discovered: (sum.parsed + sum.deleted + sum.renamed) as i64,
                            files_processed: sum.parsed as i64,
                            files_accepted: sum.budget_counters.files_accepted as i64,
                            files_skipped_by_byte_budget: sum
                                .budget_counters
                                .files_skipped_by_byte_budget
                                as i64,
                            files_failed: sum.parse_errors as i64,
                            bytes_accepted: sum.budget_counters.bytes_accepted as i64,
                            bytes_skipped: sum.budget_counters.bytes_skipped as i64,
                            nodes_written: sum.nodes_updated as i64,
                            edges_written: sum.edges_updated as i64,
                            budget_stop_reason: sum.budget_counters.budget_stop_reason.clone(),
                        },
                    );
                }
                Err(e) => {
                    let _ = s.fail_build(repo_root_str, &e.to_string());
                }
            }
        }

        crate::progress::report("writing results", Some(90));
        let summary = update_result?;
        crate::progress::report("update complete", Some(100));
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
                "budget": summary.budget,
                "budget_counters": summary.budget_counters,
                "elapsed_ms": summary.elapsed_ms,
                "build_status": build_status_json(db_path, repo_root_str),
            }),
            output_format,
        )
    } else {
        let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root))
            .unwrap_or_default();
        let build_budget = config.build_run_budget()?;

        if let Ok(s) = Store::open(db_path) {
            let _ = s.begin_build(repo_root_str);
        }

        crate::progress::report("scanning repository files", None);
        if crate::progress::is_canceled() {
            return Err(anyhow::anyhow!("canceled"));
        }
        crate::progress::report("building graph", Some(10));

        let build_result = build_graph(
            repo_root_path.as_path(),
            db_path,
            &BuildOptions {
                fail_fast: false,
                batch_size: config.parse_batch_size(),
                budget: build_budget,
            },
        );

        if let Ok(s) = Store::open(db_path) {
            match &build_result {
                Ok(sum) => {
                    let state =
                        if matches!(sum.budget.budget_status, atlas_core::BudgetStatus::Blocked) {
                            GraphBuildState::BuildFailed
                        } else if sum.budget.partial {
                            GraphBuildState::Degraded
                        } else {
                            GraphBuildState::Built
                        };
                    let _ = s.finish_build(
                        repo_root_str,
                        BuildFinishStats {
                            state,
                            files_discovered: sum.scanned as i64,
                            files_processed: sum.parsed as i64,
                            files_accepted: sum.budget_counters.files_accepted as i64,
                            files_skipped_by_byte_budget: sum
                                .budget_counters
                                .files_skipped_by_byte_budget
                                as i64,
                            files_failed: sum.parse_errors as i64,
                            bytes_accepted: sum.budget_counters.bytes_accepted as i64,
                            bytes_skipped: sum.budget_counters.bytes_skipped as i64,
                            nodes_written: sum.nodes_inserted as i64,
                            edges_written: sum.edges_inserted as i64,
                            budget_stop_reason: sum.budget_counters.budget_stop_reason.clone(),
                        },
                    );
                }
                Err(e) => {
                    let _ = s.fail_build(repo_root_str, &e.to_string());
                }
            }
        }

        crate::progress::report("writing results", Some(90));
        let summary = build_result?;
        crate::progress::report("build complete", Some(100));
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
                "budget": summary.budget,
                "budget_counters": summary.budget_counters,
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
    let policy = load_budget_policy(repo_root)?;
    let file_refs: Vec<&str> = changed_file_paths.iter().map(String::as_str).collect();
    let impact = store
        .impact_radius(
            &file_refs,
            max_depth,
            max_nodes,
            policy.graph_traversal.edges.default_limit,
        )
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

    let mut response = tool_result_value(&ctx, output_format)?;
    inject_budget_metadata(&mut response, &impact.budget);
    Ok(response)
}

pub(super) fn tool_explain_change(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let policy = load_budget_policy(repo_root)?;
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
    let summary = atlas_review::build_explain_change_summary(
        &store, &changes, &files, max_depth, max_nodes, &policy,
    )
    .context("explain_change summary generation failed")?;

    tool_result_value(&summary, output_format)
}

pub(super) fn tool_get_context(
    args: Option<&serde_json::Value>,
    repo_root: &str,
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
    let token_budget = u64_arg(args, "token_budget").map(|n| n as usize);

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
    if token_budget.is_some() {
        request.token_budget = token_budget;
    }

    let store = open_store(db_path)?;
    let policy = load_budget_policy(repo_root)?;

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

    let engine = ContextEngine::new(&store).with_budget_policy(policy);

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

    let include_context_ranking_evidence = output_format == crate::output::OutputFormat::Json;
    let packaged = package_context_result(&result, include_context_ranking_evidence);
    let mut packaged_value = serde_json::to_value(&packaged)?;
    let linked_decisions = context_decision_lookup_query(&request)
        .map(|query| {
            let hits = search_decisions_best_effort(
                repo_root,
                db_path,
                request.session_id.as_deref(),
                &query,
                3,
            );
            (query, hits)
        })
        .filter(|(_, hits)| !hits.is_empty());
    if let Some((query, hits)) = linked_decisions.as_ref() {
        packaged_value["linked_decisions"] = decision_hits_json(hits);
        packaged_value["decision_lookup_query"] = serde_json::Value::String(query.clone());
    }
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

    let response_budget_limit = policy
        .mcp_cli_payload_serialization
        .mcp_response_bytes
        .default_limit;
    let response_budget_limit =
        response_budget_limit.saturating_sub(metadata_reserve_bytes(&serde_json::json!({
            "atlas_context_files": &context_files,
            "atlas_detail_controls": {
                "max_files": result.request.max_files,
                "max_nodes": result.request.max_nodes,
                "max_edges": result.request.max_edges,
                "code_spans": result.request.include_code_spans,
                "tests": result.request.include_tests,
                "imports": result.request.include_imports,
                "neighbors": result.request.include_neighbors,
                "semantic": semantic,
                "omitted_sections": &omitted,
            }
        })));
    let mut stage_budget = if let Some(response_budget) =
        enforce_mcp_response_budget(&mut packaged_value, output_format, response_budget_limit)?
    {
        result.budget.clone().merge(response_budget)
    } else {
        result.budget.clone()
    };
    let mut response = tool_result_value(&packaged_value, output_format)?;
    if include_context_ranking_evidence {
        response["atlas_context_ranking_evidence_legend"] = context_ranking_evidence_legend_json();
    }
    response["atlas_context_files"] = serde_json::json!(context_files);

    // Emit applied-controls metadata so agents can inspect what was included/excluded.
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
    let response_bytes_before_trim = serde_json::to_vec(&response)
        .map(|bytes| bytes.len())
        .unwrap_or(0);
    trim_context_response_metadata(
        &mut response,
        policy
            .mcp_cli_payload_serialization
            .mcp_response_bytes
            .default_limit,
    );
    let response_bytes_after_trim = serde_json::to_vec(&response)
        .map(|bytes| bytes.len())
        .unwrap_or(0);
    if response_bytes_after_trim < response_bytes_before_trim {
        stage_budget = stage_budget.merge(atlas_core::BudgetReport::partial_result(
            "mcp_cli_payload_serialization.max_mcp_response_bytes",
            policy
                .mcp_cli_payload_serialization
                .mcp_response_bytes
                .default_limit,
            response_bytes_before_trim,
            true,
        ));
    }
    inject_budget_metadata(&mut response, &stage_budget);

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
    if let Some((query, hits)) = linked_decisions {
        let source_ids = hits
            .iter()
            .flat_map(|hit| hit.decision.source_ids.iter().cloned())
            .take(5)
            .collect::<Vec<_>>();
        record_mcp_decision_best_effort(
            repo_root,
            db_path,
            &format!("reuse prior decision for context: {query}"),
            Some("stored decision memory matched current context request"),
            serde_json::json!({
                "query": query,
                "conclusion": "prior decision reused for context request",
                "source_ids": source_ids,
                "evidence": hits.iter().take(3).map(|hit| serde_json::json!({
                    "decision_id": hit.decision.decision_id,
                    "summary": hit.decision.summary,
                    "relevance_score": hit.relevance_score,
                })).collect::<Vec<_>>(),
            }),
        );
    }
    let _ = ensure_final_response_budget(
        &mut response,
        policy
            .mcp_cli_payload_serialization
            .mcp_response_bytes
            .default_limit,
    )?;
    Ok(response)
}
