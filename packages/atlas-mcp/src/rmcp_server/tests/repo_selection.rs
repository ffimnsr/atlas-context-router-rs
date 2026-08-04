//! Repo-selection tests: explicit selectors, dynamic multi-workspace
//! fail-closed behavior, and roots/list canonicalization.

use super::*;
use crate::transport::ActiveRepoContext;
use rmcp::model::Root;

#[test]
fn explicit_repo_root_selector_is_canonicalized_on_rmcp_tool_path() {
    let repo_a = setup_graph_repo_fixture("src/alpha.rs", "compute", "src/alpha.rs::fn::compute");
    let repo_b = setup_graph_repo_fixture("src/beta.rs", "compute", "src/beta.rs::fn::compute");
    let repo_b_root = repo_b
        ._dir
        .path()
        .join("src")
        .to_string_lossy()
        .into_owned();
    let server = AtlasRmcpServer::new(
        repo_a._dir.path().to_string_lossy().as_ref(),
        &repo_a.db_path,
        ServerOptions::default(),
    );
    let response = server
        .call_tool_for_tests(call_tool_request(
            "query_graph",
            Some(json!({
                "repo_root": repo_b_root,
                "text": "compute",
                "output_format": "json"
            })),
        ))
        .expect("rmcp explicit repo selector");
    let complete = expect_complete(response);
    assert_eq!(
        complete
            .meta
            .as_ref()
            .and_then(|meta| meta.get("atlas:repoRoot")),
        Some(&json!(
            repo_b
                ._dir
                .path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        ))
    );
}

#[test]
fn explicit_repo_selector_switches_active_repo_in_dynamic_mode() {
    let repo_a = setup_graph_repo_fixture(
        "src/alpha.rs",
        "alpha_compute",
        "src/alpha.rs::fn::alpha_compute",
    );
    let repo_b = setup_graph_repo_fixture(
        "src/beta.rs",
        "beta_compute",
        "src/beta.rs::fn::beta_compute",
    );
    let repo_a_root = repo_a
        ._dir
        .path()
        .canonicalize()
        .expect("canonical repo a")
        .to_string_lossy()
        .into_owned();
    let repo_b_root = repo_b
        ._dir
        .path()
        .canonicalize()
        .expect("canonical repo b")
        .to_string_lossy()
        .into_owned();
    let server = AtlasRmcpServer::new_with_dynamic_roots(None, None, ServerOptions::default());
    server.set_candidate_roots_for_tests(Some(vec![repo_a_root, repo_b_root.clone()]));

    let first = server
        .call_tool_for_tests(call_tool_request(
            "query_graph",
            Some(json!({
                "repo_root": repo_b_root.clone(),
                "text": "beta_compute"
            })),
        ))
        .expect("dynamic explicit repo selector");
    let first = expect_complete(first);
    assert_eq!(
        first
            .meta
            .as_ref()
            .and_then(|meta| meta.get("atlas:repoRoot")),
        Some(&json!(repo_b_root.clone()))
    );
    assert_eq!(
        server.repo_resolution().active,
        Some(ActiveRepoContext {
            repo_root: repo_b_root.clone(),
            db_path: atlas_engine::paths::default_db_path(&repo_b_root),
        })
    );

    let second = server
        .call_tool_for_tests(call_tool_request(
            "query_graph",
            Some(json!({
                "text": "beta_compute"
            })),
        ))
        .expect("cached active repo after explicit selector");
    let second = expect_complete(second);
    assert_eq!(
        second
            .meta
            .as_ref()
            .and_then(|meta| meta.get("atlas:repoSelection"))
            .and_then(|value| value.get("selectionSource")),
        Some(&json!("cached_active_root"))
    );
    assert_eq!(
        second
            .meta
            .as_ref()
            .and_then(|meta| meta.get("atlas:repoRoot")),
        Some(&json!(repo_b_root))
    );
}

#[test]
fn dynamic_multi_workspace_without_selector_fails_closed_with_candidate_roots() {
    let repo_a = setup_graph_repo_fixture(
        "src/alpha.rs",
        "alpha_compute",
        "src/alpha.rs::fn::alpha_compute",
    );
    let repo_b = setup_graph_repo_fixture(
        "src/beta.rs",
        "beta_compute",
        "src/beta.rs::fn::beta_compute",
    );
    let repo_a_root = repo_a
        ._dir
        .path()
        .canonicalize()
        .expect("canonical repo a")
        .to_string_lossy()
        .into_owned();
    let repo_b_root = repo_b
        ._dir
        .path()
        .canonicalize()
        .expect("canonical repo b")
        .to_string_lossy()
        .into_owned();
    let server = AtlasRmcpServer::new_with_dynamic_roots(None, None, ServerOptions::default());
    server.set_candidate_roots_for_tests(Some(vec![repo_a_root.clone(), repo_b_root.clone()]));

    let error = server
        .call_tool_for_tests(call_tool_request(
            "query_graph",
            Some(json!({
                "text": "compute"
            })),
        ))
        .expect_err("dynamic ambiguity should fail closed");
    assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
    assert!(
        error
            .message
            .contains("ambiguous across multiple workspace roots")
    );
    let data = error.data.expect("repo-selection error data");
    assert_eq!(data["atlas_error_code"], json!("invalid_params"));
    assert_eq!(
        data["atlas_repo_selection"]["candidate_roots"],
        json!([repo_a_root, repo_b_root])
    );
    assert_eq!(
        data["atlas_repo_selection"]["session_mode"],
        json!("dynamic")
    );
}

#[test]
fn roots_list_changed_invalidates_cached_candidate_roots_in_dynamic_mode() {
    let server = AtlasRmcpServer::new_with_dynamic_roots(None, None, ServerOptions::default());
    server.set_candidate_roots_for_tests(Some(vec!["/tmp/demo".to_owned()]));
    server.invalidate_dynamic_roots();
    assert_eq!(server.repo_resolution().candidate_roots, None);
}

#[test]
fn roots_list_canonicalizes_noncanonical_file_uris() {
    let repo = tempfile::tempdir().expect("repo tempdir");
    fs::create_dir_all(repo.path().join(".git")).expect("create git dir");
    fs::create_dir_all(repo.path().join("src")).expect("create src");
    let nested = repo.path().join("src");
    let uri = url::Url::from_file_path(&nested)
        .expect("file url")
        .to_string();
    let server = AtlasRmcpServer::new_with_dynamic_roots(None, None, ServerOptions::default());
    let roots = vec![Root::new(uri)];
    let canonical = server.load_client_roots_result(&roots).expect("load roots");
    assert_eq!(canonical.len(), 1);
    assert_eq!(
        canonical[0],
        repo.path()
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    );
    assert_eq!(server.repo_resolution().candidate_roots, Some(canonical));
}
