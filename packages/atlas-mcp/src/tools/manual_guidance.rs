//! Per-tool usage guidance content served by the manual renderer.
//!
//! Content-only tables: usage guidance lines, tailored few-shot prompts, and
//! concrete target-tool call examples. Assembly machinery and the schema-derived
//! fallbacks stay in `manual.rs`; this module only maps tool names to text.

use serde_json::{Value, json};

use super::manual::{MAX_EXAMPLE_CHARS, truncate_text};
use super::registry::is_state_changing_tool;

pub(crate) fn tool_usage_guidance(tool_name: &str) -> Vec<String> {
    let mut usage = match tool_name {
        "status" | "doctor" | "db_check" | "debug_graph" | "list_graph_stats" | "get_metrics"
        | "broker_status" => vec![
            "Use before trusting graph-backed context or when graph results look empty/stale.",
            "Read health fields first; only rebuild or run deeper diagnostics when readiness says unsafe or stale.",
        ],
        "query_graph" | "batch_query_graph" | "resolve_symbol" => vec![
            "Use for symbol/name resolution before relationship tools or file reads.",
            "Pass short exact identifiers or qualified names; avoid natural-language-only phrases unless documented by query grammar.",
        ],
        "explain_query" => vec![
            "Use to diagnose empty `query_graph` results or validate a regex pattern without executing the search; it reports tokenisation, FTS phrase, regex plan, and search path.",
            "Pass the same arguments as the planned `query_graph` call so the explanation matches the real execution plan.",
        ],
        "list_symbols" => vec![
            "Use for deterministic, unranked symbol enumeration with kind, language, subpath, and repo filters; ordering is stable file path, line, qualified name.",
            "`query_graph` ranks by relevance; `list_symbols` pages through every match. Use next_offset for large inventories.",
        ],
        "symbol_neighbors" | "traverse_graph" | "concept_clusters" => vec![
            "Use after resolving exact symbols or files to inspect direct relationships and bounded dependency paths.",
            "Keep depth and limits small first; escalate only when direct callers/callees are insufficient.",
        ],
        "trace_calls" => vec![
            "Use after resolving an exact symbol for directional caller/callee chains, or with `to` for the shortest call path to a target symbol.",
            "`symbol_neighbors` answers one hop; `trace_calls` answers multi-hop chains. Keep max_depth and max_nodes small first and widen only when direct hops are insufficient.",
        ],
        "cross_file_links" => vec![
            "Use after resolving an exact file to inspect cross-file coupling by shared symbol references.",
            "Weak same_package fallback edges are hidden by default; set include_weak=true only when validating heuristic resolution.",
            "Each linked file reports confidence_tier and reachability so weak evidence stays visible when requested.",
        ],
        "get_context" | "get_review_context" | "get_minimal_context" | "get_impact_radius" | "explain_change" | "detect_changes" => vec![
            "Use for review, impact, and bounded context assembly around changed files or concrete targets.",
            "Prefer minimal/change detection first; escalate to broader context only when risk or missing evidence requires it.",
        ],
        "build_graph" | "update_graph" | "postprocess_graph" => vec![
            "Use only after status or detect_changes shows a missing, stale, or changed graph.",
            "Prefer update_graph for incremental working-tree changes; reserve build_graph for the first build or a deliberate full rebuild.",
        ],
        "embed_chunks" => vec![
            "Use after build or update when hybrid retrieval is wanted and `[search.embedding]` url is configured in `.atlas/config.toml`.",
            "Start with dry_run=true to count pending chunks before writing vectors; Ollama-style backends auto-download the model, while OpenAI-compatible backends need it provisioned already.",
        ],
        "get_session_status" | "resume_session" | "wake_up" | "record_session_event"
        | "compact_session" => vec![
            "Use for session continuity: check status first, then wake_up or resume_session before asking the user to repeat context.",
            "record_session_event, wake_up, and compact_session write session state; call them intentionally, never speculatively.",
        ],
        "save_context_artifact" | "read_saved_context" | "search_saved_context"
        | "cross_session_search" | "purge_saved_context" | "search_decisions"
        | "get_context_stats" | "get_global_memory" => vec![
            "Use saved-context tools for large tool outputs and cross-session recall: search before read, and page with chunk_offset for large artifacts.",
            "Artifacts are session- and repo-scoped; purge_saved_context is destructive and requires an explicit user-approved action.",
        ],
        "memory_store" | "memory_recall" | "memory_link" | "memory_ops" | "feedback_record" => vec![
            "Use memory_store for durable repo facts, memory_recall for retrieval, memory_link for directed memory-map edges, and memory_ops to inspect typed-operator history.",
            "Memory links accept the memoir vocabulary plus supersedes, which marks the target row as replaced; duplicate links collapse and self-links or unknown ids are rejected.",
            "Recall is read-only; store, link, and feedback writes are state-changing. Respect project/session/frontend/global visibility scopes.",
        ],
        "memoir_create" | "memoir_add_concept" | "memoir_link" | "memoir_inspect"
        | "memoir_delete" => vec![
            "Use memoir tools for the separate concept layer, never as code-graph truth.",
            "Create the graph before concepts, inspect before linking, and remember missing concepts are never auto-created unless create_missing is explicit.",
            "memoir_delete removes the graph plus its concepts and relations; use dry_run=true to inspect counts first.",
        ],
        "experience_inspect" => vec![
            "Use to audit query-conditioned experience edges for one atlas:// URI before trusting a shortcut.",
            "Only fresh edges activate in context assembly; inspect is read-only and never mutates edges.",
        ],
        "overview_show" | "overview_search" | "overview_export" | "overview_refresh_plan"
        | "overview_ingest" => vec![
            "Use overview tools for L0/L1/L2 progressive disclosure: start at L1 and escalate to L2 only when detail is needed.",
            "overview_ingest and overview_refresh_plan with apply=true write overview state; show, search, and export are read-only.",
        ],
        name if name.starts_with("analyze_") || name.starts_with("assess_") || name.starts_with("find_") || matches!(name, "infer_modules" | "label_components") => vec![
            "Use for deterministic analysis after target files or symbols are known.",
            "Treat ranked findings as triage; validate risky refactors with tests or narrower graph context before editing.",
        ],
        name if name.starts_with("search_") || name.starts_with("read_") || matches!(name, "get_docs_section" | "search_templates" | "search_text_assets") => vec![
            "Use for non-code assets, docs, templates, SQL, prompts, or exact file excerpts.",
            "For symbol relationships, resolve graph symbols first; use content tools as companion evidence.",
        ],
        "tool_list" | "tool_search" | "tool_help" | "man" | "repo_registry" => vec![
            "Use for live MCP capability discovery and exact runtime docs before guessing tool arguments.",
            "Prefer tool_search when name is unclear, then tool_help or atlas://tool-docs/{name} for full usage.",
        ],
        _ => vec![
            "Use when request matches tool description and required input fields are known.",
            "Read structuredContent as source of truth in JSON mode; inspect retry_guidance on errors before retrying.",
        ],
    }
    .into_iter()
    .map(str::to_owned)
    .collect::<Vec<_>>();

    if is_state_changing_tool(tool_name) {
        usage.push("State-changing tool: call intentionally, avoid speculative execution, and preserve user work.".to_owned());
    }
    usage
}

pub(crate) fn few_shot_prompts(tool_name: &str, description: &str) -> Vec<String> {
    let default_prompt = format!(
        "User asks for: {description}. Use `{tool_name}` with documented arguments, then answer from structuredContent."
    );
    let prompts = match tool_name {
        "query_graph" => vec![
            "User asks: 'Where is compute defined?' Call `query_graph` with `{ \"text\": \"compute\" }`, then resolve ambiguity before reading files.".to_owned(),
            "User asks: 'Who calls compute?' Call `query_graph` with `{ \"text\": \"who calls compute\" }`, then follow with `symbol_neighbors` if usage edges are needed.".to_owned(),
        ],
        "get_context" => vec![
            "User asks: 'Give me bounded context for compute.' Call `get_context` with `{ \"target\": { \"kind\": \"query\", \"query\": \"compute\" } }`.".to_owned(),
            "User asks: 'Review context for src/lib.rs.' Call `get_context` with `{ \"target\": { \"kind\": \"file\", \"file\": \"src/lib.rs\" } }`.".to_owned(),
        ],
        "detect_changes" => vec![
            "User asks: 'What changed in my working tree?' Call `detect_changes` with `{ \"change_source\": { \"kind\": \"working_tree\" } }`.".to_owned(),
            "User asks: 'Compare against origin/main.' Call `detect_changes` with `{ \"change_source\": { \"kind\": \"base\", \"base\": \"origin/main\" } }`.".to_owned(),
        ],
        "build_graph" => vec![
            "User asks: 'Build graph for this repository.' Call `build_graph` with `{}`. Use this when graph has not been built or needs a full rebuild.".to_owned(),
            "After `status` reports missing graph DB, call `build_graph` with `{}` before graph-backed tools.".to_owned(),
        ],
        "update_graph" => vec![
            "User asks: 'Refresh graph from my edits.' Call `update_graph` with `{ \"change_source\": { \"kind\": \"working_tree\" } }`.".to_owned(),
            "User asks: 'Update graph for staged changes.' Call `update_graph` with `{ \"change_source\": { \"kind\": \"staged\" } }`.".to_owned(),
        ],
        "trace_calls" => vec![
            "User asks: 'Trace callers of compute upward.' Call `trace_calls` with `{ \"from\": \"compute\", \"direction\": \"up\" }`, then follow with `get_context` for bounded context.".to_owned(),
            "User asks: 'Shortest call path from build to persist?' Call `trace_calls` with `{ \"from\": \"build_graph\", \"to\": \"persist\" }` and read `path` from structuredContent.".to_owned(),
        ],
        "embed_chunks" => vec![
            "User asks: 'How many chunks still need embeddings?' Call `embed_chunks` with `{ \"dry_run\": true }` and read pending_chunks from structuredContent.".to_owned(),
            "After a fresh build with `[search.embedding]` configured, call `embed_chunks` with `{}` once, then use `query_graph` with `{ \"hybrid\": true }` for fused retrieval.".to_owned(),
        ],
        "explain_query" => vec![
            "User asks: 'Why did my symbol search return nothing?' Call `explain_query` with the same `{ \"text\": \"...\" }` the failed query used, then adjust per fts_plan and warnings.".to_owned(),
            "Before a regex sweep, call `explain_query` with `{ \"regex\": \"...\" }` to validate the pattern without executing the search.".to_owned(),
        ],
        "list_symbols" => vec![
            "User asks: 'Enumerate every struct under src/auth.' Call `list_symbols` with `{ \"subpath\": \"src/auth\", \"kind\": \"struct\" }` and page with next_offset.".to_owned(),
            "User asks for a complete, unranked function inventory. Call `list_symbols` with `{ \"kind\": \"function\" }` instead of `query_graph` relevance ranking.".to_owned(),
        ],
        "tool_search" => vec![
            "User asks: 'Which tool finds docs?' Call `tool_search` with `{ \"query\": \"docs\" }`, then call `tool_help` for selected exact tool.".to_owned(),
            "User mistypes tool name. Call `tool_search` with typo text and use ranked suggestions instead of guessing.".to_owned(),
        ],
        "tool_help" | "man" => vec![
            format!("User asks how to call `{tool_name}`. Request docs for exact exported MCP tool name, then follow examples shown in usage."),
            "User asks for resource docs. Read `atlas://tool-docs/{name}` when MCP resources are easier than a tool call.".to_owned(),
        ],
        _ => vec![default_prompt],
    };
    prompts
        .into_iter()
        .map(|prompt| truncate_text(&prompt, MAX_EXAMPLE_CHARS))
        .collect()
}

/// Concrete call examples for tools whose shapes deserve more than the
/// schema-derived placeholder. Returns `None` for tools without a special.
pub(crate) fn special_target_tool_examples(tool_name: &str) -> Option<Vec<String>> {
    let examples = match tool_name {
        "query_graph" => vec![
            json!({ "name": "query_graph", "arguments": { "text": "compute" } }),
            json!({ "name": "query_graph", "arguments": { "text": "src/service.rs::fn::compute" } }),
            json!({ "name": "query_graph", "arguments": { "text": "who calls compute" } }),
            json!({ "name": "query_graph", "arguments": { "text": "what breaks if I change compute" } }),
            json!({ "name": "query_graph", "arguments": { "text": "tests for compute" } }),
        ],
        "get_context" => vec![
            json!({ "name": "get_context", "arguments": { "target": { "kind": "query", "query": "compute" } } }),
            json!({ "name": "get_context", "arguments": { "target": { "kind": "query", "query": "src/service.rs::fn::compute" } } }),
            json!({ "name": "get_context", "arguments": { "target": { "kind": "query", "query": "who calls compute" } } }),
            json!({ "name": "get_context", "arguments": { "target": { "kind": "query", "query": "what breaks if I change compute" } } }),
            json!({ "name": "get_context", "arguments": { "target": { "kind": "query", "query": "tests for compute" } } }),
        ],
        "resolve_symbol" => vec![
            json!({ "name": "resolve_symbol", "arguments": { "name": "compute" } }),
            json!({ "name": "resolve_symbol", "arguments": { "name": "src/service.rs::fn::compute" } }),
        ],
        "build_graph" => vec![json!({ "name": "build_graph", "arguments": {} })],
        "update_graph" => vec![
            json!({ "name": "update_graph", "arguments": { "change_source": { "kind": "working_tree" } } }),
            json!({ "name": "update_graph", "arguments": { "change_source": { "kind": "staged" } } }),
            json!({ "name": "update_graph", "arguments": { "change_source": { "kind": "base", "base": "origin/main" } } }),
            json!({ "name": "update_graph", "arguments": { "change_source": { "kind": "files", "files": ["src/lib.rs"] } } }),
        ],
        "trace_calls" => vec![
            json!({ "name": "trace_calls", "arguments": { "from": "src/service.rs::fn::compute", "direction": "up", "max_depth": 2 } }),
            json!({ "name": "trace_calls", "arguments": { "from": "compute", "to": "reconcile_call_targets" } }),
        ],
        "embed_chunks" => vec![
            json!({ "name": "embed_chunks", "arguments": { "dry_run": true } }),
            json!({ "name": "embed_chunks", "arguments": { "limit": 500 } }),
        ],
        "explain_query" => vec![
            json!({ "name": "explain_query", "arguments": { "text": "compute" } }),
            json!({ "name": "explain_query", "arguments": { "regex": "handle|HANDLE|Handle_" } }),
        ],
        "list_symbols" => vec![
            json!({ "name": "list_symbols", "arguments": { "kind": "function", "limit": 50 } }),
            json!({ "name": "list_symbols", "arguments": { "subpath": "src/auth", "offset": 50, "limit": 50 } }),
        ],
        _ => return None,
    };
    Some(
        examples
            .into_iter()
            .map(|example: Value| truncate_text(&example.to_string(), MAX_EXAMPLE_CHARS))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_link_usage_guidance_documents_supersedes_and_validation() {
        let guidance = tool_usage_guidance("memory_link").join(" ");
        assert!(guidance.contains("supersedes"));
        assert!(guidance.contains("self-links"));
        assert!(guidance.contains("memory-map"));
    }

    #[test]
    fn graph_diagnostic_and_inventory_tools_have_tailored_guidance() {
        for name in [
            "trace_calls",
            "embed_chunks",
            "explain_query",
            "list_symbols",
        ] {
            let guidance = tool_usage_guidance(name);
            assert!(
                !guidance
                    .iter()
                    .any(|line| line.starts_with("Use when request matches tool description")),
                "{name} falls back to generic usage guidance"
            );
            assert!(
                !guidance
                    .iter()
                    .any(|line| line.starts_with("State-changing tool:") && name != "embed_chunks"),
                "{name} has an unexpected state-changing warning"
            );
        }
        // embed_chunks is state-changing and keeps its appended warning.
        assert!(
            tool_usage_guidance("embed_chunks")
                .iter()
                .any(|line| line.starts_with("State-changing tool:"))
        );
    }

    #[test]
    fn diagnostic_and_inventory_tools_have_concrete_examples() {
        let trace = special_target_tool_examples("trace_calls").expect("trace_calls examples");
        assert!(
            trace
                .iter()
                .any(|example| example.contains(r#""direction":"up""#))
        );
        assert!(
            trace
                .iter()
                .any(|example| example.contains(r#""to":"reconcile_call_targets""#))
        );

        let embed = special_target_tool_examples("embed_chunks").expect("embed_chunks examples");
        assert!(
            embed
                .iter()
                .any(|example| example.contains(r#""dry_run":true"#))
        );

        let explain =
            special_target_tool_examples("explain_query").expect("explain_query examples");
        assert!(
            explain
                .iter()
                .any(|example| example.contains(r#""regex":"handle|HANDLE|Handle_""#))
        );

        let list = special_target_tool_examples("list_symbols").expect("list_symbols examples");
        assert!(
            list.iter()
                .any(|example| example.contains(r#""subpath":"src/auth""#))
        );

        for name in [
            "trace_calls",
            "embed_chunks",
            "explain_query",
            "list_symbols",
        ] {
            let examples =
                special_target_tool_examples(name).unwrap_or_else(|| panic!("{name} examples"));
            assert!(
                !examples
                    .iter()
                    .any(|example| example.contains('<') || example.contains('>')),
                "{name} keeps placeholder examples"
            );
        }
    }
}
