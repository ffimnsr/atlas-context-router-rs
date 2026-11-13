//! Unit tests for the memoir MCP tools (ICM-F).

use super::test_util::{setup_db_path, tool_body};
use super::*;
use tempfile::TempDir;

fn setup() -> (TempDir, String, String) {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_string_lossy().into_owned();
    (dir, repo_root, db_path)
}

fn json_call(
    tool: fn(Option<&serde_json::Value>, &str, &str, OutputFormat) -> anyhow::Result<Value>,
    args: serde_json::Value,
    repo_root: &str,
    db_path: &str,
) -> Value {
    tool_body(&tool(Some(&args), repo_root, db_path, OutputFormat::Json).unwrap())
}

#[test]
fn memoir_tools_create_add_link_and_inspect() {
    let (_dir, repo_root, db_path) = setup();

    let created = json_call(
        tool_memoir_create,
        serde_json::json!({ "name": "Architecture", "description": "system map" }),
        &repo_root,
        &db_path,
    );
    assert_eq!(created["graph"]["name"], "Architecture");
    assert_eq!(created["graph"]["scope"], "project");
    assert_eq!(created["summary"]["is_new"], true);

    let duplicate = tool_memoir_create(
        Some(&serde_json::json!({ "name": "architecture" })),
        &repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .unwrap_err()
    .to_string();
    assert!(
        duplicate.contains("already exists in repo scope 'project'"),
        "duplicate graph names fail deterministically: {duplicate}"
    );

    let _parser = json_call(
        tool_memoir_add_concept,
        serde_json::json!({
            "graph": "Architecture",
            "name": "parser",
            "description": "source parser",
            "source_id": "artifact-parser"
        }),
        &repo_root,
        &db_path,
    );
    let _lexer = json_call(
        tool_memoir_add_concept,
        serde_json::json!({
            "graph": "Architecture",
            "name": "lexer",
            "description": "token lexer"
        }),
        &repo_root,
        &db_path,
    );

    let linked = json_call(
        tool_memoir_link,
        serde_json::json!({
            "graph": "Architecture",
            "source": "parser",
            "target": "lexer",
            "relation": "depends_on",
            "confidence": 0.75,
            "source_id": "evidence-1"
        }),
        &repo_root,
        &db_path,
    );
    assert_eq!(linked["relation"]["relation_type"], "depends_on");
    assert_eq!(linked["relation"]["confidence"], 0.75);
    assert_eq!(linked["relation"]["source_id"], "evidence-1");

    let inspected = json_call(
        tool_memoir_inspect,
        serde_json::json!({
            "graph": "Architecture",
            "concept": "parser",
            "depth": 1
        }),
        &repo_root,
        &db_path,
    );
    let edge = &inspected["inspection"]["edges"][0];
    assert_eq!(edge["direction"], "outgoing");
    assert_eq!(edge["from_concept"], "parser");
    assert_eq!(edge["to_concept"], "lexer");
    assert_eq!(edge["source_id"], "evidence-1");
    assert_eq!(inspected["summary"]["edge_count"], 1);
}

#[test]
fn memoir_link_rejects_unknown_relations_and_requires_explicit_create_missing() {
    let (_dir, repo_root, db_path) = setup();
    let _ = tool_memoir_create(
        Some(&serde_json::json!({ "name": "Architecture" })),
        &repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .unwrap();
    let _ = tool_memoir_add_concept(
        Some(&serde_json::json!({
            "graph": "Architecture",
            "name": "parser",
            "description": "source parser"
        })),
        &repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .unwrap();

    let unknown = tool_memoir_link(
        Some(&serde_json::json!({
            "graph": "Architecture",
            "source": "parser",
            "target": "lexer",
            "relation": "unrelated_to",
            "create_missing": true
        })),
        &repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .unwrap_err()
    .to_string();
    assert!(
        unknown.contains("unknown memoir relation type"),
        "unknown relations must fail clearly: {unknown}"
    );

    let missing = tool_memoir_link(
        Some(&serde_json::json!({
            "graph": "Architecture",
            "source": "parser",
            "target": "lexer",
            "relation": "depends_on"
        })),
        &repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .unwrap_err()
    .to_string();
    assert!(
        missing.contains("not found in graph 'Architecture'"),
        "missing endpoints require opt-in: {missing}"
    );

    let created = json_call(
        tool_memoir_link,
        serde_json::json!({
            "graph": "Architecture",
            "source": "parser",
            "target": "lexer",
            "relation": "depends_on",
            "create_missing": true
        }),
        &repo_root,
        &db_path,
    );
    assert_eq!(created["summary"]["created_missing_concepts"], true);
    assert_eq!(created["relation"]["relation_type"], "depends_on");
}

#[test]
fn memoir_delete_reports_counts_and_removes_graph() {
    let (_dir, repo_root, db_path) = setup();
    let _ = tool_memoir_create(
        Some(&serde_json::json!({ "name": "Architecture" })),
        &repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .unwrap();
    let _ = tool_memoir_add_concept(
        Some(&serde_json::json!({
            "graph": "Architecture",
            "name": "parser",
            "description": "source parser"
        })),
        &repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .unwrap();

    let dry_run = json_call(
        tool_memoir_delete,
        serde_json::json!({ "graph": "Architecture", "dry_run": true }),
        &repo_root,
        &db_path,
    );
    assert_eq!(dry_run["summary"]["found"], true);
    assert_eq!(dry_run["summary"]["deleted"], false);
    assert_eq!(dry_run["summary"]["concepts_deleted"], 1);

    let deleted = json_call(
        tool_memoir_delete,
        serde_json::json!({ "graph": "Architecture" }),
        &repo_root,
        &db_path,
    );
    assert_eq!(deleted["summary"]["deleted"], true);
    assert_eq!(deleted["summary"]["concepts_deleted"], 1);

    let missing = json_call(
        tool_memoir_delete,
        serde_json::json!({ "graph": "Architecture" }),
        &repo_root,
        &db_path,
    );
    assert_eq!(missing["summary"]["found"], false);
    assert_eq!(missing["summary"]["deleted"], false);
}
