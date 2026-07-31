use super::*;
use serde_json::json;

#[test]
fn query_graph_regex_param_filters_results() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "regex": "compute", "output_format": "json" });
    let response = call("query_graph", Some(&args), "/ignored", &fixture.db_path)
        .expect("query_graph regex call");
    let arr = response["structuredContent"]["matches"]
        .as_array()
        .expect("matches array");
    for item in arr {
        let qn = item["qn"].as_str().unwrap_or("");
        let name = item["name"].as_str().unwrap_or("");
        assert!(
            qn.contains("compute") || name.contains("compute"),
            "regex filter should only return matching symbols, got qn={qn} name={name}"
        );
    }
}

#[test]
fn query_graph_invalid_regex_returns_error() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "regex": "[invalid", "output_format": "json" });
    let result = call("query_graph", Some(&args), "/ignored", &fixture.db_path)
        .expect("invalid regex must return tool error result");
    assert_eq!(result["isError"], serde_json::json!(true));
    assert!(
        result["structuredContent"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("invalid regex")),
        "error message should mention invalid regex"
    );
}

#[test]
fn query_graph_empty_regex_is_treated_as_missing_when_text_present() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "text": "compute",
        "regex": "",
        "output_format": "json"
    });

    let response = call("query_graph", Some(&args), "/ignored", &fixture.db_path)
        .expect("empty regex with text should still search by text");
    let items = response["structuredContent"]["matches"]
        .as_array()
        .expect("matches array");
    assert!(!items.is_empty(), "expected text search results");
}

#[test]
fn query_graph_empty_text_and_regex_returns_actionable_error() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "text": "   ",
        "regex": "",
        "output_format": "json"
    });

    let result = call("query_graph", Some(&args), "/ignored", &fixture.db_path)
        .expect("empty text and regex must return tool error result");
    let message = result["structuredContent"]["message"]
        .as_str()
        .expect("message");
    let details = &result["structuredContent"]["details"];

    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["structuredContent"]["code"],
        serde_json::json!("invalid_input")
    );
    assert!(
        message.contains("query_graph needs non-empty 'text', non-empty 'regex', or both"),
        "expected actionable empty-input message, got: {message}"
    );
    assert_eq!(
        details["offending_fields"],
        serde_json::json!(["text", "regex"])
    );
    assert_eq!(
        details["accepted_argument_families"],
        serde_json::json!(["text", "regex", "text + regex"])
    );
    assert_eq!(
        details["retry_example"],
        serde_json::json!({"text": "compute"})
    );
    assert_eq!(
        details["alternate_retry_example"],
        serde_json::json!({"regex": "compute|handle_request"})
    );
    assert_eq!(
        details["normalization_performed"],
        serde_json::json!([
            "trimmed whitespace-only text to empty",
            "normalized empty regex to missing"
        ])
    );
    assert_eq!(
        details["fail_closed_reason"],
        serde_json::json!(
            "Atlas refused to guess because both searchable inputs were empty after normalization"
        )
    );
}

#[test]
fn query_graph_fuzzy_typo_prefers_symbol_over_markdown_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db").to_string_lossy().to_string();
    let mut store = Store::open(&db_path).expect("open store");

    let function = Node {
        id: NodeId::UNSET,
        kind: NodeKind::Function,
        name: "LoadIdentityMessages".to_owned(),
        qualified_name: "internal/requestctx/context.go::fn::LoadIdentityMessages".to_owned(),
        file_path: "internal/requestctx/context.go".to_owned(),
        line_start: 1,
        line_end: 20,
        language: "go".to_owned(),
        parent_name: None,
        params: Some("()".to_owned()),
        return_type: None,
        modifiers: Some("export".to_owned()),
        is_test: false,
        file_hash: "h1".to_owned(),
        extra_json: serde_json::json!({}),
    };
    store
        .replace_file_graph(
            "internal/requestctx/context.go",
            "h1",
            Some("go"),
            Some(20),
            &[function],
            &[],
        )
        .expect("replace function graph");

    let markdown = Node {
        id: NodeId::UNSET,
        kind: NodeKind::File,
        name: "Load Identity Messages".to_owned(),
        qualified_name: "docs/load_identity_messages.md".to_owned(),
        file_path: "docs/load_identity_messages.md".to_owned(),
        line_start: 1,
        line_end: 40,
        language: "markdown".to_owned(),
        parent_name: None,
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: "h2".to_owned(),
        extra_json: serde_json::json!({}),
    };
    store
        .replace_file_graph(
            "docs/load_identity_messages.md",
            "h2",
            Some("markdown"),
            Some(40),
            &[markdown],
            &[],
        )
        .expect("replace markdown graph");

    let args = serde_json::json!({
        "text": "LoadIdentityMesages",
        "fuzzy": true,
        "include_files": true,
        "output_format": "json"
    });
    let response =
        call("query_graph", Some(&args), "/ignored", &db_path).expect("query_graph call");
    let items = response["structuredContent"]["matches"]
        .as_array()
        .expect("matches array");

    assert!(!items.is_empty(), "expected fuzzy results");
    assert_eq!(items[0]["kind"].as_str(), Some("function"));
    assert_eq!(
        items[0]["qn"].as_str(),
        Some("internal/requestctx/context.go::fn::LoadIdentityMessages")
    );
    assert!(
        items
            .iter()
            .any(|item| item["kind"].as_str() == Some("file")),
        "include_files=true should keep file nodes visible"
    );
}

#[test]
fn query_graph_include_files_opt_in_controls_file_results() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db").to_string_lossy().to_string();
    let mut store = Store::open(&db_path).expect("open store");

    let file_node = Node {
        id: NodeId::UNSET,
        kind: NodeKind::File,
        name: "Architecture Notes".to_owned(),
        qualified_name: "docs/architecture.md".to_owned(),
        file_path: "docs/architecture.md".to_owned(),
        line_start: 1,
        line_end: 10,
        language: "markdown".to_owned(),
        parent_name: None,
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: "h".to_owned(),
        extra_json: serde_json::json!({}),
    };
    store
        .replace_file_graph(
            "docs/architecture.md",
            "h",
            Some("markdown"),
            Some(10),
            &[file_node],
            &[],
        )
        .expect("replace file graph");

    let no_files = serde_json::json!({ "text": "Architecture Notes", "output_format": "json" });
    let resp =
        call("query_graph", Some(&no_files), "/ignored", &db_path).expect("query_graph no files");
    assert!(
        resp["structuredContent"]["matches"]
            .as_array()
            .is_some_and(|items| items.is_empty())
    );

    let with_files = serde_json::json!({
        "text": "Architecture Notes",
        "include_files": true,
        "output_format": "json"
    });
    let resp = call("query_graph", Some(&with_files), "/ignored", &db_path)
        .expect("query_graph with files");
    let items = resp["structuredContent"]["matches"]
        .as_array()
        .expect("matches array");
    assert_eq!(items[0]["kind"].as_str(), Some("file"));
}

#[test]
fn query_graph_supported_grammar_normalizes_identifier_and_intent_phrases() {
    let fixture = setup_mcp_fixture();

    let plain = call(
        "query_graph",
        Some(&json!({ "text": "compute", "output_format": "json" })),
        "/ignored",
        &fixture.db_path,
    )
    .expect("plain identifier query_graph");
    assert_eq!(
        plain["structuredContent"]["query"]["query_intent"]["kind"],
        json!("plain_identifier")
    );
    assert_eq!(
        plain["structuredContent"]["query"]["query_intent"]["normalized_text"],
        json!("compute")
    );

    let qname = call(
        "query_graph",
        Some(&json!({ "text": "src/service.rs::fn::compute", "output_format": "json" })),
        "/ignored",
        &fixture.db_path,
    )
    .expect("qualified name query_graph");
    assert_eq!(
        qname["structuredContent"]["query"]["query_intent"]["kind"],
        json!("exact_qualified_name")
    );
    assert_eq!(
        qname["structuredContent"]["query"]["query_intent"]["normalized_text"],
        json!("src/service.rs::fn::compute")
    );

    let who_calls = call(
        "query_graph",
        Some(&json!({ "text": "who calls compute", "output_format": "json" })),
        "/ignored",
        &fixture.db_path,
    )
    .expect("who calls query_graph");
    assert_eq!(
        who_calls["structuredContent"]["query"]["query_intent"]["kind"],
        json!("who_calls")
    );
    assert_eq!(
        who_calls["structuredContent"]["query"]["query_intent"]["normalized_text"],
        json!("compute")
    );

    let what_breaks = call(
        "query_graph",
        Some(&json!({ "text": "what breaks if I change compute", "output_format": "json" })),
        "/ignored",
        &fixture.db_path,
    )
    .expect("what breaks query_graph");
    assert_eq!(
        what_breaks["structuredContent"]["query"]["query_intent"]["kind"],
        json!("what_breaks")
    );
    assert_eq!(
        what_breaks["structuredContent"]["query"]["query_intent"]["normalized_text"],
        json!("compute")
    );

    let tests_for = call(
        "query_graph",
        Some(&json!({ "text": "tests for compute", "output_format": "json" })),
        "/ignored",
        &fixture.db_path,
    )
    .expect("tests for query_graph");
    assert_eq!(
        tests_for["structuredContent"]["query"]["query_intent"]["kind"],
        json!("tests_for")
    );
    assert_eq!(
        tests_for["structuredContent"]["query"]["query_intent"]["normalized_text"],
        json!("compute")
    );
}

#[test]
fn query_graph_rejects_natural_language_only_description_with_retry_guidance() {
    let fixture = setup_mcp_fixture();
    let result = call(
        "query_graph",
        Some(&json!({
            "text": "please show me authentication flow",
            "output_format": "json"
        })),
        "/ignored",
        &fixture.db_path,
    )
    .expect("expected invalid_input tool result");

    assert_eq!(result["isError"], json!(true));
    assert_eq!(
        result["structuredContent"]["message"],
        json!(
            "query_graph text must be exact identifier, qualified name, or supported intent phrase"
        )
    );
    assert_eq!(
        result["structuredContent"]["details"]["supported_query_grammar"],
        json!([
            "compute",
            "src/service.rs::fn::compute",
            "who calls compute",
            "what breaks if I change compute",
            "tests for compute"
        ])
    );
}

#[test]
fn query_graph_all_repos_returns_repo_provenance_for_ambiguous_results() {
    use atlas_repo::{
        RepoRegistration, RepoRegistry, RepoRelationship, RepoRelationshipKind, TrustState,
        VcsMetadata, stable_repo_id,
    };
    use camino::{Utf8Path, Utf8PathBuf};

    let fixture = setup_git_mcp_fixture();
    let root = Utf8Path::new(&fixture.repo_root);
    let root_repo_id = stable_repo_id(root);
    let dep_repo_id = stable_repo_id(Utf8Path::new("/virtual/submodule"));
    let mut store = Store::open(&fixture.db_path).expect("open store");

    let root_compute = Node {
        extra_json: json!({"repo_id": root_repo_id.clone()}),
        ..make_node(
            NodeKind::Function,
            "compute",
            "src/service.rs::fn::compute",
            "src/service.rs",
        )
    };
    store
        .replace_file_graph(
            "src/service.rs",
            "hash:src/service.rs",
            Some("rust"),
            Some(5),
            std::slice::from_ref(&root_compute),
            &[],
        )
        .expect("replace root compute");

    let dep_compute = Node {
        extra_json: json!({"repo_id": dep_repo_id.clone()}),
        ..make_node(
            NodeKind::Function,
            "compute",
            "repo::repo_dep::vendor/dep/src/service.rs::fn::compute",
            "vendor/dep/src/service.rs",
        )
    };
    store
        .replace_file_graph(
            "vendor/dep/src/service.rs",
            "hash:vendor/dep/src/service.rs",
            Some("rust"),
            Some(5),
            std::slice::from_ref(&dep_compute),
            &[],
        )
        .expect("replace dep compute");

    let mut registry = RepoRegistry::new(root_repo_id.clone());
    registry.registrations = vec![
        RepoRegistration {
            repo_id: root_repo_id.clone(),
            root: root.to_path_buf(),
            display_alias: ".".to_owned(),
            vcs: VcsMetadata {
                head: None,
                default_branch: None,
                remote_url: None,
            },
            relationship: RepoRelationship {
                kind: RepoRelationshipKind::Root,
                parent_repo_id: None,
                parent_path: None,
            },
            trust_state: TrustState::Trusted,
            enabled: true,
            include_globs: None,
            exclude_globs: None,
            dependencies: Vec::new(),
        },
        RepoRegistration {
            repo_id: dep_repo_id.clone(),
            root: Utf8PathBuf::from("/virtual/submodule"),
            display_alias: "vendor/dep".to_owned(),
            vcs: VcsMetadata {
                head: None,
                default_branch: None,
                remote_url: None,
            },
            relationship: RepoRelationship {
                kind: RepoRelationshipKind::Submodule,
                parent_repo_id: Some(root_repo_id.clone()),
                parent_path: Some("vendor/dep".to_owned()),
            },
            trust_state: TrustState::Trusted,
            enabled: true,
            include_globs: None,
            exclude_globs: None,
            dependencies: Vec::new(),
        },
    ];
    registry.save(root).expect("save registry");

    let args = serde_json::json!({ "text": "compute", "repo_scope": { "kind": "all" }, "output_format": "json" });
    let first = call(
        "query_graph",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("query_graph call");
    let second = call(
        "query_graph",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("query_graph call repeat");
    assert_eq!(
        first["structuredContent"], second["structuredContent"],
        "cross-repo query output must be deterministic"
    );

    let results = first["structuredContent"]["matches"]
        .as_array()
        .expect("matches array");
    assert!(results.len() >= 2);
    assert!(
        results
            .iter()
            .any(|item| item["repo"]["display_alias"] == json!("."))
    );
    assert!(
        results
            .iter()
            .any(|item| item["repo"]["display_alias"] == json!("vendor/dep"))
    );
}

#[test]
fn query_graph_response_carries_relationship_guidance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "compute", "output_format": "json" });

    let response =
        call("query_graph", Some(&args), "/ignored", &fixture.db_path).expect("query_graph call");

    assert_eq!(
        response["structuredContent"]["summary"]["usage_edges_included"],
        false
    );
    assert_eq!(
        response["structuredContent"]["summary"]["relationship_tools"],
        serde_json::json!(["symbol_neighbors", "traverse_graph", "get_context"])
    );
    assert_eq!(response["content"].as_array().map(Vec::len), Some(1));
}

#[test]
fn semantic_empty_result_includes_hint() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "who calls missing_symbol", "semantic": true, "output_format": "json" });

    let response = call("query_graph", Some(&args), "/ignored", &fixture.db_path)
        .expect("query_graph semantic call");

    let matches = response["structuredContent"]["matches"]
        .as_array()
        .expect("matches array");
    assert!(matches.is_empty(), "expected empty results");
    let hint = response["structuredContent"]["warnings"][0]
        .as_str()
        .expect("warning hint");
    assert!(
        hint.contains("FTS found no symbol names"),
        "hint should explain FTS limitation: {hint}"
    );
}

#[test]
fn batch_query_graph_items_returns_per_query_results() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "items": [
            { "text": "compute", "output_format": "json" },
            { "text": "handle_request", "output_format": "json" }
        ],
        "output_format": "json"
    });

    let response = call(
        "batch_query_graph",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("batch_query_graph call");

    let body = &response["structuredContent"];
    assert_eq!(body["summary"]["query_count"], 2);

    let items = body["items"].as_array().expect("items array");
    let results = body["results"].as_array().expect("results array");
    assert_eq!(items.len(), 2);
    assert_eq!(results.len(), 2);
    assert_eq!(items[0]["query_index"], 0);
    assert_eq!(items[0]["normalized_text"], "compute");
    let first_items = results[0]["matches"].as_array().expect("matches array");
    assert!(!first_items.is_empty(), "expected results for 'compute'");
    assert!(
        first_items
            .iter()
            .any(|n| n["qualified_name"] == "src/service.rs::fn::compute")
    );
    assert_eq!(items[1]["query_index"], 1);
    assert_eq!(items[1]["normalized_text"], "handle_request");
    let second_items = results[1]["matches"].as_array().expect("matches array");
    assert!(
        !second_items.is_empty(),
        "expected results for 'handle_request'"
    );
}

#[test]
fn query_graph_limit_is_clamped_by_central_budget_policy() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "text": "compute",
        "limit": 9999,
        "output_format": "json"
    });

    let response =
        call("query_graph", Some(&args), "/ignored", &fixture.db_path).expect("query_graph call");

    assert_eq!(response["budget_status"], "override_clamped");
    assert_eq!(response["budget_hit"], true);
    assert_eq!(response["budget_limit"], 200);
    assert_eq!(response["budget_observed"], 9999);
}

#[test]
fn symbol_neighbors_limit_is_clamped_by_central_budget_policy() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "qname": "src/service.rs::fn::compute",
        "limit": 9999,
        "output_format": "json"
    });

    let response =
        call("symbol_neighbors", Some(&args), "/repo", &fixture.db_path).expect("neighbors");

    assert_eq!(response["budget_status"], "override_clamped");
    assert_eq!(response["budget_hit"], true);
    assert_eq!(
        response["budget_name"],
        "review_context_extraction.max_nodes"
    );
    assert_eq!(response["budget_limit"], 200);
    assert_eq!(response["budget_observed"], 9999);
}

#[test]
fn batch_query_graph_accepts_canonical_items_only() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "items": [
            { "text": "compute" },
            { "text": "handle_request" }
        ],
        "output_format": "json"
    });
    let response = call(
        "batch_query_graph",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("expected batch_query_graph success for canonical items");

    assert!(response["_meta"].get("deprecated_input_fields").is_none());
}

#[test]
fn batch_query_graph_empty_items_returns_error() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "items": [] });
    let result = call(
        "batch_query_graph",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("expected tool error for empty items array");
    assert_eq!(result["isError"], serde_json::json!(true));
    let msg = result["structuredContent"]["message"]
        .as_str()
        .unwrap_or("");
    assert!(msg.contains("non-empty"));
}

#[test]
fn batch_query_graph_rejects_legacy_queries_field() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "queries": [{ "text": "compute" }, { "text": "handle_request" }],
        "output_format": "json"
    });

    let response = call(
        "batch_query_graph",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("batch_query_graph legacy queries must return tool error result");

    assert_eq!(response["isError"], serde_json::json!(true));
    assert_eq!(
        response["structuredContent"]["message"],
        serde_json::json!("legacy batch_query_graph fields are no longer supported")
    );
}

#[test]
fn batch_query_graph_rejects_legacy_text_field() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "text": "compute,handle_request",
        "output_format": "json"
    });

    let response = call(
        "batch_query_graph",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("batch_query_graph legacy text must return tool error result");
    assert_eq!(response["isError"], serde_json::json!(true));
    assert_eq!(
        response["structuredContent"]["message"],
        serde_json::json!("legacy batch_query_graph fields are no longer supported")
    );
}

#[test]
fn batch_query_graph_rejects_legacy_text_plus_queries() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "text": "compute",
        "queries": [{ "text": "handle_request" }]
    });
    let result = call(
        "batch_query_graph",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("expected tool error for text+queries conflict");
    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["structuredContent"]["message"],
        serde_json::json!("legacy batch_query_graph fields are no longer supported")
    );
}

#[test]
fn batch_query_graph_rejects_text_plus_items() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "text": "compute",
        "items": [{ "text": "handle_request" }]
    });
    let result = call(
        "batch_query_graph",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("expected tool error for text+items conflict");
    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["structuredContent"]["message"],
        serde_json::json!("legacy batch_query_graph fields are no longer supported")
    );
}

#[test]
fn batch_query_graph_over_limit_returns_error() {
    let fixture = setup_mcp_fixture();
    let items: Vec<serde_json::Value> = (0..21)
        .map(|i| serde_json::json!({ "text": format!("sym{i}") }))
        .collect();
    let args = serde_json::json!({ "items": items });
    let result = call(
        "batch_query_graph",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("expected tool error for >20 queries");
    assert_eq!(result["isError"], serde_json::json!(true));
    let msg = result["structuredContent"]["message"]
        .as_str()
        .unwrap_or("");
    assert!(
        msg.contains("maximum"),
        "error should mention maximum: {msg}"
    );
}

#[test]
fn batch_query_graph_partial_empty_result_carries_hint() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "items": [
            { "text": "compute" },
            { "text": "who calls missing_symbol", "semantic": true }
        ],
        "output_format": "json"
    });

    let response = call(
        "batch_query_graph",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("batch_query_graph call");

    let results = response["structuredContent"]["results"]
        .as_array()
        .expect("results array");
    assert_eq!(results.len(), 2);

    let first_items = results[0]["matches"].as_array().expect("matches");
    assert!(!first_items.is_empty());
    assert_eq!(
        results[0]["warnings"].as_array().map(Vec::len),
        Some(0),
        "no warning for successful query"
    );

    let second_items = results[1]["matches"].as_array().expect("matches");
    assert!(
        second_items.is_empty(),
        "expected empty results for NL phrase"
    );
    let hint = results[1]["warnings"][0].as_str().expect("warning present");
    assert!(
        hint.contains("FTS found no symbol names"),
        "hint should explain FTS limit: {hint}"
    );
}

#[test]
fn symbol_neighbors_includes_call_edge_sites() {
    let fixture = setup_mcp_fixture();
    let mut store = Store::open(&fixture.db_path).expect("open store");
    let handle = make_node(
        NodeKind::Function,
        "handle_request",
        "src/api.rs::fn::handle_request",
        "src/api.rs",
    );
    let first_call = make_edge(
        EdgeKind::Calls,
        "src/api.rs::fn::handle_request",
        "src/service.rs::fn::compute",
        "src/api.rs",
    );
    let mut second_call = make_edge(
        EdgeKind::Calls,
        "src/api.rs::fn::handle_request",
        "src/service.rs::fn::compute",
        "src/api.rs",
    );
    second_call.line = Some(2);
    store
        .replace_file_graph(
            "src/api.rs",
            "hash:src/api.rs",
            Some("rust"),
            Some(5),
            &[handle],
            &[first_call, second_call],
        )
        .expect("replace api graph");

    let args =
        serde_json::json!({ "qname": "src/service.rs::fn::compute", "output_format": "json" });
    let response = call(
        "symbol_neighbors",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("symbol_neighbors call");
    let text = unwrap_tool_text(response);
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(
        value.pointer("/symbol/qname").and_then(|v| v.as_str()),
        Some("src/service.rs::fn::compute")
    );
    assert_eq!(
        value.pointer("/callers/0/qn").and_then(|v| v.as_str()),
        Some("src/api.rs::fn::handle_request")
    );
    assert_eq!(value["call_sites"].as_array().map(|v| v.len()), Some(2));
    assert_eq!(
        value.pointer("/call_sites/0/from").and_then(|v| v.as_str()),
        Some("src/api.rs::fn::handle_request")
    );
    assert_eq!(
        value.pointer("/call_sites/0/to").and_then(|v| v.as_str()),
        Some("src/service.rs::fn::compute")
    );
    assert_eq!(
        value.pointer("/call_sites/0/file").and_then(|v| v.as_str()),
        Some("src/api.rs")
    );
    assert_eq!(
        value.pointer("/call_sites/0/line").and_then(|v| v.as_u64()),
        Some(1)
    );
    assert_eq!(
        value.pointer("/call_sites/1/line").and_then(|v| v.as_u64()),
        Some(2)
    );
}

#[test]
fn symbol_neighbors_normalizes_alias_qname() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "qname": "src/service.rs::function::compute",
        "output_format": "json"
    });

    let response = call(
        "symbol_neighbors",
        Some(&args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("symbol_neighbors call");
    let text = unwrap_tool_text(response);
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(value["symbol"]["qname"], "src/service.rs::fn::compute");
    assert_eq!(
        value.pointer("/callers/0/qn").and_then(|v| v.as_str()),
        Some("src/api.rs::fn::handle_request")
    );
}

#[test]
fn traverse_graph_normalizes_alias_qname() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "from_qn": "src/service.rs::function::compute",
        "output_format": "json"
    });

    let response = call("traverse_graph", Some(&args), "/ignored", &fixture.db_path)
        .expect("traverse_graph call");
    let text = unwrap_tool_text(response);
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(value["nodes"][0]["qn"], "src/service.rs::fn::compute");
}

#[test]
fn list_graph_stats_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let resp = call("list_graph_stats", None, "/repo", &fixture.db_path).expect("list_graph_stats");
    assert_provenance(&resp, "/repo", &fixture.db_path);
    let prov = &resp["atlas_provenance"];
    assert_eq!(prov["indexed_file_count"].as_i64(), Some(3));
    assert!(prov["last_indexed_at"].as_str().is_some());
}

#[test]
fn query_graph_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "compute" });
    let resp = call("query_graph", Some(&args), "/repo", &fixture.db_path).expect("query_graph");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn query_graph_clean_repo_has_no_freshness_warning() {
    let fixture = setup_git_mcp_fixture();
    let args = serde_json::json!({ "text": "compute", "output_format": "json" });

    let resp = call(
        "query_graph",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("query_graph");

    assert!(
        resp.get("atlas_freshness").is_none(),
        "clean repo should not emit freshness warning"
    );
}

#[test]
fn query_graph_changed_non_code_zero_node_file_has_no_freshness_warning() {
    let fixture = setup_git_mcp_fixture();
    write_repo_file(
        std::path::Path::new(&fixture.repo_root),
        ".gitignore",
        "target/\n",
    );
    let args = serde_json::json!({ "text": "compute", "output_format": "json" });

    let resp = call(
        "query_graph",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("query_graph");

    assert!(
        resp.get("atlas_freshness").is_none(),
        "non-code zero-node file changes should not emit freshness warning"
    );
}

#[test]
fn query_graph_stale_changed_symbol_emits_freshness_warning() {
    let fixture = setup_git_mcp_fixture();
    write_repo_file(
        std::path::Path::new(&fixture.repo_root),
        "src/service.rs",
        "pub fn compute() -> i32 { 99 }\n",
    );
    let args = serde_json::json!({ "text": "compute", "output_format": "json" });

    let resp = call(
        "query_graph",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("query_graph");

    assert_eq!(
        resp.pointer("/atlas_freshness/stale")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        resp.pointer("/atlas_freshness/stale_result_files/0")
            .and_then(|value| value.as_str()),
        Some("src/service.rs")
    );
    assert!(
        resp.pointer("/atlas_freshness/warning")
            .and_then(|value| value.as_str())
            .is_some_and(|warning| warning.contains("stale"))
    );
}

#[test]
fn traverse_graph_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args =
        serde_json::json!({ "from_qn": "src/service.rs::fn::compute", "output_format": "json" });
    let resp =
        call("traverse_graph", Some(&args), "/repo", &fixture.db_path).expect("traverse_graph");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn symbol_neighbors_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args =
        serde_json::json!({ "qname": "src/service.rs::fn::compute", "output_format": "json" });
    let resp =
        call("symbol_neighbors", Some(&args), "/repo", &fixture.db_path).expect("symbol_neighbors");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn symbol_neighbors_missing_qname_sets_error_code() {
    let fixture = setup_mcp_fixture();
    let args =
        serde_json::json!({ "qname": "src/nonexistent.rs::fn::ghost", "output_format": "json" });
    let resp = call("symbol_neighbors", Some(&args), "/repo", &fixture.db_path)
        .expect("symbol_neighbors should not error for missing symbol");
    assert_eq!(
        resp["structuredContent"]["lookup"]["error_code"].as_str(),
        Some("node_not_found")
    );
    assert_eq!(
        resp["structuredContent"]["summary"]["status"],
        json!("node_not_found")
    );
    assert_eq!(resp["structuredContent"]["callers"], json!([]));
    assert_error_code_doc_link(
        resp["structuredContent"]["lookup"]["error_code_docs"]
            .as_str()
            .expect("lookup.error_code_docs"),
        "node_not_found",
    );
    assert!(
        resp["structuredContent"]["lookup"]["message"]
            .as_str()
            .is_some()
    );
    let suggestions = resp["structuredContent"]["lookup"]["suggestions"]
        .as_array()
        .expect("lookup.suggestions");
    assert!(!suggestions.is_empty());
}

#[test]
fn cross_file_links_returns_stable_shape_for_linked_and_isolated_files() {
    let fixture = setup_mcp_fixture();
    let linked_args = serde_json::json!({ "file": "src/service.rs", "output_format": "json" });
    let linked_resp = call(
        "cross_file_links",
        Some(&linked_args),
        "/repo",
        &fixture.db_path,
    )
    .expect("cross_file_links linked");
    let linked: serde_json::Value =
        serde_json::from_str(&unwrap_tool_text(linked_resp)).expect("parse json");
    assert_eq!(linked["source_file"], json!("src/service.rs"));
    assert!(linked["linked_files"].as_array().is_some());
    assert!(linked["coupling_metric"].as_object().is_some());
    assert!(linked["summary"].as_object().is_some());

    let isolated_args =
        serde_json::json!({ "file": "tests/service_test.rs", "output_format": "json" });
    let isolated_resp = call(
        "cross_file_links",
        Some(&isolated_args),
        "/repo",
        &fixture.db_path,
    )
    .expect("cross_file_links isolated");
    let isolated: serde_json::Value =
        serde_json::from_str(&unwrap_tool_text(isolated_resp)).expect("parse json");
    assert_eq!(isolated["source_file"], json!("tests/service_test.rs"));
    assert!(isolated["linked_files"].as_array().is_some());
}

#[test]
fn cross_file_links_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "file": "src/service.rs", "output_format": "json" });
    let resp =
        call("cross_file_links", Some(&args), "/repo", &fixture.db_path).expect("cross_file_links");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn concept_clusters_returns_stable_shape_for_present_and_empty_clusters() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "files": ["src/service.rs"], "output_format": "json" });
    let resp =
        call("concept_clusters", Some(&args), "/repo", &fixture.db_path).expect("concept_clusters");
    let value: serde_json::Value =
        serde_json::from_str(&unwrap_tool_text(resp)).expect("parse json");
    assert_eq!(value["seed_files"], json!(["src/service.rs"]));
    assert!(value["clusters"].as_array().is_some());
    assert!(value["summary"].as_object().is_some());
    assert!(value["truncated"].as_bool().is_some());

    let empty_args =
        serde_json::json!({ "files": ["tests/service_test.rs"], "output_format": "json" });
    let empty_resp = call(
        "concept_clusters",
        Some(&empty_args),
        "/repo",
        &fixture.db_path,
    )
    .expect("concept_clusters empty");
    let empty_value: serde_json::Value =
        serde_json::from_str(&unwrap_tool_text(empty_resp)).expect("parse json");
    assert_eq!(empty_value["seed_files"], json!(["tests/service_test.rs"]));
    assert!(empty_value["clusters"].as_array().is_some());
}

#[test]
fn concept_clusters_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "files": ["src/service.rs"], "output_format": "json" });
    let resp =
        call("concept_clusters", Some(&args), "/repo", &fixture.db_path).expect("concept_clusters");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn batch_query_graph_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "compute" });
    let resp = call("batch_query_graph", Some(&args), "/repo", &fixture.db_path)
        .expect("batch_query_graph");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn provenance_indexed_file_count_is_zero_for_empty_db() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("empty.db").to_string_lossy().to_string();
    let _ = Store::open(&db_path).expect("open store");

    let resp =
        call("list_graph_stats", None, "/repo", &db_path).expect("list_graph_stats on empty db");
    let prov = &resp["atlas_provenance"];
    assert_eq!(prov["indexed_file_count"].as_i64(), Some(0));
    assert!(prov["last_indexed_at"].is_null());
}

#[test]
fn explain_query_describes_fts_path() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "compute", "output_format": "json" });
    let resp =
        call("explain_query", Some(&args), "/repo", &fixture.db_path).expect("explain_query call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(v["normalized_query"]["search_path"].as_str(), Some("fts5"));
    let tokens = v["tokenization"]["fts_tokens"]
        .as_array()
        .expect("fts_tokens array");
    assert!(tokens.iter().any(|t| t.as_str() == Some("compute")));
    assert_eq!(
        v["tokenization"]["fts_phrase"].as_str(),
        Some("\"compute\"")
    );
    assert_eq!(v["regex_plan"]["valid"].as_bool(), Some(true));
}

#[test]
fn explain_query_missing_input_returns_error() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "output_format": "json" });
    let result = call("explain_query", Some(&args), "/repo", &fixture.db_path)
        .expect("missing input must return tool error result");
    assert_eq!(result["isError"], serde_json::json!(true));
}

#[test]
fn explain_query_validates_invalid_regex() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "regex": "[invalid", "output_format": "json" });
    let result = call("explain_query", Some(&args), "/repo", &fixture.db_path)
        .expect("invalid regex must return tool error result");

    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["structuredContent"]["code"],
        serde_json::json!("invalid_input")
    );
    assert!(
        result["structuredContent"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("invalid regex"))
    );
}

#[test]
fn explain_query_with_regex_only_uses_structural_scan_path() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "regex": "compute.*", "output_format": "json" });
    let resp = call("explain_query", Some(&args), "/repo", &fixture.db_path)
        .expect("explain_query regex-only call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(
        v["normalized_query"]["search_path"].as_str(),
        Some("regex_structural_scan")
    );
    assert_eq!(v["regex_plan"]["valid"].as_bool(), Some(true));
}

#[test]
fn explain_query_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "compute", "output_format": "json" });
    let resp =
        call("explain_query", Some(&args), "/repo", &fixture.db_path).expect("explain_query");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn resolve_symbol_finds_exact_match() {
    let fixture = setup_mcp_fixture();
    let args =
        serde_json::json!({ "name": "compute", "file": "src/service.rs", "output_format": "json" });
    let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("resolve_symbol call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(v["summary"]["status"], json!("resolved"));
    assert_eq!(
        v["best_match"]["qualified_name"].as_str(),
        Some("src/service.rs::fn::compute")
    );
    assert!(v["summary"]["match_count"].as_i64().unwrap_or(0) >= 1);
    let matches = v["ambiguity"]["matches"].as_array().expect("matches array");
    assert!(!matches.is_empty());
    assert_eq!(matches[0]["kind"].as_str(), Some("function"));
    assert_eq!(matches[0]["file_path"].as_str(), Some("src/service.rs"));
}

#[test]
fn resolve_symbol_missing_name_returns_error() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "output_format": "json" });
    let result = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("missing name must return tool error result");
    assert_eq!(result["isError"], serde_json::json!(true));
}

#[test]
fn resolve_symbol_empty_name_returns_error() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "name": "", "output_format": "json" });
    let result = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("empty name must return tool error result");
    assert_eq!(result["isError"], serde_json::json!(true));
}

#[test]
fn resolve_symbol_no_match_returns_tool_error() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "name": "nonexistent_symbol_xyz", "output_format": "json" });
    let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("resolve_symbol call");

    assert_eq!(resp["isError"], json!(true));
    assert_eq!(resp["structuredContent"]["code"], json!("invalid_input"));
}

#[test]
fn resolve_symbol_kind_alias_fn_resolves_to_function() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "name": "compute", "kind": "fn", "output_format": "json" });
    let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("resolve_symbol call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(v["summary"]["status"], json!("resolved"));
    assert_eq!(
        v["best_match"]["qualified_name"].as_str(),
        Some("src/service.rs::fn::compute")
    );
}

#[test]
fn resolve_symbol_file_filter_narrows_results() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "name": "handle_request", "file": "src/api.rs", "output_format": "json" });
    let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("resolve_symbol call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(v["summary"]["status"], json!("resolved"));
    assert_eq!(
        v["best_match"]["qualified_name"].as_str(),
        Some("src/api.rs::fn::handle_request")
    );
    let matches = v["ambiguity"]["matches"].as_array().expect("matches array");
    for m in matches {
        assert!(m["file_path"].as_str().unwrap_or("").contains("src/api.rs"));
    }
}

#[test]
fn resolve_symbol_returns_ambiguous_success_shape() {
    let fixture = setup_mcp_fixture();
    let mut store = Store::open(&fixture.db_path).expect("open store");
    let dupe = make_node(
        NodeKind::Function,
        "compute",
        "src/extra.rs::fn::compute",
        "src/extra.rs",
    );
    store
        .replace_file_graph(
            "src/extra.rs",
            "hash:src/extra.rs",
            Some("rust"),
            Some(1),
            &[dupe],
            &[],
        )
        .expect("replace extra graph");

    let args = serde_json::json!({ "name": "compute", "output_format": "json" });
    let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("resolve_symbol call");
    let v: serde_json::Value = serde_json::from_str(&unwrap_tool_text(resp)).expect("parse json");
    assert_eq!(v["summary"]["status"], json!("ambiguous"));
    assert!(v["best_match"].is_object() || v["best_match"].is_null());
    assert!(v["ambiguity"]["ambiguous"].as_bool().unwrap_or(false));
    assert!(v["ambiguity"]["matches"].as_array().unwrap().len() >= 2);
}

#[test]
fn resolve_symbol_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "name": "compute" });
    let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("resolve_symbol call");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn resolve_symbol_includes_suggestions() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "name": "compute", "output_format": "json" });
    let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("resolve_symbol call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    let suggestions = v["suggestions"].as_array().expect("suggestions array");
    assert!(!suggestions.is_empty());
    let next_tools = suggestions[0]["next_tools"].as_array().expect("next_tools");
    assert!(
        next_tools
            .iter()
            .any(|t| t.as_str() == Some("symbol_neighbors"))
    );
}

#[test]
fn resolve_symbol_truncation_metadata_present() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "name": "compute", "output_format": "json" });
    let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
        .expect("resolve_symbol call");
    let text = unwrap_tool_text(resp.clone());
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    assert!(v["summary"]["truncated"].as_bool().is_some());
}

#[test]
fn query_graph_truncation_metadata_present() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "compute" });
    let resp =
        call("query_graph", Some(&args), "/repo", &fixture.db_path).expect("query_graph call");
    let result_count = resp["structuredContent"]["matches"]
        .as_array()
        .map(Vec::len)
        .expect("matches array");
    assert!(resp.get("atlas_truncated").is_some());
    assert!(
        result_count > 0,
        "query_graph should return at least one result"
    );
}

#[test]
fn query_graph_subpath_filters_results() {
    let fixture = setup_mcp_fixture();
    let args =
        serde_json::json!({ "text": "compute", "subpath": "tests", "output_format": "json" });
    let resp = call("query_graph", Some(&args), "/repo", &fixture.db_path)
        .expect("query_graph subpath call");
    if let Some(arr) = resp["structuredContent"]["matches"].as_array() {
        for item in arr {
            let fp = item["file"].as_str().unwrap_or("");
            assert!(
                fp.starts_with("tests"),
                "subpath='tests' must restrict results to tests/, got file={fp}"
            );
        }
    }
}

#[test]
fn query_graph_fuzzy_returns_near_miss() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "comput", "fuzzy": true, "output_format": "json" });
    let resp = call("query_graph", Some(&args), "/repo", &fixture.db_path)
        .expect("query_graph fuzzy call");
    let arr = resp["structuredContent"]["matches"]
        .as_array()
        .expect("expected matches array");
    assert!(
        arr.iter()
            .any(|item| item["qn"].as_str().unwrap_or("").contains("compute"))
    );
}

#[test]
fn query_graph_hybrid_falls_back_to_fts() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "compute", "hybrid": true, "output_format": "json" });
    let resp = call("query_graph", Some(&args), "/repo", &fixture.db_path)
        .expect("query_graph hybrid call");
    let arr = resp["structuredContent"]["matches"]
        .as_array()
        .expect("expected matches array");
    assert!(
        arr.iter()
            .any(|item| item["qn"].as_str().unwrap_or("").contains("compute"))
    );
    assert_eq!(resp["atlas_query_mode"].as_str(), Some("fts5"));
}

#[test]
fn query_graph_response_includes_query_mode() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "compute", "output_format": "json" });
    let resp =
        call("query_graph", Some(&args), "/repo", &fixture.db_path).expect("query_graph call");
    assert!(resp.get("atlas_query_mode").is_some());
    assert_eq!(resp["atlas_query_mode"].as_str(), Some("fts5"));
}

#[test]
fn query_graph_json_includes_ranking_evidence_and_legend() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "compute", "output_format": "json" });
    let resp =
        call("query_graph", Some(&args), "/repo", &fixture.db_path).expect("query_graph call");
    let first = resp["structuredContent"]["matches"]
        .as_array()
        .and_then(|items| items.first())
        .expect("first result");
    assert!(first.get("ranking_evidence").is_some());
    assert_eq!(
        first["ranking_evidence"]["base_mode"].as_str(),
        Some("fts5")
    );
    assert!(resp.get("atlas_ranking_evidence_legend").is_some());
    assert!(resp["atlas_ranking_evidence_legend"]["exact_name_match"].is_string());
}

#[test]
fn batch_query_graph_json_includes_ranking_evidence() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "items": [{ "text": "compute" }],
        "output_format": "json"
    });
    let resp = call("batch_query_graph", Some(&args), "/repo", &fixture.db_path)
        .expect("batch_query_graph call");
    let first = &resp["structuredContent"]["results"]
        .as_array()
        .expect("batch results")[0]["matches"][0];
    assert!(first.get("ranking_evidence").is_some());
    assert!(resp.get("atlas_ranking_evidence_legend").is_some());
}

#[test]
fn explain_query_reports_active_query_mode_and_ranking_factors() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "compute", "fuzzy": true, "output_format": "json" });
    let resp =
        call("explain_query", Some(&args), "/repo", &fixture.db_path).expect("explain_query call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    assert_eq!(
        v["normalized_query"]["active_query_mode"].as_str(),
        Some("fts5")
    );
    let factors = v["normalized_query"]["ranking_factors"]
        .as_array()
        .expect("ranking_factors array");
    assert!(
        factors
            .iter()
            .any(|f| f.as_str() == Some("fuzzy_edit_distance_boost"))
    );
}

#[test]
fn explain_query_matches_include_ranking_evidence() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "text": "compute", "output_format": "json" });
    let resp =
        call("explain_query", Some(&args), "/repo", &fixture.db_path).expect("explain_query call");
    let text = unwrap_tool_text(resp.clone());
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    let first = &v["matches"].as_array().expect("matches")[0];
    assert!(first.get("ranking_evidence").is_some());
    assert!(resp.get("atlas_ranking_evidence_legend").is_some());
}

#[test]
fn explain_query_reports_subpath_filter() {
    let fixture = setup_mcp_fixture();
    let args =
        serde_json::json!({ "text": "compute", "subpath": "src/auth", "output_format": "json" });
    let resp =
        call("explain_query", Some(&args), "/repo", &fixture.db_path).expect("explain_query call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    assert_eq!(
        v["normalized_query"]["filters_applied"]["subpath"].as_bool(),
        Some(true)
    );
    assert_eq!(v["input"]["subpath"].as_str(), Some("src/auth"));
}
