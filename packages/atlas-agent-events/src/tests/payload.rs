use super::*;

#[test]
fn file_changed_hook_marks_stale_and_drops_inline_content() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");
    let changed_file = format!("{repo}/src/lib.rs");
    let payload = json!({
        "changed_files": [changed_file],
        "content": "secret inline contents",
        "diff": "@@ -1 +1 @@",
        "files": [{
            "path": "src/lib.rs",
            "before": "old body",
            "after": "new body",
            "snippet": "pub fn beta() {}"
        }]
    });

    let persisted = persist_hook_event(
        &repo,
        &graph_db_path,
        "hook",
        "file-changed",
        payload.clone(),
    )
    .unwrap();
    let actions = execute_hook_actions(
        &repo,
        &graph_db_path,
        "hook",
        resolve_hook_policy("file-changed").unwrap(),
        &persisted,
        &payload,
    );
    let persisted_payload = last_hook_payload(&graph_db_path, &repo, "hook");

    assert_eq!(actions["freshness"]["status"], "stale");
    assert_eq!(actions["freshness"]["inline_content_persisted"], false);
    assert!(
        actions["freshness"]["changed_files"]
            .as_array()
            .unwrap()
            .contains(&json!("src/lib.rs"))
    );
    assert_eq!(
        persisted_payload["hook_metadata"]["freshness"]["status"],
        "stale"
    );
    assert_eq!(
        persisted_payload["hook_metadata"]["freshness"]["inline_content_persisted"],
        false
    );
    assert!(persisted_payload["payload"]["content"].is_null());
    assert!(persisted_payload["payload"]["diff"].is_null());
    assert!(persisted_payload["payload"]["files"][0]["before"].is_null());
    assert!(persisted_payload["payload"]["files"][0]["after"].is_null());
    if let Some(source_id) = persisted.source_id {
        assert!(persisted.storage_kind.is_some());
        let mut content_store =
            ContentStore::open(&derive_content_db_path(&graph_db_path)).unwrap();
        content_store.migrate().unwrap();
        assert!(content_store.get_source(&source_id).unwrap().is_some());
    } else {
        assert!(persisted.storage_kind.is_none());
    }
}

#[test]
fn large_user_prompt_payload_routes_to_content_store() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");
    let payload = json!({ "prompt": format!("review {}", "x".repeat(6_000)) });

    let persisted = persist_hook_event(
        &repo,
        &graph_db_path,
        "hook",
        "user-prompt",
        payload.clone(),
    )
    .unwrap();
    let actions = execute_hook_actions(
        &repo,
        &graph_db_path,
        "hook",
        resolve_hook_policy("user-prompt").unwrap(),
        &persisted,
        &payload,
    );
    let persisted_payload = last_hook_payload(&graph_db_path, &repo, "hook");

    assert_eq!(persisted.storage_kind, Some("pointer"));
    assert!(persisted.source_id.is_some());
    assert_eq!(actions["prompt_routing"]["status"], "routed");
    assert_eq!(persisted_payload["payload_storage"]["kind"], "pointer");
}

#[test]
fn large_stop_payload_routes_to_content_store() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");
    let payload = json!({ "summary": "x".repeat(6_000) });

    let persisted = persist_hook_event(&repo, &graph_db_path, "hook", "stop", payload).unwrap();
    let persisted_payload = last_hook_payload(&graph_db_path, &repo, "hook");

    assert_eq!(persisted.storage_kind, Some("pointer"));
    assert!(persisted.source_id.is_some());
    assert!(persisted.snapshot.is_some());
    assert_eq!(persisted_payload["payload_storage"]["kind"], "pointer");
    assert_eq!(persisted_payload["hook_event"], "stop");
}

#[test]
fn pre_and_post_compact_hooks_round_trip_resume_snapshot() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");

    persist_hook_event(
        &repo,
        &graph_db_path,
        "hook",
        "user-prompt",
        json!({ "prompt": "compact this session" }),
    )
    .unwrap();
    let pre_compact =
        persist_hook_event(&repo, &graph_db_path, "hook", "pre-compact", Value::Null).unwrap();
    assert_eq!(pre_compact.snapshot.as_ref().unwrap()["event_count"], 2);

    let post_compact =
        persist_hook_event(&repo, &graph_db_path, "hook", "post-compact", Value::Null).unwrap();
    let post_actions = execute_hook_actions(
        &repo,
        &graph_db_path,
        "hook",
        resolve_hook_policy("post-compact").unwrap(),
        &post_compact,
        &Value::Null,
    );

    assert_eq!(post_actions["lifecycle"]["status"], "verified");
    assert_eq!(post_actions["lifecycle"]["has_resume_snapshot"], true);
}

#[test]
fn post_tool_use_hook_refreshes_graph_for_changed_files() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path();
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"hook-refresh\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(repo.join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
    std::fs::create_dir_all(repo.join(".atlas")).unwrap();
    git(repo, &["init", "--quiet"]);
    git(repo, &["add", "Cargo.toml", "src/lib.rs"]);

    let repo_str = repo.to_string_lossy().into_owned();
    let graph_db_path = format!("{repo_str}/.atlas/worldtree.db");
    Store::open(&graph_db_path).unwrap();
    let source_repo_id = stable_repo_id(Utf8Path::new(&repo_str));
    build_graph(
        Utf8Path::new(&repo_str),
        &graph_db_path,
        &BuildOptions {
            source_repo_id: Some(source_repo_id.clone()),
            ..BuildOptions::default()
        },
    )
    .unwrap();

    std::fs::write(
        repo.join("src/lib.rs"),
        "pub fn alpha() {}\npub fn beta() {}\n",
    )
    .unwrap();

    let payload = json!({
        "tool_name": "Write",
        "changed_files": [repo.join("src/lib.rs").to_string_lossy().into_owned()],
    });
    let persisted = persist_hook_event(
        &repo_str,
        &graph_db_path,
        "hook",
        "post-tool-use",
        payload.clone(),
    )
    .unwrap();
    let actions = execute_hook_actions(
        &repo_str,
        &graph_db_path,
        "hook",
        resolve_hook_policy("post-tool-use").unwrap(),
        &persisted,
        &payload,
    );

    assert_eq!(actions["graph_refresh"]["status"], "updated");
    let store = Store::open(&graph_db_path).unwrap();
    let nodes = store.nodes_by_file("src/lib.rs").unwrap();
    assert!(
        nodes
            .iter()
            .any(|node| node.qualified_name.ends_with("::fn::beta"))
    );
    let repo_hashes = store.file_hashes_for_repo(&source_repo_id).unwrap();
    assert!(
        repo_hashes.contains_key("Cargo.toml") && repo_hashes.contains_key("src/lib.rs"),
        "hook refresh must update same repo-scoped file inventory as build"
    );
    assert!(
        store.file_hashes_for_repo("legacy").unwrap().is_empty(),
        "hook refresh must not create legacy file rows"
    );
}
