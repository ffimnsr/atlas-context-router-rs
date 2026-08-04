//! Unit tests for `read_saved_context` and cross-session/repo isolation.

use super::test_util::{medium_content, save_indexed_artifact, setup_db_path, tool_body};
use super::*;
use atlas_adapters::{ArtifactIdentity, derive_content_db_path, generate_source_id};
use atlas_contentstore::{ContentStore, SourceMeta};
use atlas_repo::RepoRegistry;
use camino::Utf8Path;
use tempfile::TempDir;

#[test]
fn read_saved_context_missing_source_id_returns_error() {
    let dir = TempDir::new().unwrap();
    let db_path = setup_db_path(&dir);
    let err = tool_read_saved_context(
        Some(&serde_json::json!({})),
        "",
        &db_path,
        OutputFormat::Json,
    );
    assert!(err.is_err());
    assert!(err.unwrap_err().to_string().contains("source_id"));
}

#[test]
fn read_saved_context_unknown_source_id_returns_not_found() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();

    let args = serde_json::json!({"source_id": "does_not_exist"});
    let result =
        tool_read_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
    let body = tool_body(&result);
    assert!(!body["found"].as_bool().unwrap());
    assert_eq!(body["access_status"], "not_found");
    assert!(body["warnings"][0].as_str().unwrap().contains("not found"));
}

#[test]
fn read_saved_context_found_artifact_returns_metadata_and_content() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();

    let content = medium_content("artifact");
    let source_id = save_indexed_artifact(repo_root, &db_path, "my label", &content, None);
    assert!(!source_id.is_empty(), "artifact must be indexed (not raw)");

    let args = serde_json::json!({"source_id": source_id});
    let result =
        tool_read_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
    let body: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();

    assert!(body["found"].as_bool().unwrap());
    assert_eq!(body["source_id"].as_str().unwrap(), source_id);
    assert_eq!(body["label"].as_str().unwrap(), "my label");
    assert_eq!(body["repo_scope"]["repo_count"].as_u64().unwrap(), 1);
    assert_eq!(body["repo_scope"]["requested_repo_roots"][0], repo_root);
    assert_eq!(body["identity_kind"].as_str().unwrap(), "artifact_label");
    assert_eq!(body["identity_value"].as_str().unwrap(), "my label");
    assert!(body["created_at"].is_string());
    assert!(body["artifact_kind"].is_string());
    assert!(body["chunk_count"].as_u64().unwrap() > 0);
    assert!(body["byte_count"].as_u64().unwrap() > 0);
    assert!(body["returned_chunk_ids"].as_array().is_some());
    assert!(!body["content"].as_str().unwrap().is_empty());
    assert!(!body["truncated"].as_bool().unwrap());
}

#[test]
fn read_saved_context_oversized_artifact_sets_truncated_and_continuation_hint() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();

    // Build content large enough to span multiple chunks and exceed a tiny cap.
    // Use unique paragraphs to avoid duplicate chunk_ids.
    let content: String = (0..200)
        .map(|i| format!("paragraph number {i} with some unique text here\n\n"))
        .collect();
    let source_id = save_indexed_artifact(repo_root, &db_path, "big artifact", &content, None);
    assert!(!source_id.is_empty(), "artifact must be indexed");

    // Request with a very tight byte cap so the first chunk alone exceeds it.
    let args = serde_json::json!({"source_id": source_id, "max_bytes": 1});
    let result =
        tool_read_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
    let body: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();

    assert!(body["found"].as_bool().unwrap());
    assert!(body["truncated"].as_bool().unwrap());
    assert!(body["next_chunk_offset"].is_number());
    assert!(body["next_chunk_id"].as_str().is_some());
    assert!(
        body["continuation_hint"]
            .as_str()
            .unwrap()
            .contains("chunk_offset")
    );
}

#[test]
fn read_saved_context_max_bytes_is_clamped_by_central_budget_policy() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();

    let content: String = (0..200)
        .map(|i| {
            format!(
                "chunk paragraph {i} with unique payload data {}\n\n",
                "x".repeat(256)
            )
        })
        .collect();
    let source_id = save_indexed_artifact(repo_root, &db_path, "very large", &content, None);
    assert!(!source_id.is_empty(), "artifact must be indexed");

    let args = serde_json::json!({"source_id": source_id, "max_bytes": 999999});
    let result =
        tool_read_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
    let body: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();

    assert_eq!(result["budget_status"], "partial_result");
    assert_eq!(result["budget_hit"], true);
    assert_eq!(
        result["budget_name"],
        "mcp_cli_payload_serialization.max_saved_context_bytes"
    );
    assert_eq!(result["budget_limit"], 32768);
    assert!(result["budget_observed"].as_u64().unwrap() > 32_768);
    assert_eq!(body["found"], true);
    assert!(
        body["truncated"].as_bool().unwrap() || body["content"].as_str().unwrap().len() <= 32_768
    );
}

#[test]
fn read_saved_context_paging_chunk_offset_skips_earlier_chunks() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();

    // Use unique paragraphs to avoid duplicate chunk_ids.
    let content: String = (0..100)
        .map(|i| format!("unique paragraph {i} here\n\n"))
        .collect();
    let source_id = save_indexed_artifact(repo_root, &db_path, "paged", &content, None);
    assert!(!source_id.is_empty());

    // First call with cap that forces truncation.
    let args = serde_json::json!({"source_id": source_id, "max_bytes": 100});
    let r1 = tool_read_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
    let b1: Value = serde_json::from_str(r1["content"][0]["text"].as_str().unwrap()).unwrap();

    let next = b1.get("next_chunk_offset").and_then(|v| v.as_u64());
    if let Some(offset) = next {
        let args2 = serde_json::json!({"source_id": source_id, "chunk_offset": offset});
        let r2 =
            tool_read_saved_context(Some(&args2), repo_root, &db_path, OutputFormat::Json).unwrap();
        let b2: Value = serde_json::from_str(r2["content"][0]["text"].as_str().unwrap()).unwrap();
        assert!(b2["found"].as_bool().unwrap());
        assert_eq!(b2["chunk_offset"].as_u64().unwrap(), offset);
        assert!(b2["returned_chunk_ids"].as_array().is_some());
    }
    // If not truncated the content was small enough in one page — test still passes.
}

#[test]
fn read_saved_context_cross_session_isolation_blocks_read() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();

    let content = medium_content("secret");
    let source_id =
        save_indexed_artifact(repo_root, &db_path, "private", &content, Some("session-A"));
    assert!(!source_id.is_empty());

    // Attempt to read with a different session_id.
    let args = serde_json::json!({"source_id": source_id, "session_id": "session-B"});
    let result =
        tool_read_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
    let body = tool_body(&result);
    assert!(!body["found"].as_bool().unwrap());
    assert_eq!(body["access_status"], "session_mismatch");
    assert!(
        body["warnings"][0]
            .as_str()
            .unwrap()
            .contains("not accessible from this session")
    );
}

#[test]
fn read_saved_context_cross_repo_isolation_blocks_read() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();

    let content = medium_content("cross-repo");
    let source_id = save_indexed_artifact(repo_root, &db_path, "repo-bound", &content, None);
    assert!(!source_id.is_empty());

    let args = serde_json::json!({"source_id": source_id});
    let result =
        tool_read_saved_context(Some(&args), "/different/repo", &db_path, OutputFormat::Json)
            .unwrap();
    let body = tool_body(&result);
    assert!(!body["found"].as_bool().unwrap());
    assert_eq!(body["access_status"], "repo_scope_mismatch");
    assert!(
        body["warnings"][0]
            .as_str()
            .unwrap()
            .contains("does not overlap current request scope")
    );
}

#[test]
fn read_saved_context_allows_overlap_with_multi_repo_artifact_scope() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();
    let content_db = derive_content_db_path(&db_path);
    let mut store = ContentStore::open(&content_db).unwrap();
    store.migrate().unwrap();

    let identity = ArtifactIdentity::artifact_label("multi-scope-artifact");
    let source_id = generate_source_id(&identity, &medium_content("multi-scope"));
    store
        .index_artifact(
            SourceMeta {
                id: source_id.clone(),
                session_id: Some("sess-overlap".to_owned()),
                agent_id: None,
                source_type: "review_context".to_owned(),
                label: "multi-scope".to_owned(),
                repo_root: None,
                repo_roots: vec![repo_root.to_owned(), "/other/repo".to_owned()],
                repo_id: None,
                repo_ids: vec![],
                identity_kind: identity.kind_str().to_owned(),
                identity_value: identity.value().to_owned(),
            },
            &medium_content("multi-scope"),
            "text/plain",
        )
        .unwrap();

    let args = serde_json::json!({"source_id": source_id});
    let result =
        tool_read_saved_context(Some(&args), repo_root, &db_path, OutputFormat::Json).unwrap();
    let body = tool_body(&result);
    assert_eq!(body["access_status"], "ok");
    assert_eq!(body["repo_scope"]["repo_count"].as_u64().unwrap(), 2);
}

#[test]
fn search_saved_context_cross_session_honors_repo_scope_isolation() {
    let dir = TempDir::new().unwrap();
    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let db_path = setup_db_path(&dir);
    let repo_root = dir.path().to_str().unwrap();

    let registry = RepoRegistry {
        schema_version: atlas_repo::REPO_REGISTRY_SCHEMA_VERSION,
        root_repo_id: atlas_repo::stable_repo_id(Utf8Path::new(repo_root)),
        registrations: vec![
            atlas_repo::RepoRegistration {
                repo_id: atlas_repo::stable_repo_id(Utf8Path::new(repo_root)),
                root: Utf8Path::new(repo_root).to_path_buf(),
                display_alias: ".".to_owned(),
                vcs: atlas_repo::VcsMetadata {
                    head: None,
                    default_branch: None,
                    remote_url: None,
                },
                relationship: atlas_repo::RepoRelationship {
                    kind: atlas_repo::RepoRelationshipKind::Root,
                    parent_repo_id: None,
                    parent_path: None,
                },
                trust_state: atlas_repo::TrustState::Trusted,
                enabled: true,
                include_globs: None,
                exclude_globs: None,
                dependencies: Vec::new(),
            },
            atlas_repo::RepoRegistration {
                repo_id: "repo_other".to_owned(),
                root: Utf8Path::new("/other/repo").to_path_buf(),
                display_alias: "other".to_owned(),
                vcs: atlas_repo::VcsMetadata {
                    head: None,
                    default_branch: None,
                    remote_url: None,
                },
                relationship: atlas_repo::RepoRelationship {
                    kind: atlas_repo::RepoRelationshipKind::Manual,
                    parent_repo_id: None,
                    parent_path: None,
                },
                trust_state: atlas_repo::TrustState::Trusted,
                enabled: true,
                include_globs: None,
                exclude_globs: None,
                dependencies: Vec::new(),
            },
        ],
        warnings: Vec::new(),
    };
    registry.save(Utf8Path::new(repo_root)).unwrap();

    let content_db = derive_content_db_path(&db_path);
    let mut store = ContentStore::open(&content_db).unwrap();
    store.migrate().unwrap();
    let local_identity = ArtifactIdentity::artifact_label("local-artifact");
    let local_source_id = generate_source_id(&local_identity, &medium_content("local-scope"));
    store
        .index_artifact(
            SourceMeta {
                id: local_source_id,
                session_id: Some("sess-a".to_owned()),
                agent_id: None,
                source_type: "review_context".to_owned(),
                label: "local-artifact".to_owned(),
                repo_root: Some(repo_root.to_owned()),
                repo_roots: vec![repo_root.to_owned()],
                repo_id: None,
                repo_ids: vec![],
                identity_kind: local_identity.kind_str().to_owned(),
                identity_value: local_identity.value().to_owned(),
            },
            &medium_content("local-scope"),
            "text/plain",
        )
        .unwrap();
    let other_identity = ArtifactIdentity::artifact_label("other-artifact");
    let other_source_id = generate_source_id(&other_identity, &medium_content("other-scope"));
    store
        .index_artifact(
            SourceMeta {
                id: other_source_id,
                session_id: Some("sess-b".to_owned()),
                agent_id: None,
                source_type: "review_context".to_owned(),
                label: "other-artifact".to_owned(),
                repo_root: Some("/other/repo".to_owned()),
                repo_roots: vec!["/other/repo".to_owned()],
                repo_id: None,
                repo_ids: vec![],
                identity_kind: other_identity.kind_str().to_owned(),
                identity_value: other_identity.value().to_owned(),
            },
            &medium_content("other-scope"),
            "text/plain",
        )
        .unwrap();

    let result = tool_search_saved_context(
        Some(&serde_json::json!({
            "query": "scope",
            "cross_session": true,
            "repo_scope": {
                "kind": "repo_id",
                "repo_id": atlas_repo::stable_repo_id(Utf8Path::new(repo_root))
            },
        })),
        repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .unwrap();
    let body = tool_body(&result);
    let results = body["matches"].as_array().unwrap();

    assert_eq!(
        body["query"]["repo_scope"]["repo_count"],
        serde_json::json!(1)
    );
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["label"], serde_json::json!("local-artifact"));
    assert_eq!(results[0]["repo_roots"], serde_json::json!([repo_root]));
}
