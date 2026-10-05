//! Per-tool parity fixture arguments for registry dispatch tests.
//!
//! Split out of `registry.rs` so the registry test file stays well under the
//! 1,000-line module limit; the dispatch test that consumes it lives in
//! `registry.rs`.

use serde_json::{Value, json};

pub(super) fn parity_args(tool_name: &str, source_id: &str) -> Value {
    match tool_name {
        "list_graph_stats" => json!({}),
        "tool_list" => json!({}),
        "tool_search" => json!({ "query": "query" }),
        "tool_help" => json!({ "name": "query_graph" }),
        "man" => json!({ "namespace": "mcp", "tool_name": "query_graph" }),
        "query_graph" => json!({ "text": "compute" }),
        "batch_query_graph" => json!({
            "items": [{ "text": "compute" }, { "text": "handle_request" }],
            "output_format": "json"
        }),
        "get_impact_radius" => {
            json!({ "change_source": { "kind": "files", "files": ["src/service.rs"] }, "output_format": "json" })
        }
        "get_review_context" => {
            json!({ "change_source": { "kind": "files", "files": ["src/service.rs"] }, "output_format": "json" })
        }
        "detect_changes" => {
            json!({ "change_source": { "kind": "working_tree" }, "output_format": "json" })
        }
        "build_graph" => json!({ "output_format": "json" }),
        "update_graph" => {
            json!({ "change_source": { "kind": "files", "files": ["src/service.rs"] }, "output_format": "json" })
        }
        "postprocess_graph" => {
            json!({ "changed_only": true, "stage": "flows", "dry_run": true, "output_format": "json" })
        }
        "traverse_graph" => {
            json!({ "from_qn": "src/service.rs::fn::compute", "output_format": "json" })
        }
        "get_minimal_context" => {
            json!({ "change_source": { "kind": "working_tree" }, "output_format": "json" })
        }
        "explain_change" => {
            json!({ "change_source": { "kind": "files", "files": ["src/service.rs"] }, "output_format": "json" })
        }
        "get_context" => {
            json!({ "target": { "kind": "query", "query": "compute" }, "output_format": "json" })
        }
        "analyze_architecture" => json!({ "output_format": "json" }),
        "analyze_metrics" => json!({ "output_format": "json" }),
        "assess_risk" => {
            json!({ "symbol": "src/service.rs::fn::compute", "output_format": "json" })
        }
        "analyze_patterns" => json!({ "output_format": "json" }),
        "find_large_functions" => {
            json!({ "threshold": 2, "mode": "large", "output_format": "json" })
        }
        "find_complex_functions" => {
            json!({ "complexity_threshold": 1, "output_format": "json" })
        }
        "find_similar_functions" => {
            json!({ "symbol": "compute", "output_format": "json" })
        }
        "find_duplicates" => json!({ "output_format": "json" }),
        "infer_modules" => json!({ "output_format": "json" }),
        "label_components" => json!({ "output_format": "json" }),
        "get_session_status" => json!({ "output_format": "json" }),
        "compact_session" => json!({ "output_format": "json" }),
        "resume_session" => json!({ "mark_consumed": false, "output_format": "json" }),
        "record_session_event" => {
            json!({ "event": "user-prompt", "payload": { "prompt": "parity" }, "output_format": "json" })
        }
        "wake_up" => json!({ "output_format": "json" }),
        "search_saved_context" => json!({ "query": "parity-seed", "output_format": "json" }),
        "search_decisions" => json!({ "query": "parity-seed", "output_format": "json" }),
        "read_saved_context" => json!({ "source_id": source_id, "output_format": "json" }),
        "repo_registry" => json!({ "output_format": "json" }),
        "save_context_artifact" => json!({
            "content": "parity preview payload".repeat(40),
            "label": "parity-save",
            "output_format": "json"
        }),
        "get_context_stats" => json!({ "output_format": "json" }),
        "purge_saved_context" => json!({ "keep_days": 365, "output_format": "json" }),
        "cross_session_search" => json!({ "query": "parity-seed", "output_format": "json" }),
        "get_global_memory" => json!({ "limit": 5, "output_format": "json" }),
        "memory_store" => json!({ "text": "parity memory body", "output_format": "json" }),
        "memory_recall" => json!({ "query": "parity", "output_format": "json" }),
        "feedback_record" => json!({
            "predicted": "parity dead",
            "actual": "parity alive",
            "analysis_kind": "dead_code",
            "output_format": "json"
        }),
        "symbol_neighbors" => {
            json!({ "qname": "src/service.rs::fn::compute", "output_format": "json" })
        }
        "cross_file_links" => json!({ "file": "src/service.rs", "output_format": "json" }),
        "concept_clusters" => json!({ "files": ["src/service.rs"], "output_format": "json" }),
        "search_files" => json!({ "pattern": "*.rs", "output_format": "json" }),
        "search_content" => json!({ "query": "compute", "output_format": "json" }),
        "read_file_excerpt" => {
            json!({ "file": "src/service.rs", "selector": { "kind": "range", "start_line": 1, "end_line": 3 }, "output_format": "json" })
        }
        "get_docs_section" => {
            json!({ "file": "README.md", "selector": { "kind": "heading", "heading": "document.overview" }, "output_format": "json" })
        }
        "read_file_around_match" => {
            json!({ "file": "src/service.rs", "query": "compute", "output_format": "json" })
        }
        "search_templates" => json!({ "kind": "html", "output_format": "json" }),
        "search_text_assets" => json!({ "kind": "config", "output_format": "json" }),
        "broker_status" => json!({ "output_format": "json" }),
        "get_metrics" => json!({ "output_format": "json" }),
        "status" => json!({ "output_format": "json" }),
        "doctor" => json!({ "output_format": "json" }),
        "db_check" => json!({ "output_format": "json" }),
        "debug_graph" => json!({ "output_format": "json" }),
        "explain_query" => json!({ "text": "compute", "output_format": "json" }),
        "resolve_symbol" => json!({ "name": "compute", "output_format": "json" }),
        "list_symbols" => json!({ "kind": "function", "limit": 5, "output_format": "json" }),
        "analyze_safety" => {
            json!({ "symbol": "src/service.rs::fn::compute", "output_format": "json" })
        }
        "analyze_remove" => {
            json!({ "symbols": ["src/service.rs::fn::compute"], "output_format": "json" })
        }
        "analyze_dead_code" => json!({ "summary": true, "output_format": "json" }),
        "analyze_dependency" => {
            json!({ "symbol": "src/service.rs::fn::compute", "output_format": "json" })
        }
        other => panic!("missing parity args for tool {other}"),
    }
}
