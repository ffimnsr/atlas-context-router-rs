use super::*;

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
        repo_provenance: None,
    };
    store
        .replace_file_graph_for_repo(
            "repo_test",
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
        repo_provenance: None,
    };
    store
        .replace_file_graph_for_repo(
            "repo_test",
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
        repo_provenance: None,
    };
    store
        .replace_file_graph_for_repo(
            "repo_test",
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
fn query_graph_who_calls_executes_caller_edges() {
    let fixture = setup_mcp_fixture();
    let resp = call(
        "query_graph",
        Some(&json!({ "text": "who calls compute", "output_format": "json" })),
        "/ignored",
        &fixture.db_path,
    )
    .expect("who calls query_graph");
    let v = &resp["structuredContent"];
    assert_eq!(v["summary"]["usage_edges_included"], json!(true));
    let matches = v["matches"].as_array().expect("matches array");
    assert!(
        matches
            .iter()
            .any(|m| m["qn"] == json!("src/api.rs::fn::handle_request")),
        "caller handle_request must be present: {matches:?}"
    );
}

#[test]
fn query_graph_what_breaks_returns_impacted_callers() {
    let fixture = setup_mcp_fixture();
    let resp = call(
        "query_graph",
        Some(&json!({ "text": "what breaks if I change compute", "output_format": "json" })),
        "/ignored",
        &fixture.db_path,
    )
    .expect("what breaks query_graph");
    let v = &resp["structuredContent"];
    assert_eq!(v["summary"]["usage_edges_included"], json!(true));
    let matches = v["matches"].as_array().expect("matches array");
    assert!(
        matches
            .iter()
            .any(|m| m["qn"] == json!("src/api.rs::fn::handle_request")),
        "impacted caller handle_request must be present: {matches:?}"
    );
}

#[test]
fn query_graph_tests_for_returns_test_nodes() {
    let fixture = setup_mcp_fixture();
    let resp = call(
        "query_graph",
        Some(&json!({ "text": "tests for compute", "output_format": "json" })),
        "/ignored",
        &fixture.db_path,
    )
    .expect("tests for query_graph");
    let v = &resp["structuredContent"];
    assert_eq!(v["query"]["query_intent"]["kind"], json!("tests_for"));
    let matches = v["matches"].as_array().expect("matches array");
    assert!(
        matches
            .iter()
            .any(|m| m["qn"] == json!("tests/service_test.rs::fn::compute_test")),
        "test node compute_test must be present: {matches:?}"
    );
}

#[test]
fn query_graph_rejects_unrecognized_natural_language_intent_phrase() {
    let fixture = setup_mcp_fixture();
    let result = call(
        "query_graph",
        Some(&json!({ "text": "where is the place that handles error codes?", "output_format": "json" })),
        "/ignored",
        &fixture.db_path,
    )
    .expect("expected invalid_input tool result");
    assert_eq!(result["isError"], json!(true));
    assert_eq!(result["structuredContent"]["code"], json!("invalid_input"));
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
        .replace_file_graph_for_repo(
            "repo_test",
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
        .replace_file_graph_for_repo(
            "repo_test",
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
