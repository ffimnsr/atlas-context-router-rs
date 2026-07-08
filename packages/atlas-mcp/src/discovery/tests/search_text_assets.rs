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
// search_text_assets
// -----------------------------------------------------------------------

#[test]
fn search_text_assets_finds_sql_files() {
    let (_dir, root) = make_repo(&[
        ("migrations/001_init.sql", "CREATE TABLE users;"),
        ("src/main.rs", "fn main() {}"),
    ]);
    let args = serde_json::json!({ "kind": "sql" });
    let resp = tool_search_text_assets(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    let files: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    assert!(
        files.iter().any(|f| f.ends_with("001_init.sql")),
        "{files:?}"
    );
    assert!(!files.iter().any(|f| f.ends_with("main.rs")), "{files:?}");
    assert_eq!(v["atlas_result_kind"], "text_asset_files");
}

#[test]
fn search_text_assets_finds_config_files() {
    let (_dir, root) = make_repo(&[
        ("config/app.toml", "[server]"),
        ("config/db.yaml", "host: localhost"),
        ("src/lib.rs", ""),
    ]);
    let args = serde_json::json!({ "kind": "config" });
    let resp = tool_search_text_assets(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    let files: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    assert!(files.iter().any(|f| f.ends_with("app.toml")), "{files:?}");
    assert!(files.iter().any(|f| f.ends_with("db.yaml")), "{files:?}");
}

#[test]
fn search_text_assets_finds_prompt_files() {
    let (_dir, root) = make_repo(&[
        ("prompts/review.md", "Review this code"),
        ("docs/guide.md", "# Guide"),
        ("system.prompt", "You are an assistant"),
    ]);
    let args = serde_json::json!({ "kind": "prompt" });
    let resp = tool_search_text_assets(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    let files: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    assert!(
        files.iter().any(|f| f.ends_with("system.prompt")),
        "system.prompt missing: {files:?}"
    );
    // prompts/*.md should match
    assert!(
        files.iter().any(|f| f.contains("prompts/review.md")),
        "prompts/review.md missing: {files:?}"
    );
    // docs/guide.md should NOT match (not in prompts/ and not a .prompt file)
    assert!(
        !files.iter().any(|f| f.ends_with("guide.md")),
        "guide.md leaked: {files:?}"
    );
}

#[test]
fn search_text_assets_no_results_hint() {
    let (_dir, root) = make_repo(&[("src/main.rs", "fn main() {}")]);
    let args = serde_json::json!({ "kind": "sql" });
    let resp = tool_search_text_assets(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(v["result_count"], 0);
    assert!(v["atlas_hint"].is_string(), "expected hint");
}

#[test]
fn search_text_assets_default_finds_multiple_kinds() {
    let (_dir, root) = make_repo(&[
        ("schema.sql", "CREATE TABLE x;"),
        ("config.toml", "[section]"),
        ("deploy.yaml", "service: web"),
    ]);
    let args = serde_json::json!({});
    let resp = tool_search_text_assets(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(v["result_count"].as_u64().unwrap() >= 3, "{v}");
}

#[test]
fn search_text_assets_subpath_scoping() {
    let (_dir, root) = make_repo(&[
        ("services/auth/db.sql", "SELECT 1;"),
        ("services/billing/db.sql", "SELECT 2;"),
    ]);
    let args = serde_json::json!({ "kind": "sql", "subpath": "services/auth" });
    let resp = tool_search_text_assets(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    let files: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    assert!(
        files.iter().any(|f| f.contains("auth/db.sql")),
        "auth/db.sql missing: {files:?}"
    );
    assert!(
        !files.iter().any(|f| f.contains("billing")),
        "billing leaked: {files:?}"
    );
}

#[test]
fn search_text_assets_empty_or_null_subpath_matches_omitted_root_scope() {
    let (_dir, root) = make_repo(&[
        ("services/auth/db.sql", "SELECT 1;"),
        ("src/lib.rs", "fn x() {}"),
    ]);

    let omitted = parse_tool_json(
        tool_search_text_assets(
            Some(&serde_json::json!({ "kind": "sql" })),
            &root,
            OutputFormat::Json,
        )
        .expect("omitted subpath"),
    );
    let empty = parse_tool_json(
        tool_search_text_assets(
            Some(&serde_json::json!({ "kind": "sql", "subpath": "" })),
            &root,
            OutputFormat::Json,
        )
        .expect("empty subpath"),
    );
    let null = parse_tool_json(
        tool_search_text_assets(
            Some(&serde_json::json!({ "kind": "sql", "subpath": null })),
            &root,
            OutputFormat::Json,
        )
        .expect("null subpath"),
    );

    assert_eq!(empty, omitted);
    assert_eq!(null, omitted);
}

#[test]
fn search_text_assets_subpath_path_traversal_is_rejected() {
    let (_dir, root) = make_repo(&[("services/auth/db.sql", "SELECT 1;")]);
    for bad in &["../", "../../etc", "/etc"] {
        let args = serde_json::json!({ "kind": "sql", "subpath": bad });
        let result = tool_search_text_assets(Some(&args), &root, OutputFormat::Json)
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
fn search_text_assets_atlasignore_respected() {
    let (_dir, root) = make_repo(&[
        (".atlasignore", "secret.sql\n"),
        ("secret.sql", "DROP TABLE users;"),
        ("public.sql", "SELECT 1;"),
    ]);
    let args = serde_json::json!({ "kind": "sql" });
    let resp = tool_search_text_assets(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    let files: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    assert!(
        !files.iter().any(|f| f.ends_with("secret.sql")),
        "secret.sql leaked: {files:?}"
    );
    assert!(
        files.iter().any(|f| f.ends_with("public.sql")),
        "public.sql missing: {files:?}"
    );
}
