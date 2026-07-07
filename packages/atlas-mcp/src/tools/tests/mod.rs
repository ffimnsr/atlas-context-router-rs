use super::{call, tool_list};
use crate::output::OutputFormat;
use atlas_contentstore::{ContentStore, IndexingStats};
use atlas_core::EdgeKind;
use atlas_core::error_code_docs_ref;
use atlas_core::kinds::NodeKind;
use atlas_core::model::{Edge, Node, NodeId};
use atlas_store_sqlite::{BuildFinishStats, GraphBuildState, Store};
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

pub(super) use super::shared::{DEFAULT_OUTPUT_DESCRIPTION, tool_result_value};

mod analysis;
mod context_ops;
mod graph;
mod health;
mod readiness;
mod registry;

pub(super) struct McpFixture {
    pub(super) _dir: TempDir,
    pub(super) db_path: String,
}

pub(super) struct GitMcpFixture {
    pub(super) _dir: TempDir,
    pub(super) repo_root: String,
    pub(super) db_path: String,
}

pub(super) fn git_run(dir: &std::path::Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_CONFIG")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_CONFIG_KEY_0")
        .env_remove("GIT_CONFIG_VALUE_0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_GRAFT_FILE")
        .env_remove("GIT_IMPLICIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_INTERNAL_SUPER_PREFIX")
        .env_remove("GIT_NAMESPACE")
        .env_remove("GIT_NO_REPLACE_OBJECTS")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_PREFIX")
        .env_remove("GIT_REPLACE_REF_BASE")
        .env_remove("GIT_SHALLOW_FILE")
        .env_remove("GIT_WORK_TREE")
        .env("GIT_AUTHOR_NAME", "Atlas Tests")
        .env("GIT_AUTHOR_EMAIL", "atlas-tests@example.com")
        .env("GIT_COMMITTER_NAME", "Atlas Tests")
        .env("GIT_COMMITTER_EMAIL", "atlas-tests@example.com")
        .status()
        .expect("git command");
    assert!(status.success(), "git {args:?} failed");
}

pub(super) fn write_repo_file(dir: &std::path::Path, rel_path: &str, contents: &str) {
    let path = dir.join(rel_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dirs");
    }
    std::fs::write(path, contents).expect("write repo file");
}

pub(super) fn make_node(kind: NodeKind, name: &str, qualified_name: &str, file_path: &str) -> Node {
    Node {
        id: NodeId::UNSET,
        kind,
        name: name.to_owned(),
        qualified_name: qualified_name.to_owned(),
        file_path: file_path.to_owned(),
        line_start: 1,
        line_end: 5,
        language: "rust".to_owned(),
        parent_name: None,
        params: Some("()".to_owned()),
        return_type: None,
        modifiers: Some("pub".to_owned()),
        is_test: kind == NodeKind::Test,
        file_hash: format!("hash:{file_path}"),
        extra_json: serde_json::json!({}),
    }
}

pub(super) fn make_edge(kind: EdgeKind, source_qn: &str, target_qn: &str, file_path: &str) -> Edge {
    Edge {
        id: 0,
        kind,
        source_qn: source_qn.to_owned(),
        target_qn: target_qn.to_owned(),
        file_path: file_path.to_owned(),
        line: Some(1),
        confidence: 1.0,
        confidence_tier: Some("high".to_owned()),
        extra_json: serde_json::json!({}),
    }
}

pub(super) fn setup_mcp_fixture() -> McpFixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();

    let mut store = Store::open(&db_path).expect("open store");

    let compute = make_node(
        NodeKind::Function,
        "compute",
        "src/service.rs::fn::compute",
        "src/service.rs",
    );
    store
        .replace_file_graph(
            "src/service.rs",
            "hash:src/service.rs",
            Some("rust"),
            Some(5),
            std::slice::from_ref(&compute),
            &[],
        )
        .expect("replace service graph");

    let handle = make_node(
        NodeKind::Function,
        "handle_request",
        "src/api.rs::fn::handle_request",
        "src/api.rs",
    );
    let handle_calls_compute = make_edge(
        EdgeKind::Calls,
        "src/api.rs::fn::handle_request",
        "src/service.rs::fn::compute",
        "src/api.rs",
    );
    store
        .replace_file_graph(
            "src/api.rs",
            "hash:src/api.rs",
            Some("rust"),
            Some(5),
            std::slice::from_ref(&handle),
            &[handle_calls_compute],
        )
        .expect("replace api graph");

    let compute_test = make_node(
        NodeKind::Test,
        "compute_test",
        "tests/service_test.rs::fn::compute_test",
        "tests/service_test.rs",
    );
    let test_targets_compute = make_edge(
        EdgeKind::Tests,
        "tests/service_test.rs::fn::compute_test",
        "src/service.rs::fn::compute",
        "tests/service_test.rs",
    );
    store
        .replace_file_graph(
            "tests/service_test.rs",
            "hash:tests/service_test.rs",
            Some("rust"),
            Some(5),
            std::slice::from_ref(&compute_test),
            &[test_targets_compute],
        )
        .expect("replace test graph");

    let content_db_path = atlas_engine::paths::content_db_path(&db_path);
    let mut content_store = ContentStore::open(&content_db_path).expect("open content store");
    content_store.migrate().expect("migrate content store");
    content_store
        .begin_indexing("/repo", 3)
        .expect("begin indexing");
    content_store
        .finish_indexing(
            "/repo",
            &IndexingStats {
                files_indexed: 3,
                chunks_written: 3,
                chunks_reused: 0,
            },
        )
        .expect("finish indexing");

    McpFixture { _dir: dir, db_path }
}

pub(super) fn setup_git_mcp_fixture() -> GitMcpFixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    git_run(root, &["init", "--quiet"]);
    git_run(root, &["config", "user.email", "atlas-tests@example.com"]);
    git_run(root, &["config", "user.name", "Atlas Tests"]);

    write_repo_file(root, "src/service.rs", "pub fn compute() -> i32 { 1 }\n");
    write_repo_file(
        root,
        "src/api.rs",
        "pub fn handle_request() -> i32 { crate::service::compute() }\n",
    );
    write_repo_file(
        root,
        "tests/service_test.rs",
        "#[test]\nfn compute_test() { assert_eq!(1, 1); }\n",
    );
    write_repo_file(
        root,
        "README.md",
        "# Overview\nfixture docs\n## Install\nstep\n",
    );

    git_run(root, &["add", "-A"]);
    git_run(root, &["commit", "--quiet", "-m", "initial"]);

    let db_path = root.join("atlas.db").to_string_lossy().to_string();
    let mut store = Store::open(&db_path).expect("open store");

    let compute = make_node(
        NodeKind::Function,
        "compute",
        "src/service.rs::fn::compute",
        "src/service.rs",
    );
    store
        .replace_file_graph(
            "src/service.rs",
            "hash:src/service.rs",
            Some("rust"),
            Some(5),
            std::slice::from_ref(&compute),
            &[],
        )
        .expect("replace service graph");

    let handle = make_node(
        NodeKind::Function,
        "handle_request",
        "src/api.rs::fn::handle_request",
        "src/api.rs",
    );
    let handle_calls_compute = make_edge(
        EdgeKind::Calls,
        "src/api.rs::fn::handle_request",
        "src/service.rs::fn::compute",
        "src/api.rs",
    );
    store
        .replace_file_graph(
            "src/api.rs",
            "hash:src/api.rs",
            Some("rust"),
            Some(5),
            std::slice::from_ref(&handle),
            &[handle_calls_compute],
        )
        .expect("replace api graph");

    let compute_test = make_node(
        NodeKind::Test,
        "compute_test",
        "tests/service_test.rs::fn::compute_test",
        "tests/service_test.rs",
    );
    let test_targets_compute = make_edge(
        EdgeKind::Tests,
        "tests/service_test.rs::fn::compute_test",
        "src/service.rs::fn::compute",
        "tests/service_test.rs",
    );
    store
        .replace_file_graph(
            "tests/service_test.rs",
            "hash:tests/service_test.rs",
            Some("rust"),
            Some(5),
            std::slice::from_ref(&compute_test),
            &[test_targets_compute],
        )
        .expect("replace test graph");

    let readme_heading = Node {
        id: NodeId::UNSET,
        kind: NodeKind::Module,
        name: "Overview".to_owned(),
        qualified_name: "README.md::heading::document.overview".to_owned(),
        file_path: "README.md".to_owned(),
        line_start: 1,
        line_end: 1,
        language: "markdown".to_owned(),
        parent_name: None,
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: "hash:README.md".to_owned(),
        extra_json: serde_json::json!({ "level": 1, "path": "document.overview" }),
    };
    let install_heading = Node {
        id: NodeId::UNSET,
        kind: NodeKind::Module,
        name: "Install".to_owned(),
        qualified_name: "README.md::heading::document.overview.install".to_owned(),
        file_path: "README.md".to_owned(),
        line_start: 3,
        line_end: 3,
        language: "markdown".to_owned(),
        parent_name: None,
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: "hash:README.md".to_owned(),
        extra_json: serde_json::json!({ "level": 2, "path": "document.overview.install" }),
    };
    store
        .replace_file_graph(
            "README.md",
            "hash:README.md",
            Some("markdown"),
            Some(4),
            &[readme_heading, install_heading],
            &[],
        )
        .expect("replace readme graph");

    let content_db_path = atlas_engine::paths::content_db_path(&db_path);
    let mut content_store = ContentStore::open(&content_db_path).expect("open content store");
    content_store.migrate().expect("migrate content store");
    content_store
        .begin_indexing(&root.to_string_lossy(), 4)
        .expect("begin indexing");
    content_store
        .finish_indexing(
            &root.to_string_lossy(),
            &IndexingStats {
                files_indexed: 4,
                chunks_written: 4,
                chunks_reused: 0,
            },
        )
        .expect("finish indexing");

    store
        .finish_build(
            &root.to_string_lossy(),
            BuildFinishStats {
                state: GraphBuildState::Built,
                files_discovered: 4,
                files_processed: 4,
                files_accepted: 4,
                files_skipped_by_byte_budget: 0,
                files_failed: 0,
                bytes_accepted: 400,
                bytes_skipped: 0,
                nodes_written: 5,
                edges_written: 2,
                budget_stop_reason: None,
            },
        )
        .expect("finish build");

    GitMcpFixture {
        repo_root: root.to_string_lossy().to_string(),
        db_path,
        _dir: dir,
    }
}

pub(super) fn unwrap_tool_text(resp: serde_json::Value) -> String {
    resp.get("content")
        .and_then(|c| c.as_array())
        .and_then(|c| c.first())
        .and_then(|c0| c0.get("text"))
        .and_then(|t| t.as_str())
        .expect("tool response content[0].text")
        .to_owned()
}

pub(super) fn unwrap_tool_format(resp: &serde_json::Value) -> &str {
    resp.pointer("/_meta/atlas:outputFormat")
        .and_then(|value| value.as_str())
        .expect("tool response _meta.atlas:outputFormat")
}

pub(super) fn fallback_reason(resp: &serde_json::Value) -> Option<&str> {
    resp.pointer("/_meta/atlas:fallbackReason")
        .and_then(|value| value.as_str())
}

pub(super) fn assert_provenance(resp: &serde_json::Value, expected_repo: &str, expected_db: &str) {
    let prov = resp
        .get("atlas_provenance")
        .expect("atlas_provenance must be present");
    assert_eq!(
        prov.get("repo_root").and_then(|v| v.as_str()),
        Some(expected_repo),
        "provenance.repo_root mismatch"
    );
    assert_eq!(
        prov.get("db_path").and_then(|v| v.as_str()),
        Some(expected_db),
        "provenance.db_path mismatch"
    );
    assert!(
        prov.get("indexed_file_count")
            .and_then(|v| v.as_i64())
            .is_some(),
        "provenance.indexed_file_count must be an integer"
    );
    assert!(
        prov.get("last_indexed_at").is_some(),
        "provenance.last_indexed_at key must be present"
    );
}

pub(super) fn assert_error_code_doc_link(actual: &str, error_code: &str) {
    assert_eq!(actual, error_code_docs_ref(error_code));

    let catalog_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/error_codes.md");
    let catalog = std::fs::read_to_string(&catalog_path).expect("read docs/error_codes.md");
    assert!(
        catalog.contains(&format!("<a id=\"{error_code}\"></a>")),
        "docs/error_codes.md missing anchor for {error_code}"
    );
}
