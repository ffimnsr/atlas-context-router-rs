use anyhow::Result;
use atlas_adapters::{AdapterHooks, McpAdapter};
use atlas_core::{GraphToolRequirement, ReadinessOverride, ReadinessVerdict};
use atlas_store_sqlite::Store;

use crate::discovery::{
    tool_get_docs_section, tool_read_file_around_match, tool_read_file_excerpt,
    tool_search_content, tool_search_files, tool_search_templates, tool_search_text_assets,
};
use crate::output::OutputFormat;
use crate::session_events::tool_record_session_event;
use crate::session_tools::{
    tool_compact_session, tool_cross_session_search, tool_feedback_record, tool_get_context_stats,
    tool_get_global_memory, tool_get_session_status, tool_memory_recall, tool_memory_store,
    tool_purge_saved_context, tool_read_saved_context, tool_resume_session,
    tool_save_context_artifact, tool_search_decisions, tool_search_saved_context,
};
use crate::tool_result::{
    ToolErrorCode, ToolErrorPayload, normalize_tool_execution_error, tool_execution_error_value,
};
use crate::wake_up::tool_wake_up;

use super::analysis::{
    tool_analyze_architecture, tool_analyze_dead_code, tool_analyze_dependency,
    tool_analyze_metrics, tool_analyze_patterns, tool_analyze_remove, tool_analyze_safety,
    tool_assess_risk, tool_find_complex_functions, tool_find_duplicates, tool_find_large_functions,
    tool_find_similar_functions, tool_infer_modules, tool_label_components,
};
use super::context_ops::{
    tool_build_graph, tool_detect_changes, tool_explain_change, tool_get_context,
    tool_get_impact_radius, tool_get_minimal_context, tool_get_review_context, tool_update_graph,
};
use super::graph::{
    tool_batch_query_graph, tool_concept_clusters, tool_cross_file_links, tool_explain_query,
    tool_list_graph_stats, tool_query_graph, tool_resolve_symbol, tool_symbol_neighbors,
    tool_traverse_graph,
};
use super::health::{
    tool_broker_status, tool_db_check, tool_debug_graph, tool_doctor, tool_status,
};
use super::inventory::{tool_repo_registry, tool_tool_list, tool_tool_search};
use super::manual::{tool_help, tool_man};
use super::metrics::tool_get_metrics;
use super::postprocess::tool_postprocess_graph;
use super::shared::{
    bool_arg, derive_graph_readiness, derive_graph_readiness_open_failed,
    resolve_repo_scope_selection,
};
use super::symbols::tool_list_symbols;

fn response_file_list(response: &serde_json::Value, pointer: &str) -> Vec<String> {
    response
        .pointer(pointer)
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn response_single_file(response: &serde_json::Value, pointer: &str) -> Vec<String> {
    response
        .pointer(pointer)
        .and_then(|value| value.as_str())
        .map(|value| vec![value.to_owned()])
        .unwrap_or_default()
}

fn response_insight_finding_files(response: &serde_json::Value) -> Vec<String> {
    let mut files = std::collections::BTreeSet::new();
    if let Some(findings) = response.pointer("/structuredContent/findings")
        && let Some(findings) = findings.as_array()
    {
        for finding in findings {
            if let Some(evidence_items) = finding.get("evidence").and_then(|value| value.as_array())
            {
                for evidence in evidence_items {
                    if let Some(file_path) =
                        evidence.get("file_path").and_then(|value| value.as_str())
                    {
                        files.insert(file_path.to_owned());
                    }
                }
            }
        }
    }
    files.into_iter().collect()
}

fn response_query_graph_files(response: &serde_json::Value) -> Vec<String> {
    let mut files = std::collections::BTreeSet::new();

    if let Some(items) = response
        .pointer("/structuredContent/matches")
        .and_then(|value| value.as_array())
    {
        for item in items {
            if let Some(file) = item.get("file").and_then(|value| value.as_str()) {
                files.insert(file.to_owned());
            }
        }
        return files.into_iter().collect();
    }

    let Some(text) = response
        .pointer("/content/0/text")
        .and_then(|value| value.as_str())
    else {
        return Vec::new();
    };
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(text) else {
        return Vec::new();
    };
    let Some(items) = parsed.as_array() else {
        return Vec::new();
    };
    for item in items {
        if let Some(file) = item.get("file").and_then(|value| value.as_str()) {
            files.insert(file.to_owned());
        }
    }
    files.into_iter().collect()
}

fn normalized_contract_tool(name: &str) -> bool {
    matches!(
        name,
        "query_graph"
            | "batch_query_graph"
            | "detect_changes"
            | "get_impact_radius"
            | "get_review_context"
            | "get_minimal_context"
            | "explain_change"
            | "traverse_graph"
            | "get_context"
            | "build_graph"
            | "update_graph"
            | "postprocess_graph"
            | "status"
            | "doctor"
            | "db_check"
            | "debug_graph"
            | "explain_query"
            | "get_session_status"
            | "compact_session"
            | "resume_session"
            | "record_session_event"
            | "wake_up"
            | "search_saved_context"
            | "search_decisions"
            | "read_saved_context"
            | "save_context_artifact"
            | "purge_saved_context"
            | "cross_session_search"
            | "get_global_memory"
            | "memory_store"
            | "memory_recall"
            | "feedback_record"
            | "symbol_neighbors"
            | "cross_file_links"
            | "concept_clusters"
            | "analyze_safety"
            | "analyze_remove"
            | "analyze_dead_code"
            | "analyze_dependency"
            | "resolve_symbol"
            | "search_files"
            | "search_content"
            | "read_file_excerpt"
            | "get_docs_section"
            | "read_file_around_match"
            | "search_templates"
            | "search_text_assets"
            | "repo_registry"
    )
}

fn mirror_metadata_into_structured_content(
    response: &mut serde_json::Value,
    field: &str,
    predicate: impl Fn(&str) -> bool,
) {
    let Some(value) = response.get(field).cloned() else {
        return;
    };
    let Some(tool_name) = response
        .get("structuredContent")
        .and_then(|structured| structured.get("tool"))
        .and_then(|value| value.as_str())
        .map(str::to_owned)
    else {
        return;
    };
    if !predicate(&tool_name) {
        return;
    }
    let Some(structured) = response.get_mut("structuredContent") else {
        return;
    };
    let Some(structured_obj) = structured.as_object_mut() else {
        return;
    };
    structured_obj.insert(field.to_owned(), value);
}

fn blocked_error_code(
    readiness: &atlas_core::GraphReadiness,
    execution_state: atlas_core::GraphExecutionState,
) -> &str {
    if readiness.error_code != "none" {
        return &readiness.error_code;
    }
    match execution_state {
        atlas_core::GraphExecutionState::Missing => "missing_graph_db",
        atlas_core::GraphExecutionState::Partial => "degraded_build",
        atlas_core::GraphExecutionState::Corrupt => match readiness.health_class {
            Some(class) => class.as_str(),
            None => "sqlite_corrupt",
        },
        atlas_core::GraphExecutionState::Stale => "stale_index",
        atlas_core::GraphExecutionState::Fresh => "none",
    }
}

fn blocked_graph_tool_response(
    name: &str,
    readiness: &atlas_core::GraphReadiness,
    execution_state: atlas_core::GraphExecutionState,
    reason: &str,
    suggestions: &[String],
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    let error_code = blocked_error_code(readiness, execution_state);
    let message = atlas_core::graph_health_error_message(error_code);
    let health_class = readiness.health_class.map(|class| class.as_str());
    let recommended_rebuild_command = readiness.recommended_rebuild_command();
    let details = serde_json::json!({
        "blocked": true,
        "error_code": error_code,
        "health_class": health_class,
        "reason": reason,
        "execution_state": execution_state.as_str(),
        "suggestions": suggestions,
        "db_path": &readiness.db_path,
        "repo_root": &readiness.repo_root,
        "quarantine_path": &readiness.quarantine_path,
        "recommended_rebuild_command": recommended_rebuild_command,
    });
    let payload = ToolErrorPayload::new(ToolErrorCode::GraphStale, message)
        .with_tool(name)
        .with_retry_guidance("Run diagnostics or rebuild graph, then retry.")
        .with_details(details);
    let mut response = tool_execution_error_value(output_format, &payload)?;
    response["atlas_readiness"] = serde_json::json!({
        "blocked": true,
        "safe_to_answer": false,
        "execution_state": execution_state.as_str(),
        "health_class": health_class,
        "error_code": error_code,
        "reason": reason,
        "suggestions": suggestions,
        "quarantine_path": &readiness.quarantine_path,
        "recommended_rebuild_command": recommended_rebuild_command,
    });
    response["atlas_freshness"] = serde_json::json!({
        "stale": execution_state == atlas_core::GraphExecutionState::Stale,
        "blocked": true,
        "safe_to_answer": false,
        "execution_state": execution_state.as_str(),
        "reason": "graph facts unavailable; run diagnostics or rebuild before answering from graph state"
    });

    let atlas_readiness = response["atlas_readiness"].clone();
    let atlas_freshness = response["atlas_freshness"].clone();
    if let Some(structured) = response
        .get_mut("structuredContent")
        .and_then(serde_json::Value::as_object_mut)
    {
        structured.insert("ok".to_owned(), serde_json::json!(false));
        structured.insert("blocked".to_owned(), serde_json::json!(true));
        structured.insert("error_code".to_owned(), serde_json::json!(error_code));
        structured.insert("health_class".to_owned(), serde_json::json!(health_class));
        structured.insert(
            "execution_state".to_owned(),
            serde_json::json!(execution_state.as_str()),
        );
        structured.insert(
            "recommended_rebuild_command".to_owned(),
            serde_json::json!(recommended_rebuild_command),
        );
        structured.insert(
            "quarantine_path".to_owned(),
            serde_json::json!(&readiness.quarantine_path),
        );
        structured.insert("atlas_readiness".to_owned(), atlas_readiness);
        structured.insert("atlas_freshness".to_owned(), atlas_freshness);
    }
    Ok(response)
}

fn inject_freshness_warning(
    response: &mut serde_json::Value,
    name: &str,
    repo_root: &str,
    db_path: &str,
) {
    let relevant_files = match name {
        "query_graph" => response_query_graph_files(response),
        "get_context" => response_file_list(response, "/structuredContent/context_files"),
        "get_review_context" | "get_impact_radius" => {
            response_file_list(response, "/structuredContent/change_source/resolved_files")
        }
        "analyze_architecture"
        | "analyze_metrics"
        | "assess_risk"
        | "analyze_patterns"
        | "find_large_functions"
        | "find_complex_functions"
        | "find_similar_functions"
        | "find_duplicates"
        | "infer_modules"
        | "label_components" => response_insight_finding_files(response),
        "get_docs_section" => response_single_file(response, "/file"),
        _ => Vec::new(),
    };

    if let Some(freshness) =
        super::shared::compute_freshness_warning(repo_root, db_path, &relevant_files)
    {
        response["atlas_freshness"] = serde_json::json!(freshness);
    }
}

pub fn call(
    name: &str,
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
) -> Result<serde_json::Value> {
    call_with_worker_threads(name, args, repo_root, db_path, 2)
}

pub(crate) fn call_with_worker_threads(
    name: &str,
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    worker_threads: usize,
) -> Result<serde_json::Value> {
    let started = std::time::Instant::now();
    let result = call_with_worker_threads_inner(name, args, repo_root, db_path, worker_threads);
    atlas_metrics::record_mcp_tool_call(
        name,
        started.elapsed().as_millis() as u64,
        tool_call_succeeded(&result),
    );
    result
}

/// Tool execution errors are returned as `Ok` responses with `isError: true`,
/// so outcome counting must inspect the payload in addition to the `Result`.
fn tool_call_succeeded(result: &Result<serde_json::Value>) -> bool {
    match result {
        Ok(value) => value.get("isError").and_then(serde_json::Value::as_bool) != Some(true),
        Err(_) => false,
    }
}

fn call_with_worker_threads_inner(
    name: &str,
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    worker_threads: usize,
) -> Result<serde_json::Value> {
    let mut adapter = McpAdapter::open(repo_root);
    if let Some(ref mut a) = adapter {
        a.before_command(name);
    }
    let result = call_inner(name, args, repo_root, db_path, worker_threads.max(1));
    if let Some(ref mut a) = adapter {
        a.after_command(name, result.is_ok());
    }
    if result.is_ok() {
        crate::session_tools::emit_session_event_best_effort(name, args, repo_root, db_path);
    }
    result
}

pub(crate) fn is_known_tool_name(name: &str) -> bool {
    match name {
        #[cfg(test)]
        "__test_sleep" | "__test_panic" => true,
        "list_graph_stats"
        | "repo_registry"
        | "tool_list"
        | "tool_search"
        | "tool_help"
        | "man"
        | "query_graph"
        | "batch_query_graph"
        | "get_impact_radius"
        | "get_review_context"
        | "detect_changes"
        | "build_graph"
        | "update_graph"
        | "postprocess_graph"
        | "traverse_graph"
        | "get_minimal_context"
        | "explain_change"
        | "get_context"
        | "analyze_architecture"
        | "analyze_metrics"
        | "assess_risk"
        | "analyze_patterns"
        | "find_large_functions"
        | "find_complex_functions"
        | "find_similar_functions"
        | "find_duplicates"
        | "infer_modules"
        | "label_components"
        | "get_session_status"
        | "compact_session"
        | "resume_session"
        | "record_session_event"
        | "wake_up"
        | "search_saved_context"
        | "search_decisions"
        | "read_saved_context"
        | "save_context_artifact"
        | "get_context_stats"
        | "purge_saved_context"
        | "cross_session_search"
        | "get_global_memory"
        | "memory_store"
        | "memory_recall"
        | "feedback_record"
        | "symbol_neighbors"
        | "cross_file_links"
        | "concept_clusters"
        | "search_files"
        | "search_content"
        | "read_file_excerpt"
        | "get_docs_section"
        | "read_file_around_match"
        | "search_templates"
        | "search_text_assets"
        | "broker_status"
        | "get_metrics"
        | "status"
        | "doctor"
        | "db_check"
        | "debug_graph"
        | "explain_query"
        | "resolve_symbol"
        | "list_symbols"
        | "analyze_safety"
        | "analyze_remove"
        | "analyze_dead_code"
        | "analyze_dependency" => true,
        _ => false,
    }
}

fn call_inner(
    name: &str,
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    worker_threads: usize,
) -> Result<serde_json::Value> {
    #[cfg(test)]
    if name == "__test_sleep" {
        let sleep_ms = args
            .and_then(|value| value.get("sleep_ms"))
            .and_then(|value| value.as_u64())
            .unwrap_or(25);
        let chunk_ms = args
            .and_then(|value| value.get("chunk_ms"))
            .and_then(|value| value.as_u64())
            .filter(|value| *value > 0)
            .unwrap_or(sleep_ms);
        let report_progress = args
            .and_then(|value| value.get("report_progress"))
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let mut elapsed_ms = 0_u64;
        while elapsed_ms < sleep_ms {
            if crate::progress::is_canceled() {
                return Err(anyhow::anyhow!("canceled"));
            }
            let remaining_ms = sleep_ms.saturating_sub(elapsed_ms);
            let this_chunk_ms = remaining_ms.min(chunk_ms.max(1));
            std::thread::sleep(std::time::Duration::from_millis(this_chunk_ms));
            elapsed_ms = elapsed_ms.saturating_add(this_chunk_ms);
            if report_progress {
                let pct = (((elapsed_ms as f64 / sleep_ms.max(1) as f64) * 100.0).round() as u32)
                    .min(100);
                crate::progress::report(&format!("slept {elapsed_ms}/{sleep_ms} ms"), Some(pct));
            }
        }
        if crate::progress::is_canceled() {
            return Err(anyhow::anyhow!("canceled"));
        }
        return crate::tool_result::tool_result_value(
            &serde_json::json!({
                "tool": "__test_sleep",
                "slept_ms": sleep_ms,
            }),
            OutputFormat::Json,
        );
    }

    #[cfg(test)]
    if name == "__test_panic" {
        let msg = args
            .and_then(|value| value.get("message"))
            .and_then(|value| value.as_str())
            .unwrap_or("test panic");
        panic!("{msg}");
    }

    if !is_known_tool_name(name) {
        return Err(anyhow::anyhow!("unknown tool: {name}"));
    }

    let output_format = default_output_format_for_tool(name);

    // Derive canonical readiness once for graph-backed tools.
    // Non-graph tools (file search, session, broker) skip this.
    let requirement = tool_graph_requirement(name);
    let readiness = if requirement.is_some() {
        let r = match Store::open(db_path) {
            Ok(store) => derive_graph_readiness(&store, repo_root, db_path),
            Err(e) => derive_graph_readiness_open_failed(repo_root, db_path, &e.to_string()),
        };
        Some(r)
    } else {
        None
    };

    // Gate blocked tools before invoking them.
    if let (Some(readiness), Some(req)) = (&readiness, requirement) {
        let allow_stale = bool_arg(args, "allow_stale").unwrap_or(false);
        let allow_partial = bool_arg(args, "allow_partial").unwrap_or(false);
        let overrides = ReadinessOverride {
            allow_stale,
            allow_partial,
        };
        if let ReadinessVerdict::Blocked {
            execution_state,
            reason,
            suggestions,
        } = readiness.check_tool(req, overrides)
        {
            let mut blocked = blocked_graph_tool_response(
                name,
                readiness,
                execution_state,
                &reason,
                &suggestions,
                output_format,
            )?;
            inject_provenance(&mut blocked, args, repo_root, db_path);
            mirror_metadata_into_structured_content(
                &mut blocked,
                "atlas_freshness",
                normalized_contract_tool,
            );
            return Ok(blocked);
        }
    }

    let dispatch_result = match name {
        "list_graph_stats" => tool_list_graph_stats(db_path, output_format),
        "repo_registry" => tool_repo_registry(repo_root, output_format),
        "tool_list" => tool_tool_list(args, output_format),
        "tool_search" => tool_tool_search(args, output_format),
        "tool_help" => tool_help(args, output_format),
        "man" => tool_man(args, output_format),
        "query_graph" => tool_query_graph(args, repo_root, db_path, output_format),
        "batch_query_graph" => tool_batch_query_graph(args, repo_root, db_path, output_format),
        "get_impact_radius" => tool_get_impact_radius(args, repo_root, db_path, output_format),
        "get_review_context" => tool_get_review_context(args, repo_root, db_path, output_format),
        "detect_changes" => tool_detect_changes(args, repo_root, db_path, output_format),
        "build_graph" => tool_build_graph(args, repo_root, db_path, output_format),
        "update_graph" => tool_update_graph(args, repo_root, db_path, output_format),
        "postprocess_graph" => tool_postprocess_graph(args, repo_root, db_path, output_format),
        "traverse_graph" => tool_traverse_graph(args, repo_root, db_path, output_format),
        "get_minimal_context" => tool_get_minimal_context(args, repo_root, db_path, output_format),
        "explain_change" => tool_explain_change(args, repo_root, db_path, output_format),
        "get_context" => tool_get_context(args, repo_root, db_path, output_format),
        "analyze_architecture" => {
            tool_analyze_architecture(args, repo_root, db_path, output_format)
        }
        "analyze_metrics" => tool_analyze_metrics(args, repo_root, db_path, output_format),
        "assess_risk" => tool_assess_risk(args, repo_root, db_path, output_format),
        "analyze_patterns" => tool_analyze_patterns(args, repo_root, db_path, output_format),
        "find_large_functions" => {
            tool_find_large_functions(args, repo_root, db_path, output_format)
        }
        "find_complex_functions" => {
            tool_find_complex_functions(args, repo_root, db_path, output_format)
        }
        "find_similar_functions" => {
            tool_find_similar_functions(args, repo_root, db_path, output_format)
        }
        "find_duplicates" => tool_find_duplicates(args, repo_root, db_path, output_format),
        "infer_modules" => tool_infer_modules(args, repo_root, db_path, output_format),
        "label_components" => tool_label_components(args, repo_root, db_path, output_format),
        "get_session_status" => tool_get_session_status(args, repo_root, db_path, output_format),
        "compact_session" => tool_compact_session(args, repo_root, db_path, output_format),
        "resume_session" => tool_resume_session(args, repo_root, db_path, output_format),
        "record_session_event" => {
            tool_record_session_event(args, repo_root, db_path, output_format)
        }
        "wake_up" => tool_wake_up(args, repo_root, db_path, output_format),
        "search_saved_context" => {
            tool_search_saved_context(args, repo_root, db_path, output_format)
        }
        "search_decisions" => tool_search_decisions(args, repo_root, db_path, output_format),
        "read_saved_context" => tool_read_saved_context(args, repo_root, db_path, output_format),
        "save_context_artifact" => {
            tool_save_context_artifact(args, repo_root, db_path, output_format)
        }
        "get_context_stats" => tool_get_context_stats(args, repo_root, db_path, output_format),
        "purge_saved_context" => tool_purge_saved_context(args, repo_root, db_path, output_format),
        "cross_session_search" => {
            tool_cross_session_search(args, repo_root, db_path, output_format)
        }
        "get_global_memory" => tool_get_global_memory(args, repo_root, db_path, output_format),
        "memory_store" => tool_memory_store(args, repo_root, db_path, output_format),
        "memory_recall" => tool_memory_recall(args, repo_root, db_path, output_format),
        "feedback_record" => tool_feedback_record(args, repo_root, db_path, output_format),
        "symbol_neighbors" => tool_symbol_neighbors(args, repo_root, db_path, output_format),
        "cross_file_links" => tool_cross_file_links(args, repo_root, db_path, output_format),
        "concept_clusters" => tool_concept_clusters(args, repo_root, db_path, output_format),
        "search_files" => tool_search_files(args, repo_root, output_format),
        "search_content" => tool_search_content(args, repo_root, output_format),
        "read_file_excerpt" => tool_read_file_excerpt(args, repo_root, output_format),
        "get_docs_section" => tool_get_docs_section(args, repo_root, db_path, output_format),
        "read_file_around_match" => tool_read_file_around_match(args, repo_root, output_format),
        "search_templates" => tool_search_templates(args, repo_root, output_format),
        "search_text_assets" => tool_search_text_assets(args, repo_root, output_format),
        "broker_status" => tool_broker_status(repo_root, db_path, worker_threads, output_format),
        "get_metrics" => tool_get_metrics(output_format),
        "status" => tool_status(repo_root, db_path, output_format),
        "doctor" => tool_doctor(repo_root, db_path, output_format),
        "db_check" => tool_db_check(args, repo_root, db_path, output_format),
        "debug_graph" => tool_debug_graph(args, repo_root, db_path, output_format),
        "explain_query" => tool_explain_query(args, repo_root, db_path, output_format),
        "resolve_symbol" => tool_resolve_symbol(args, repo_root, db_path, output_format),
        "list_symbols" => tool_list_symbols(args, repo_root, db_path, output_format),
        "analyze_safety" => tool_analyze_safety(args, db_path, output_format),
        "analyze_remove" => tool_analyze_remove(args, db_path, output_format),
        "analyze_dead_code" => tool_analyze_dead_code(args, repo_root, db_path, output_format),
        "analyze_dependency" => tool_analyze_dependency(args, db_path, output_format),
        _ => unreachable!("known tool set checked before dispatch"),
    };

    let mut response = match dispatch_result {
        Ok(response) => response,
        Err(error) => return normalize_tool_execution_error(name, output_format, error),
    };

    inject_provenance(&mut response, args, repo_root, db_path);
    inject_freshness_warning(&mut response, name, repo_root, db_path);
    mirror_metadata_into_structured_content(
        &mut response,
        "atlas_freshness",
        normalized_contract_tool,
    );

    // Stamp canonical readiness on graph-backed tool responses.
    if let (Some(readiness), Some(req)) = (&readiness, requirement) {
        let allow_stale = bool_arg(args, "allow_stale").unwrap_or(false);
        let allow_partial = bool_arg(args, "allow_partial").unwrap_or(false);
        let overrides = ReadinessOverride {
            allow_stale,
            allow_partial,
        };
        let verdict = readiness.check_tool(req, overrides);
        let (execution_state, safe_to_answer, warning) = match verdict {
            ReadinessVerdict::Allowed {
                execution_state,
                safe_to_answer,
                warning,
            } => (execution_state, safe_to_answer, warning),
            ReadinessVerdict::Blocked {
                execution_state, ..
            } => (execution_state, false, None),
        };
        let mut atlas_readiness = serde_json::json!({
            "execution_state": execution_state.as_str(),
            "safe_to_answer": safe_to_answer,
            "blocked": false,
            "health_class": readiness.health_class.map(|class| class.as_str()),
            "error_code": &readiness.error_code,
            "quarantine_path": &readiness.quarantine_path,
            "recommended_rebuild_command": readiness.recommended_rebuild_command(),
        });
        if let Some(w) = warning {
            atlas_readiness["warning"] = serde_json::Value::String(w);
        }
        response["atlas_readiness"] = atlas_readiness;
    }

    Ok(response)
}

fn default_output_format_for_tool(_name: &str) -> OutputFormat {
    OutputFormat::Json
}

/// Map a tool name to its [`GraphToolRequirement`] class.
///
/// Returns `None` for tools that do not need graph readiness gating
/// (file search, session tools, broker status, etc.).
fn tool_graph_requirement(name: &str) -> Option<GraphToolRequirement> {
    match name {
        // Symbol lookup: blocked only on Corrupt or Missing; Partial allowed
        // when `allow_partial=true` is set.
        "query_graph" | "batch_query_graph" | "resolve_symbol" | "explain_query"
        | "list_symbols" => Some(GraphToolRequirement::SymbolLookup),
        // Traversal: blocked on Partial, Corrupt, Missing.
        "symbol_neighbors" | "traverse_graph" | "cross_file_links" | "concept_clusters"
        | "list_graph_stats" => Some(GraphToolRequirement::Traversal),
        // Analysis: blocked on Partial, Corrupt, Missing.
        "get_context"
        | "get_impact_radius"
        | "get_review_context"
        | "get_minimal_context"
        | "explain_change"
        | "detect_changes"
        | "analyze_architecture"
        | "analyze_metrics"
        | "assess_risk"
        | "analyze_patterns"
        | "find_large_functions"
        | "find_complex_functions"
        | "find_similar_functions"
        | "find_duplicates"
        | "infer_modules"
        | "label_components"
        | "analyze_safety"
        | "analyze_remove"
        | "analyze_dead_code"
        | "analyze_dependency" => Some(GraphToolRequirement::Analysis),
        // Docs section: reads Markdown heading nodes from the graph DB.
        "get_docs_section" => Some(GraphToolRequirement::SymbolLookup),
        _ => None,
    }
}

fn inject_provenance(
    response: &mut serde_json::Value,
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
) {
    let (indexed_file_count, last_indexed_at) = if let Ok(store) = Store::open(db_path) {
        if let Ok(meta) = store.provenance_meta() {
            (meta.indexed_file_count, meta.last_indexed_at)
        } else {
            (0, None)
        }
    } else {
        (0, None)
    };

    let registry =
        atlas_repo::RepoRegistry::load_or_bootstrap(camino::Utf8Path::new(repo_root)).ok();
    let selected_repo_ids = resolve_repo_scope_selection("dispatch_provenance", args, repo_root)
        .ok()
        .and_then(|resolved| resolved.selection)
        .map(|selection| {
            selection
                .registrations
                .iter()
                .map(|entry| entry.repo_id.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    response["atlas_provenance"] = serde_json::json!({
        "repo_root": repo_root,
        "db_path": db_path,
        "indexed_file_count": indexed_file_count,
        "last_indexed_at": last_indexed_at,
        "registry_root_repo_id": registry.as_ref().map(|registry| registry.root_repo_id.clone()),
        "registry_repo_count": registry.as_ref().map(|registry| registry.registrations.len()),
        "selected_repo_ids": selected_repo_ids,
    });
    mirror_metadata_into_structured_content(response, "atlas_provenance", normalized_contract_tool);
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
