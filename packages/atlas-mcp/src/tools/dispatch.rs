use anyhow::Result;
use atlas_adapters::{AdapterHooks, McpAdapter};
use atlas_store_sqlite::Store;

use crate::discovery_tools::{
    tool_search_content, tool_search_files, tool_search_templates, tool_search_text_assets,
};
use crate::output::{OutputFormat, resolve_output_format};
use crate::session_tools::{
    tool_compact_session, tool_cross_session_search, tool_get_context_stats,
    tool_get_global_memory, tool_get_session_status, tool_purge_saved_context,
    tool_read_saved_context, tool_resume_session, tool_save_context_artifact,
    tool_search_saved_context,
};

use super::analysis::{
    tool_analyze_dead_code, tool_analyze_dependency, tool_analyze_remove, tool_analyze_safety,
};
use super::context_ops::{
    tool_build_or_update_graph, tool_detect_changes, tool_explain_change, tool_get_context,
    tool_get_impact_radius, tool_get_minimal_context, tool_get_review_context,
};
use super::graph::{
    tool_batch_query_graph, tool_concept_clusters, tool_cross_file_links, tool_explain_query,
    tool_list_graph_stats, tool_query_graph, tool_resolve_symbol, tool_symbol_neighbors,
    tool_traverse_graph,
};
use super::health::{tool_db_check, tool_debug_graph, tool_doctor, tool_status};

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

fn inject_freshness_warning(
    response: &mut serde_json::Value,
    name: &str,
    repo_root: &str,
    db_path: &str,
) {
    let relevant_files = match name {
        "query_graph" => response_file_list(response, "/atlas_result_files"),
        "get_context" => response_file_list(response, "/atlas_context_files"),
        "get_review_context" | "get_impact_radius" => {
            response_file_list(response, "/atlas_change_source/resolved_files")
        }
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
    let mut adapter = McpAdapter::open(repo_root);
    if let Some(ref mut a) = adapter {
        a.before_command(name);
    }
    let result = call_inner(name, args, repo_root, db_path);
    if let Some(ref mut a) = adapter {
        a.after_command(name, result.is_ok());
    }
    if result.is_ok() {
        crate::session_tools::emit_session_event_best_effort(name, args, repo_root, db_path);
    }
    result
}

fn call_inner(
    name: &str,
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
) -> Result<serde_json::Value> {
    let output_format = resolve_output_format(args, default_output_format_for_tool(name))?;
    let mut response = match name {
        "list_graph_stats" => tool_list_graph_stats(db_path, output_format),
        "query_graph" => tool_query_graph(args, repo_root, db_path, output_format),
        "batch_query_graph" => tool_batch_query_graph(args, repo_root, db_path, output_format),
        "get_impact_radius" => tool_get_impact_radius(args, repo_root, db_path, output_format),
        "get_review_context" => tool_get_review_context(args, repo_root, db_path, output_format),
        "detect_changes" => tool_detect_changes(args, repo_root, db_path, output_format),
        "build_or_update_graph" => {
            tool_build_or_update_graph(args, repo_root, db_path, output_format)
        }
        "traverse_graph" => tool_traverse_graph(args, repo_root, db_path, output_format),
        "get_minimal_context" => tool_get_minimal_context(args, repo_root, db_path, output_format),
        "explain_change" => tool_explain_change(args, repo_root, db_path, output_format),
        "get_context" => tool_get_context(args, repo_root, db_path, output_format),
        "get_session_status" => tool_get_session_status(args, repo_root, db_path, output_format),
        "compact_session" => tool_compact_session(args, repo_root, db_path, output_format),
        "resume_session" => tool_resume_session(args, repo_root, db_path, output_format),
        "search_saved_context" => {
            tool_search_saved_context(args, repo_root, db_path, output_format)
        }
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
        "symbol_neighbors" => tool_symbol_neighbors(args, repo_root, db_path, output_format),
        "cross_file_links" => tool_cross_file_links(args, repo_root, db_path, output_format),
        "concept_clusters" => tool_concept_clusters(args, repo_root, db_path, output_format),
        "search_files" => tool_search_files(args, repo_root, output_format),
        "search_content" => tool_search_content(args, repo_root, output_format),
        "search_templates" => tool_search_templates(args, repo_root, output_format),
        "search_text_assets" => tool_search_text_assets(args, repo_root, output_format),
        "status" => tool_status(repo_root, db_path, output_format),
        "doctor" => tool_doctor(repo_root, db_path, output_format),
        "db_check" => tool_db_check(args, repo_root, db_path, output_format),
        "debug_graph" => tool_debug_graph(args, repo_root, db_path, output_format),
        "explain_query" => tool_explain_query(args, repo_root, db_path, output_format),
        "resolve_symbol" => tool_resolve_symbol(args, repo_root, db_path, output_format),
        "analyze_safety" => tool_analyze_safety(args, db_path, output_format),
        "analyze_remove" => tool_analyze_remove(args, db_path, output_format),
        "analyze_dead_code" => tool_analyze_dead_code(args, db_path, output_format),
        "analyze_dependency" => tool_analyze_dependency(args, db_path, output_format),
        other => return Err(anyhow::anyhow!("unknown tool: {other}")),
    }?;

    inject_provenance(&mut response, repo_root, db_path);
    inject_freshness_warning(&mut response, name, repo_root, db_path);
    Ok(response)
}

fn default_output_format_for_tool(_name: &str) -> OutputFormat {
    OutputFormat::Toon
}

fn inject_provenance(response: &mut serde_json::Value, repo_root: &str, db_path: &str) {
    let (indexed_file_count, last_indexed_at) = if let Ok(store) = Store::open(db_path) {
        if let Ok(meta) = store.provenance_meta() {
            (meta.indexed_file_count, meta.last_indexed_at)
        } else {
            (0, None)
        }
    } else {
        (0, None)
    };

    response["atlas_provenance"] = serde_json::json!({
        "repo_root": repo_root,
        "db_path": db_path,
        "indexed_file_count": indexed_file_count,
        "last_indexed_at": last_indexed_at,
    });
}
