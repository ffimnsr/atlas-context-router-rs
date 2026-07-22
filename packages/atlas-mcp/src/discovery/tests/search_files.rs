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

fn match_paths(value: &serde_json::Value) -> Vec<&str> {
    value["matches"]
        .as_array()
        .expect("matches array")
        .iter()
        .map(|row| row["path"].as_str().expect("match path"))
        .collect()
}

// -----------------------------------------------------------------------
// search_files
// -----------------------------------------------------------------------

#[test]
fn search_files_finds_markdown() {
    let (_dir, root) = make_repo(&[
        ("README.md", "# hello"),
        ("docs/guide.md", "# guide"),
        ("src/main.rs", "fn main() {}"),
    ]);
    let args = serde_json::json!({ "pattern": "*.md" });
    let resp = tool_search_files(Some(&args), &root, OutputFormat::Json).unwrap();
    let v = parse_tool_json(resp.clone());
    let files = match_paths(&v);
    assert!(files.iter().any(|f| f.ends_with("README.md")), "{files:?}");
    assert!(files.iter().any(|f| f.ends_with("guide.md")), "{files:?}");
    assert!(!files.iter().any(|f| f.ends_with("main.rs")), "{files:?}");
    assert_eq!(v["tool"], "search_files");
    assert_eq!(v["query"]["pattern"], "*.md");
    assert_eq!(v["subpath"], serde_json::Value::Null);
    assert_eq!(v["summary"]["returned_count"], serde_json::json!(2));
    let first_match = &v["matches"].as_array().expect("matches")[0];
    assert!(first_match["file_name"].is_string());
    assert!(first_match["extension"].is_string() || first_match["extension"].is_null());
    assert_eq!(resp["structuredContent"], v);
}

#[test]
fn search_files_finds_sql_config_template() {
    let (_dir, root) = make_repo(&[
        ("schema.sql", "CREATE TABLE foo;"),
        ("config/app.toml", "[section]"),
        ("templates/index.html", "<html></html>"),
        ("src/lib.rs", ""),
    ]);
    for (pattern, expected) in [
        ("*.sql", "schema.sql"),
        ("*.toml", "app.toml"),
        ("*.html", "index.html"),
    ] {
        let args = serde_json::json!({ "pattern": pattern });
        let resp = tool_search_files(Some(&args), &root, OutputFormat::Json).unwrap();
        let v = parse_tool_json(resp);
        let files = match_paths(&v);
        assert!(
            files.iter().any(|f| f.ends_with(expected)),
            "pattern={pattern} expected={expected} got={files:?}"
        );
    }
}

#[test]
fn search_files_gitignore_excludes_node_modules() {
    let (_dir, root) = make_repo(&[
        (".gitignore", "node_modules/\n"),
        ("node_modules/index.js", "// vendor"),
        ("src/main.js", "// src"),
    ]);
    let args = serde_json::json!({ "pattern": "*.js" });
    let resp = tool_search_files(Some(&args), &root, OutputFormat::Json).unwrap();
    let v = parse_tool_json(resp);
    let files = match_paths(&v);
    assert!(
        !files.iter().any(|f| f.contains("node_modules")),
        "node_modules leaked: {files:?}"
    );
    assert!(files.iter().any(|f| f.ends_with("main.js")), "{files:?}");
}

#[test]
fn search_files_atlasignore_respected() {
    let (_dir, root) = make_repo(&[
        (".atlasignore", "secret.rs\n"),
        ("secret.rs", ""),
        ("public.rs", ""),
    ]);
    let args = serde_json::json!({ "pattern": "*.rs" });
    let resp = tool_search_files(Some(&args), &root, OutputFormat::Json).unwrap();
    let v = parse_tool_json(resp);
    let files = match_paths(&v);
    assert!(
        !files.iter().any(|f| f.ends_with("secret.rs")),
        "secret.rs leaked: {files:?}"
    );
    assert!(files.iter().any(|f| f.ends_with("public.rs")), "{files:?}");
}

#[test]
fn search_files_no_results_emit_stable_empty_schema() {
    let (_dir, root) = make_repo(&[("src/main.rs", "")]);
    let args = serde_json::json!({ "pattern": "*.nonexistent" });
    let resp = tool_search_files(Some(&args), &root, OutputFormat::Json).unwrap();
    let v = parse_tool_json(resp);
    assert_eq!(v["matches"], serde_json::json!([]));
    assert_eq!(v["summary"]["returned_count"], serde_json::json!(0));
    assert_eq!(v["summary"]["has_matches"], serde_json::json!(false));
    assert_eq!(v["warnings"].as_array().map(|rows| rows.len()), Some(1));
}

#[test]
fn search_files_empty_or_null_subpath_matches_omitted_root_scope() {
    let (_dir, root) = make_repo(&[("src/lib.rs", "fn x() {}"), ("docs/readme.md", "# hi")]);

    let omitted = parse_tool_json(
        tool_search_files(
            Some(&serde_json::json!({ "pattern": "*.rs" })),
            &root,
            OutputFormat::Json,
        )
        .expect("omitted subpath"),
    );
    let empty = parse_tool_json(
        tool_search_files(
            Some(&serde_json::json!({ "pattern": "*.rs", "subpath": "" })),
            &root,
            OutputFormat::Json,
        )
        .expect("empty subpath"),
    );
    let null = parse_tool_json(
        tool_search_files(
            Some(&serde_json::json!({ "pattern": "*.rs", "subpath": null })),
            &root,
            OutputFormat::Json,
        )
        .expect("null subpath"),
    );

    assert_eq!(empty, omitted);
    assert_eq!(null, omitted);
}

#[test]
fn search_files_subpath_path_traversal_is_rejected() {
    let (_dir, root) = make_repo(&[("src/lib.rs", "fn x() {}")]);
    for bad in &["../", "../../etc", "/etc", "../sibling"] {
        let args = serde_json::json!({ "pattern": "*.rs", "subpath": bad });
        let result = tool_search_files(Some(&args), &root, OutputFormat::Json)
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
fn search_files_subpath_limits_to_subdir() {
    let (_dir, root) = make_repo(&[
        ("services/auth/schema.sql", "CREATE TABLE users;"),
        ("services/billing/schema.sql", "CREATE TABLE invoices;"),
        ("root.sql", "SELECT 1;"),
    ]);
    let args = serde_json::json!({ "pattern": "*.sql", "subpath": "services/auth" });
    let resp = tool_search_files(Some(&args), &root, OutputFormat::Json).unwrap();
    let v = parse_tool_json(resp);
    let files = match_paths(&v);
    assert!(
        files.iter().any(|f| f.contains("auth/schema.sql")),
        "expected auth file: {files:?}"
    );
    assert!(
        !files.iter().any(|f| f.contains("billing")),
        "billing should be excluded by subpath: {files:?}"
    );
    assert_eq!(v["subpath"], serde_json::json!("services/auth"));
    assert_eq!(v["summary"]["scope"], serde_json::json!("subpath"));
}

#[test]
fn search_files_exclude_globs_skips_matched() {
    let (_dir, root) = make_repo(&[
        ("generated/schema.sql", "-- auto"),
        ("src/manual.sql", "-- hand"),
    ]);
    let args = serde_json::json!({
        "pattern": "*.sql",
        "exclude_globs": ["generated/**"]
    });
    let resp = tool_search_files(Some(&args), &root, OutputFormat::Json).unwrap();
    let v = parse_tool_json(resp);
    let files = match_paths(&v);
    assert!(
        !files.iter().any(|f| f.contains("generated")),
        "generated leaked: {files:?}"
    );
    assert!(
        files.iter().any(|f| f.ends_with("manual.sql")),
        "manual.sql missing: {files:?}"
    );
}
