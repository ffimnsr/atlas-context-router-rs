use super::super::*;
use crate::output::OutputFormat;
use std::fs;
use std::path::Path;

fn make_repo(files: &[(&str, &str)]) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_str().unwrap().to_owned();
    fs::create_dir_all(format!("{root}/.git")).unwrap();
    for (rel, content) in files {
        let full = format!("{root}/{rel}");
        if let Some(parent) = Path::new(&full).parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&full, content).unwrap();
    }
    (dir, root)
}

fn parse_tool_json(resp: serde_json::Value) -> serde_json::Value {
    serde_json::from_str(resp["content"][0]["text"].as_str().expect("tool text"))
        .expect("parse json tool text")
}

// -----------------------------------------------------------------------
// search_content
// -----------------------------------------------------------------------

#[test]
fn search_content_literal_match() {
    let (_dir, root) = make_repo(&[
        (
            "src/auth.rs",
            "fn verify_token(tok: &str) -> bool {\n    true\n}\n",
        ),
        ("src/other.rs", "fn unrelated() {}\n"),
    ]);
    let args = serde_json::json!({ "query": "verify_token", "exclude_generated": false });
    let resp = tool_search_content(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    let ms = v["matches"].as_array().unwrap();
    assert!(!ms.is_empty(), "expected at least one match");
    assert!(
        ms.iter()
            .any(|m| m["file"].as_str().unwrap().ends_with("auth.rs")),
        "{ms:?}"
    );
    assert_eq!(v["atlas_result_kind"], "content_matches");
}

#[test]
fn search_content_regex_match() {
    let (_dir, root) = make_repo(&[("src/lib.rs", "pub fn foo() {}\npub fn bar() {}\n")]);
    let args = serde_json::json!({
        "query": r"pub fn \w+",
        "is_regex": true,
        "exclude_generated": false
    });
    let resp = tool_search_content(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(
        v["result_count"].as_u64().unwrap() >= 2,
        "expected ≥2 matches: {v}"
    );
}

#[test]
fn search_content_invalid_regex_returns_guidance() {
    let (_dir, root) = make_repo(&[(
        "src/lib.rs",
        "pub enum Command {\n    Context { value: String },\n}\n",
    )]);
    let args = serde_json::json!({
        "query": "Command::Context|Context {",
        "is_regex": true,
        "exclude_generated": false
    });

    let result = tool_search_content(Some(&args), &root, OutputFormat::Json)
        .expect("invalid regex must return tool error result");
    let message = result["structuredContent"]["message"]
        .as_str()
        .expect("message");
    let detail = result["structuredContent"]["details"]["detail"]
        .as_str()
        .expect("detail");

    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["structuredContent"]["code"],
        serde_json::json!("invalid_input")
    );
    assert!(
        message.contains("invalid regex pattern for search_content"),
        "expected invalid regex guidance, got: {message}"
    );
    assert!(
        detail.contains("Set is_regex=false for literal text search"),
        "expected literal-search guidance, got detail: {detail}"
    );
    assert!(
        detail.contains(r"Command::Context|Context \{"),
        "expected escaped regex guidance, got detail: {detail}"
    );
}

#[test]
fn search_content_exclude_generated_node_modules() {
    let (_dir, root) = make_repo(&[
        ("node_modules/vendor.js", "function secret() {}"),
        ("src/main.js", "function app() {}"),
    ]);
    let args = serde_json::json!({ "query": "function", "exclude_generated": true });
    let resp = tool_search_content(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    let ms = v["matches"].as_array().unwrap();
    assert!(
        !ms.iter()
            .any(|m| m["file"].as_str().unwrap().contains("node_modules")),
        "node_modules leaked: {ms:?}"
    );
}

#[test]
fn search_content_min_js_suppressed_by_default() {
    let (_dir, root) = make_repo(&[
        ("dist/app.min.js", "var x=1;function a(){return x}"),
        ("src/main.js", "function main() {}"),
    ]);
    let args = serde_json::json!({ "query": "function" });
    let resp = tool_search_content(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    let ms = v["matches"].as_array().unwrap();
    assert!(
        !ms.iter()
            .any(|m| m["file"].as_str().unwrap().ends_with(".min.js")),
        "min.js leaked: {ms:?}"
    );
}

#[test]
fn search_content_max_results_truncates() {
    let files: Vec<(String, String)> = (0..10)
        .map(|i| (format!("src/f{i}.rs"), format!("fn target_{i}() {{}}")))
        .collect();
    let file_refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let (_dir, root) = make_repo(&file_refs);
    let args = serde_json::json!({
        "query": "target",
        "max_results": 3,
        "exclude_generated": false
    });
    let resp = tool_search_content(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(
        v["result_count"].as_u64().unwrap() <= 3,
        "result_count exceeded max: {v}"
    );
    assert!(v["truncated"].as_bool().unwrap(), "expected truncated=true");
}

#[test]
fn search_content_max_results_is_clamped_by_central_budget_policy() {
    let files: Vec<(String, String)> = (0..210)
        .map(|i| (format!("src/f{i}.rs"), format!("fn target_{i}() {{}}")))
        .collect();
    let file_refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    let (_dir, root) = make_repo(&file_refs);
    let args = serde_json::json!({
        "query": "target",
        "max_results": 9999,
        "exclude_generated": false
    });

    let resp = tool_search_content(Some(&args), &root, OutputFormat::Json).unwrap();
    let body: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();

    assert_eq!(resp["budget_status"], "partial_result");
    assert_eq!(resp["budget_hit"], true);
    assert_eq!(resp["budget_name"], "review_context_extraction.max_nodes");
    assert_eq!(resp["budget_limit"], 200);
    assert_eq!(resp["budget_observed"], 201);
    assert_eq!(body["result_count"], 200);
    assert_eq!(body["truncated"], true);
}

#[test]
fn search_content_symbol_hint_present() {
    let (_dir, root) = make_repo(&[("src/lib.rs", "fn my_func() {}")]);
    let args = serde_json::json!({ "query": "my_func", "exclude_generated": false });
    let resp = tool_search_content(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(v["atlas_hint"].is_string(), "expected symbol hint: {v}");
    assert!(
        v["atlas_hint"].as_str().unwrap().contains("query_graph"),
        "hint should mention query_graph: {v}"
    );
}

#[test]
fn search_content_rich_snippets_are_opt_in() {
    let (_dir, root) = make_repo(&[(
        "src/lib.rs",
        "fn before() {}\nfn target() {}\nfn after() {}\n",
    )]);
    let args = serde_json::json!({
        "query": "target",
        "exclude_generated": false,
        "rich_snippets": true,
        "snippet_context_lines": 1
    });
    let resp = tool_search_content(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    let snippets = v["rich_snippets"].as_array().expect("rich snippets array");
    assert_eq!(snippets.len(), 1, "expected one grouped snippet: {v}");
    assert_eq!(snippets[0]["match_line"], 2);
    assert!(
        snippets[0]["snippet"]
            .as_str()
            .is_some_and(|text| text.contains("fn before()") && text.contains("fn after()"))
    );
    let lines = snippets[0]["lines"]
        .as_array()
        .expect("snippet lines array");
    assert_eq!(lines[0]["kind"], "before");
    assert_eq!(lines[1]["kind"], "match");
    assert_eq!(lines[2]["kind"], "after");
}

#[test]
fn search_content_default_payload_omits_rich_snippets() {
    let (_dir, root) = make_repo(&[("src/lib.rs", "fn target() {}\n")]);
    let args = serde_json::json!({ "query": "target", "exclude_generated": false });
    let resp = tool_search_content(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(
        v.get("rich_snippets").is_none(),
        "default payload should stay compact: {v}"
    );
}

#[test]
fn search_content_empty_or_null_subpath_matches_omitted_root_scope() {
    let (_dir, root) = make_repo(&[
        ("src/lib.rs", "fn auth_token() {}"),
        ("docs/notes.md", "auth_token"),
    ]);

    let omitted = parse_tool_json(
        tool_search_content(
            Some(&serde_json::json!({ "query": "auth_token", "exclude_generated": false })),
            &root,
            OutputFormat::Json,
        )
        .expect("omitted subpath"),
    );
    let empty = parse_tool_json(
        tool_search_content(
            Some(&serde_json::json!({ "query": "auth_token", "subpath": "", "exclude_generated": false })),
            &root,
            OutputFormat::Json,
        )
        .expect("empty subpath"),
    );
    let null = parse_tool_json(
        tool_search_content(
            Some(&serde_json::json!({ "query": "auth_token", "subpath": null, "exclude_generated": false })),
            &root,
            OutputFormat::Json,
        )
        .expect("null subpath"),
    );

    assert_eq!(empty, omitted);
    assert_eq!(null, omitted);
}

#[test]
fn search_content_subpath_path_traversal_is_rejected() {
    let (_dir, root) = make_repo(&[("src/lib.rs", "fn x() {}")]);
    for bad in &["../", "../../etc", "/etc"] {
        let args = serde_json::json!({
            "query": "fn",
            "subpath": bad,
            "exclude_generated": false
        });
        let result = tool_search_content(Some(&args), &root, OutputFormat::Json)
            .expect("path traversal should return tool error result");
        assert_eq!(result["isError"], serde_json::json!(true));
        assert_eq!(
            result["structuredContent"]["code"],
            serde_json::json!("invalid_input"),
            "subpath '{bad}' should be rejected as traversal attempt"
        );
    }
}

#[test]
fn search_content_subpath_limits_to_subdir() {
    let (_dir, root) = make_repo(&[
        ("services/auth/main.rs", "fn auth_token() {}"),
        ("services/billing/main.rs", "fn auth_token() {}"),
    ]);
    let args = serde_json::json!({
        "query": "auth_token",
        "subpath": "services/auth",
        "exclude_generated": false
    });
    let resp = tool_search_content(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    let ms = v["matches"].as_array().unwrap();
    assert!(
        !ms.iter()
            .any(|m| m["file"].as_str().unwrap().contains("billing")),
        "billing should be excluded by subpath: {ms:?}"
    );
}

#[test]
fn search_content_exclude_globs_skips_matched() {
    let (_dir, root) = make_repo(&[
        ("generated/api.rs", "fn do_thing() {}"),
        ("src/lib.rs", "fn do_thing() {}"),
    ]);
    let args = serde_json::json!({
        "query": "do_thing",
        "exclude_globs": ["generated/**"],
        "exclude_generated": false
    });
    let resp = tool_search_content(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    let ms = v["matches"].as_array().unwrap();
    assert!(
        !ms.iter()
            .any(|m| m["file"].as_str().unwrap().contains("generated")),
        "generated leaked: {ms:?}"
    );
    assert!(
        ms.iter()
            .any(|m| m["file"].as_str().unwrap().ends_with("lib.rs")),
        "lib.rs missing: {ms:?}"
    );
}
