use anyhow::Result;
use atlas_adapters::{AdapterHooks, McpAdapter};
use atlas_core::{GraphToolRequirement, ReadinessOverride, ReadinessVerdict};
use atlas_store_sqlite::Store;

use crate::discovery_tools::{
    tool_get_docs_section, tool_read_file_around_match, tool_read_file_excerpt,
    tool_search_content, tool_search_files, tool_search_templates, tool_search_text_assets,
};
use crate::output::{OutputFormat, resolve_output_format};
use crate::session_tools::{
    tool_compact_session, tool_cross_session_search, tool_get_context_stats,
    tool_get_global_memory, tool_get_session_status, tool_purge_saved_context,
    tool_read_saved_context, tool_resume_session, tool_save_context_artifact,
    tool_search_decisions, tool_search_saved_context,
};

use super::analysis::{
    tool_analyze_architecture, tool_analyze_dead_code, tool_analyze_dependency,
    tool_analyze_metrics, tool_analyze_patterns, tool_analyze_remove, tool_analyze_safety,
    tool_assess_risk, tool_find_complex_functions, tool_find_large_functions,
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
use super::health::{
    tool_broker_status, tool_db_check, tool_debug_graph, tool_doctor, tool_status,
};
use super::postprocess::tool_postprocess_graph;
use super::shared::{bool_arg, derive_graph_readiness, derive_graph_readiness_open_failed};

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
        "analyze_architecture"
        | "analyze_metrics"
        | "assess_risk"
        | "analyze_patterns"
        | "find_large_functions"
        | "find_complex_functions" => response_file_list(response, "/atlas_result_files"),
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

pub(crate) fn is_known_tool_name(name: &str) -> bool {
    match name {
        #[cfg(test)]
        "__test_sleep" | "__test_panic" => true,
        "list_graph_stats"
        | "query_graph"
        | "batch_query_graph"
        | "get_impact_radius"
        | "get_review_context"
        | "detect_changes"
        | "build_or_update_graph"
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
        | "get_session_status"
        | "compact_session"
        | "resume_session"
        | "search_saved_context"
        | "search_decisions"
        | "read_saved_context"
        | "save_context_artifact"
        | "get_context_stats"
        | "purge_saved_context"
        | "cross_session_search"
        | "get_global_memory"
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
        | "status"
        | "doctor"
        | "db_check"
        | "debug_graph"
        | "explain_query"
        | "resolve_symbol"
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
) -> Result<serde_json::Value> {
    #[cfg(test)]
    if name == "__test_sleep" {
        let sleep_ms = args
            .and_then(|value| value.get("sleep_ms"))
            .and_then(|value| value.as_u64())
            .unwrap_or(25);
        std::thread::sleep(std::time::Duration::from_millis(sleep_ms));
        return Ok(serde_json::json!({ "slept_ms": sleep_ms }));
    }

    #[cfg(test)]
    if name == "__test_panic" {
        let msg = args
            .and_then(|value| value.get("message"))
            .and_then(|value| value.as_str())
            .unwrap_or("test panic");
        panic!("{msg}");
    }

    let output_format = resolve_output_format(args, default_output_format_for_tool(name))?;

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
            let mut blocked = serde_json::json!({
                "content": [{"type": "text", "text": format!("Graph not ready: {reason}")}],
                "isError": true,
                "atlas_readiness": {
                    "execution_state": execution_state.as_str(),
                    "blocked": true,
                    "reason": reason,
                    "suggestions": suggestions,
                },
            });
            inject_provenance(&mut blocked, repo_root, db_path);
            return Ok(blocked);
        }
    }

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
        "get_session_status" => tool_get_session_status(args, repo_root, db_path, output_format),
        "compact_session" => tool_compact_session(args, repo_root, db_path, output_format),
        "resume_session" => tool_resume_session(args, repo_root, db_path, output_format),
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
        "broker_status" => tool_broker_status(repo_root, db_path, output_format),
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
        });
        if let Some(w) = warning {
            atlas_readiness["warning"] = serde_json::Value::String(w);
        }
        response["atlas_readiness"] = atlas_readiness;
    }

    Ok(response)
}

fn default_output_format_for_tool(_name: &str) -> OutputFormat {
    OutputFormat::Toon
}

/// Map a tool name to its [`GraphToolRequirement`] class.
///
/// Returns `None` for tools that do not need graph readiness gating
/// (file search, session tools, broker status, etc.).
fn tool_graph_requirement(name: &str) -> Option<GraphToolRequirement> {
    match name {
        // Symbol lookup: blocked only on Corrupt or Missing; Partial allowed
        // when `allow_partial=true` is set.
        "query_graph" | "batch_query_graph" | "resolve_symbol" | "explain_query" => {
            Some(GraphToolRequirement::SymbolLookup)
        }
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
        | "analyze_safety"
        | "analyze_remove"
        | "analyze_dead_code"
        | "analyze_dependency" => Some(GraphToolRequirement::Analysis),
        // Docs section: reads Markdown heading nodes from the graph DB.
        "get_docs_section" => Some(GraphToolRequirement::SymbolLookup),
        _ => None,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use jsonschema::{Draft, JSONSchema};
    use serde_json::json;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use tempfile::TempDir;

    fn setup_repo() -> (TempDir, PathBuf, String) {
        let dir = tempfile::tempdir().expect("tempdir");
        let src_dir = dir.path().join("src");
        fs::create_dir_all(&src_dir).expect("create src dir");
        fs::write(
            src_dir.join("lib.rs"),
            "pub fn greet() -> &'static str { \"hi\" }\n",
        )
        .expect("write fixture source");
        fs::write(
            dir.path().join("README.md"),
            "# Fixture Repo\n\n## Status\n\nFixture status content.\n",
        )
        .expect("write fixture readme");
        fs::create_dir_all(dir.path().join("config")).expect("create config dir");
        fs::write(dir.path().join("config/app.toml"), "name = \"fixture\"\n")
            .expect("write fixture config");
        fs::create_dir_all(dir.path().join("templates")).expect("create templates dir");
        fs::write(
            dir.path().join("templates/index.html"),
            "<html><body>{{ greet }}</body></html>\n",
        )
        .expect("write fixture template");
        fs::create_dir_all(dir.path().join("queries")).expect("create queries dir");
        fs::write(dir.path().join("queries/example.sql"), "select 1;\n")
            .expect("write fixture sql");
        git(dir.path(), &["init", "--quiet"]);
        git(dir.path(), &["config", "user.name", "Atlas Tests"]);
        git(
            dir.path(),
            &["config", "user.email", "atlas-tests@example.com"],
        );
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "--quiet", "-m", "fixture baseline"]);
        let db_path = dir.path().join(".atlas").join("worldtree.db");
        (dir, db_path.clone(), db_path.to_string_lossy().into_owned())
    }

    fn git(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn every_tool_descriptor_name_routes_through_dispatcher() {
        let (repo_dir, _db_path, db_path) = setup_repo();
        let repo_root = repo_dir.path().to_string_lossy().into_owned();

        for tool in super::super::registry::tool_list()["tools"]
            .as_array()
            .expect("tools array")
        {
            let name = tool["name"].as_str().expect("tool name");
            let result = call(
                name,
                Some(&json!({"output_format": "json"})),
                &repo_root,
                &db_path,
            );
            if let Err(error) = result {
                assert!(
                    !error.to_string().starts_with("unknown tool:"),
                    "descriptor name must dispatch: {name}"
                );
            }
        }
    }

    fn assert_matches_output_schema(name: &str, response: &serde_json::Value, schema: &JSONSchema) {
        let structured = response
            .get("structuredContent")
            .expect("structuredContent")
            .clone();
        assert!(
            structured.is_object(),
            "{name} structuredContent must be object when outputSchema exists"
        );
        if let Err(errors) = schema.validate(&structured) {
            let details = errors
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("\n");
            panic!("{name} output schema mismatch:\n{details}\nvalue={structured:#}");
        }
    }

    fn schema_test_artifact_content() -> String {
        "schema-test artifact payload ".repeat(32)
    }

    fn schema_test_args(name: &str, saved_source_id: &str) -> serde_json::Value {
        match name {
            "list_graph_stats" => json!({"output_format": "json"}),
            "query_graph" => json!({"text": "greet", "output_format": "json"}),
            "batch_query_graph" => json!({"text": "greet", "output_format": "json"}),
            "get_impact_radius" => json!({"files": ["src/lib.rs"], "output_format": "json"}),
            "get_review_context" => json!({"files": ["src/lib.rs"], "output_format": "json"}),
            "detect_changes" => json!({"working_tree": true, "output_format": "json"}),
            "build_or_update_graph" => json!({"mode": "build", "output_format": "json"}),
            "postprocess_graph" => json!({"dry_run": true, "output_format": "json"}),
            "traverse_graph" => {
                json!({"from_qn": "src/lib.rs::fn::greet", "output_format": "json"})
            }
            "get_minimal_context" => json!({"output_format": "json"}),
            "explain_change" => json!({"files": ["src/lib.rs"], "output_format": "json"}),
            "get_context" => json!({"query": "greet", "output_format": "json"}),
            "analyze_architecture" => json!({"output_format": "json"}),
            "analyze_metrics" => json!({"output_format": "json"}),
            "assess_risk" => json!({"symbol": "src/lib.rs::fn::greet", "output_format": "json"}),
            "analyze_patterns" => json!({"output_format": "json"}),
            "find_large_functions" => json!({"output_format": "json"}),
            "find_complex_functions" => json!({"output_format": "json"}),
            "get_session_status" => json!({"output_format": "json"}),
            "compact_session" => json!({"output_format": "json"}),
            "resume_session" => json!({"output_format": "json"}),
            "search_saved_context" => json!({"query": "schema-test", "output_format": "json"}),
            "search_decisions" => json!({"query": "schema-test", "output_format": "json"}),
            "read_saved_context" => json!({"source_id": saved_source_id, "output_format": "json"}),
            "save_context_artifact" => json!({
                "content": schema_test_artifact_content(),
                "label": "schema-test-artifact-second",
                "source_type": "mcp_artifact",
                "content_type": "text/plain",
                "output_format": "json"
            }),
            "get_context_stats" => json!({"output_format": "json"}),
            "purge_saved_context" => json!({"keep_days": 36500, "output_format": "json"}),
            "cross_session_search" => json!({"query": "schema-test", "output_format": "json"}),
            "get_global_memory" => json!({"output_format": "json"}),
            "symbol_neighbors" => {
                json!({"qname": "src/lib.rs::fn::greet", "output_format": "json"})
            }
            "cross_file_links" => json!({"file": "src/lib.rs", "output_format": "json"}),
            "concept_clusters" => json!({"files": ["src/lib.rs"], "output_format": "json"}),
            "search_files" => json!({"pattern": "*.rs", "output_format": "json"}),
            "search_content" => json!({"query": "greet", "output_format": "json"}),
            "read_file_excerpt" => {
                json!({"file": "src/lib.rs", "start_line": 1, "end_line": 1, "output_format": "json"})
            }
            "get_docs_section" => {
                json!({"file": "README.md", "heading": "Status", "output_format": "json"})
            }
            "read_file_around_match" => {
                json!({"file": "src/lib.rs", "query": "greet", "output_format": "json"})
            }
            "search_templates" => json!({"output_format": "json"}),
            "search_text_assets" => json!({"output_format": "json"}),
            "broker_status" => json!({"output_format": "json"}),
            "status" => json!({"output_format": "json"}),
            "doctor" => json!({"output_format": "json"}),
            "db_check" => json!({"output_format": "json"}),
            "debug_graph" => json!({"output_format": "json"}),
            "explain_query" => json!({"text": "greet", "output_format": "json"}),
            "resolve_symbol" => json!({"name": "greet", "output_format": "json"}),
            "analyze_safety" => json!({"symbol": "src/lib.rs::fn::greet", "output_format": "json"}),
            "analyze_remove" => {
                json!({"symbols": ["src/lib.rs::fn::greet"], "output_format": "json"})
            }
            "analyze_dead_code" => json!({"output_format": "json"}),
            "analyze_dependency" => {
                json!({"symbol": "src/lib.rs::fn::greet", "output_format": "json"})
            }
            other => panic!("missing schema test args for {other}"),
        }
    }

    #[test]
    fn tools_with_output_schema_emit_schema_compatible_structured_content() {
        let (repo_dir, _db_path, db_path) = setup_repo();
        let repo_root = repo_dir.path().to_string_lossy().into_owned();

        let build = call(
            "build_or_update_graph",
            Some(&json!({"mode": "build", "output_format": "json"})),
            &repo_root,
            &db_path,
        )
        .expect("build graph");
        let _ = build;

        let saved = call(
            "save_context_artifact",
            Some(&json!({
                "content": schema_test_artifact_content(),
                "label": "schema-test-artifact-seed",
                "source_type": "mcp_artifact",
                "content_type": "text/plain",
                "output_format": "json"
            })),
            &repo_root,
            &db_path,
        )
        .expect("seed saved context artifact");
        let saved_source_id = saved["structuredContent"]["source_id"]
            .as_str()
            .expect("saved source id");

        for tool in super::super::registry::tool_descriptors() {
            let Some(output_schema) = tool.output_schema.as_ref() else {
                continue;
            };
            let schema = JSONSchema::options()
                .with_draft(Draft::Draft202012)
                .compile(output_schema)
                .unwrap_or_else(|error| {
                    panic!("{} output schema should compile: {error}", tool.name)
                });
            let name = tool.name.as_str();
            let args = schema_test_args(name, saved_source_id);
            let value = call(name, Some(&args), &repo_root, &db_path)
                .unwrap_or_else(|error| panic!("{name} should succeed for schema test: {error}"));
            assert_matches_output_schema(name, &value, &schema);
        }
    }
}
