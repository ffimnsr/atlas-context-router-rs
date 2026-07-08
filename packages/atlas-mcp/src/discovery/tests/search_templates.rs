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
// search_templates
// -----------------------------------------------------------------------

#[test]
fn search_templates_finds_html_files() {
    let (_dir, root) = make_repo(&[
        ("templates/index.html", "<html></html>"),
        ("templates/base.html", "<html></html>"),
        ("src/main.rs", "fn main() {}"),
    ]);
    let args = serde_json::json!({ "kind": "html" });
    let resp = tool_search_templates(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    let files: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    assert!(files.iter().any(|f| f.ends_with("index.html")), "{files:?}");
    assert!(!files.iter().any(|f| f.ends_with("main.rs")), "{files:?}");
    assert_eq!(v["atlas_result_kind"], "template_files");
}

#[test]
fn search_templates_finds_jinja_files() {
    let (_dir, root) = make_repo(&[
        ("templates/email.j2", "Hello {{ name }}"),
        ("templates/layout.jinja2", "{% block %}{% endblock %}"),
        ("src/lib.rs", ""),
    ]);
    let args = serde_json::json!({ "kind": "jinja" });
    let resp = tool_search_templates(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    let files: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    assert!(files.iter().any(|f| f.ends_with("email.j2")), "{files:?}");
    assert!(
        files.iter().any(|f| f.ends_with("layout.jinja2")),
        "{files:?}"
    );
}

#[test]
fn search_templates_no_results_hint() {
    let (_dir, root) = make_repo(&[("src/main.rs", "fn main() {}")]);
    let args = serde_json::json!({ "kind": "haml" });
    let resp = tool_search_templates(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(v["result_count"], 0);
    assert!(v["atlas_hint"].is_string(), "expected hint on empty result");
}

#[test]
fn search_templates_default_finds_multiple_kinds() {
    let (_dir, root) = make_repo(&[
        ("a.html", "<html/>"),
        ("b.hbs", "{{> partial}}"),
        ("c.j2", "{{ var }}"),
        ("d.tera", "{% if x %}{% endif %}"),
    ]);
    let args = serde_json::json!({});
    let resp = tool_search_templates(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    assert!(v["result_count"].as_u64().unwrap() >= 4, "{v}");
}

#[test]
fn search_templates_exclude_globs() {
    let (_dir, root) = make_repo(&[
        ("generated/page.html", "<html/>"),
        ("src/index.html", "<html/>"),
    ]);
    let args = serde_json::json!({
        "kind": "html",
        "exclude_globs": ["generated/**"]
    });
    let resp = tool_search_templates(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    let files: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    assert!(
        !files.iter().any(|f| f.contains("generated")),
        "generated leaked: {files:?}"
    );
    assert!(
        files.iter().any(|f| f.ends_with("index.html")),
        "index.html missing: {files:?}"
    );
}

#[test]
fn search_templates_gitignore_excluded() {
    let (_dir, root) = make_repo(&[
        (".gitignore", "vendor/\n"),
        ("vendor/base.html", "<html/>"),
        ("src/index.html", "<html/>"),
    ]);
    let args = serde_json::json!({ "kind": "html" });
    let resp = tool_search_templates(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();
    let files: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f.as_str().unwrap())
        .collect();
    assert!(
        !files.iter().any(|f| f.contains("vendor")),
        "vendor leaked: {files:?}"
    );
}

#[test]
fn search_templates_empty_or_null_subpath_matches_omitted_root_scope() {
    let (_dir, root) = make_repo(&[
        ("templates/index.html", "<html></html>"),
        ("src/main.rs", "fn main() {}"),
    ]);

    let omitted = parse_tool_json(
        tool_search_templates(
            Some(&serde_json::json!({ "kind": "html" })),
            &root,
            OutputFormat::Json,
        )
        .expect("omitted subpath"),
    );
    let empty = parse_tool_json(
        tool_search_templates(
            Some(&serde_json::json!({ "kind": "html", "subpath": "" })),
            &root,
            OutputFormat::Json,
        )
        .expect("empty subpath"),
    );
    let null = parse_tool_json(
        tool_search_templates(
            Some(&serde_json::json!({ "kind": "html", "subpath": null })),
            &root,
            OutputFormat::Json,
        )
        .expect("null subpath"),
    );

    assert_eq!(empty, omitted);
    assert_eq!(null, omitted);
}

#[test]
fn search_templates_subpath_path_traversal_is_rejected() {
    let (_dir, root) = make_repo(&[("templates/index.html", "<html></html>")]);
    for bad in &["../", "../../etc", "/etc"] {
        let args = serde_json::json!({ "kind": "html", "subpath": bad });
        let result = tool_search_templates(Some(&args), &root, OutputFormat::Json)
            .expect("path traversal should return tool error result");
        assert_eq!(result["isError"], serde_json::json!(true));
        assert_eq!(
            result["structuredContent"]["code"],
            serde_json::json!("invalid_input"),
            "subpath '{bad}' should be rejected as traversal attempt"
        );
    }
}
