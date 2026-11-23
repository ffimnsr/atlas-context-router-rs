use super::*;

#[test]
fn post_tool_use_build_test_flow_persists_review_refresh_artifacts() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path();
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"hook-review-refresh\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(repo.join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
    std::fs::create_dir_all(repo.join(".atlas")).unwrap();
    git(repo, &["init", "--quiet"]);
    git(repo, &["add", "Cargo.toml", "src/lib.rs"]);

    let repo_str = repo.to_string_lossy().into_owned();
    let graph_db_path = format!("{repo_str}/.atlas/worldtree.db");
    Store::open(&graph_db_path).unwrap();
    build_graph(
        Utf8Path::new(&repo_str),
        &graph_db_path,
        &BuildOptions::default(),
    )
    .unwrap();

    std::fs::write(
        repo.join("src/lib.rs"),
        "pub fn alpha() {}\npub fn beta() {}\n",
    )
    .unwrap();

    let payload = json!({
        "tool_name": "Bash",
        "status": "ok",
        "command": "cargo test",
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
    assert_eq!(
        actions["review_refresh"]["status"], "refreshed",
        "review_refresh={}",
        actions["review_refresh"]
    );
    assert_eq!(actions["review_refresh"]["trigger"], "test");
    assert!(
        actions["review_refresh"]["changed_files"]
            .as_array()
            .unwrap()
            .contains(&json!("src/lib.rs"))
    );

    let artifacts = actions["review_refresh"]["artifacts"].as_array().unwrap();
    assert_eq!(artifacts.len(), 3);

    let mut content_store = ContentStore::open(&derive_content_db_path(&graph_db_path)).unwrap();
    content_store.migrate().unwrap();
    for artifact in artifacts {
        let source_id = artifact["source_id"].as_str().unwrap();
        let source = content_store.get_source(source_id).unwrap().unwrap();
        assert!(matches!(
            source.source_type.as_str(),
            "review_context" | "explain_change" | "impact_result"
        ));
    }
}

#[test]
fn tool_may_change_files_recognizes_vscode_tool_names() {
    // VS Code file-editing tools
    assert!(tool_may_change_files("replace_string_in_file"));
    assert!(tool_may_change_files("multi_replace_string_in_file"));
    assert!(tool_may_change_files("create_file"));
    assert!(tool_may_change_files("edit_notebook_file"));
    assert!(tool_may_change_files("create_directory"));
    // VS Code terminal tool — can run formatters, git, cargo fmt, etc.
    assert!(tool_may_change_files("run_in_terminal"));
    // VS Code camelCase built-in variants
    assert!(tool_may_change_files("editFiles"));
    assert!(tool_may_change_files("runInTerminal"));
    // Claude Code names still work
    assert!(tool_may_change_files("edit"));
    assert!(tool_may_change_files("write"));
    assert!(tool_may_change_files("bash"));
    assert!(tool_may_change_files("multiedit"));
    assert!(tool_may_change_files("patch"));
    // Read-only tools must not trigger refresh
    assert!(!tool_may_change_files("read_file"));
    assert!(!tool_may_change_files("grep_search"));
    assert!(!tool_may_change_files("file_search"));
    assert!(!tool_may_change_files("semantic_search"));
    assert!(!tool_may_change_files("get_errors"));
    assert!(!tool_may_change_files("list_dir"));
}

#[test]
fn post_tool_use_run_in_terminal_triggers_graph_refresh() {
    // run_in_terminal has no file paths in its payload, but it can invoke formatters,
    // git, cargo fmt, etc. Atlas must trigger a WorkingTree scan rather than skipping.
    let dir = TempDir::new().unwrap();
    let repo = dir.path();
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"hook-terminal\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(repo.join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
    std::fs::create_dir_all(repo.join(".atlas")).unwrap();
    git(repo, &["init", "--quiet"]);
    git(repo, &["add", "Cargo.toml", "src/lib.rs"]);

    let repo_str = repo.to_string_lossy().into_owned();
    let graph_db_path = format!("{repo_str}/.atlas/worldtree.db");
    Store::open(&graph_db_path).unwrap();
    atlas_engine::build_graph(
        Utf8Path::new(&repo_str),
        &graph_db_path,
        &atlas_engine::BuildOptions::default(),
    )
    .unwrap();

    // Simulate run_in_terminal payload: no file paths, just a command string.
    let payload = json!({
        "tool_name": "run_in_terminal",
        "tool_input": {
            "command": "cargo fmt",
            "explanation": "Format code",
            "goal": "Format",
            "mode": "sync"
        },
        "tool_response": "formatted"
    });
    let persisted = persist_hook_event(
        &repo_str,
        &graph_db_path,
        "copilot",
        "post-tool-use",
        payload.clone(),
    )
    .unwrap();
    let actions = execute_hook_actions(
        &repo_str,
        &graph_db_path,
        "copilot",
        resolve_hook_policy("post-tool-use").unwrap(),
        &persisted,
        &payload,
    );

    // Must attempt a WorkingTree refresh, not be skipped as "tool_not_graph_relevant".
    assert_ne!(
        actions["graph_refresh"]["status"], "skipped",
        "run_in_terminal must not be skipped; got: {}",
        actions["graph_refresh"]
    );
    assert_eq!(
        actions["graph_refresh"]["status"], "updated",
        "graph_refresh={}",
        actions["graph_refresh"]
    );
}

#[test]
fn post_tool_use_replace_string_in_file_targets_changed_file() {
    // replace_string_in_file carries filePath inside tool_input, so atlas should pick
    // up the specific changed file and run a targeted (Files) refresh.
    let dir = TempDir::new().unwrap();
    let repo = dir.path();
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"hook-replace\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(repo.join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
    std::fs::create_dir_all(repo.join(".atlas")).unwrap();
    git(repo, &["init", "--quiet"]);
    git(repo, &["add", "Cargo.toml", "src/lib.rs"]);

    let repo_str = repo.to_string_lossy().into_owned();
    let graph_db_path = format!("{repo_str}/.atlas/worldtree.db");
    Store::open(&graph_db_path).unwrap();
    atlas_engine::build_graph(
        Utf8Path::new(&repo_str),
        &graph_db_path,
        &atlas_engine::BuildOptions::default(),
    )
    .unwrap();

    // Simulate the file being edited, then the VS Code PostToolUse payload.
    std::fs::write(
        repo.join("src/lib.rs"),
        "pub fn alpha() {}\npub fn gamma() {}\n",
    )
    .unwrap();

    let abs_path = repo.join("src/lib.rs").to_string_lossy().into_owned();
    let payload = json!({
        "tool_name": "replace_string_in_file",
        "tool_input": {
            "filePath": abs_path,
            "oldString": "pub fn alpha() {}\n",
            "newString": "pub fn alpha() {}\npub fn gamma() {}\n"
        },
        "tool_response": "replaced"
    });
    let persisted = persist_hook_event(
        &repo_str,
        &graph_db_path,
        "copilot",
        "post-tool-use",
        payload.clone(),
    )
    .unwrap();
    let actions = execute_hook_actions(
        &repo_str,
        &graph_db_path,
        "copilot",
        resolve_hook_policy("post-tool-use").unwrap(),
        &persisted,
        &payload,
    );

    assert_eq!(
        actions["graph_refresh"]["status"], "updated",
        "graph_refresh={}",
        actions["graph_refresh"]
    );
    // Targeted refresh: changed_files must list src/lib.rs.
    assert!(
        actions["graph_refresh"]["changed_files"]
            .as_array()
            .unwrap()
            .contains(&json!("src/lib.rs")),
        "expected src/lib.rs in changed_files, got: {}",
        actions["graph_refresh"]["changed_files"]
    );
    let store = Store::open(&graph_db_path).unwrap();
    let nodes = store.nodes_by_file("src/lib.rs").unwrap();
    assert!(
        nodes
            .iter()
            .any(|node| node.qualified_name.ends_with("::fn::gamma")),
        "gamma should be indexed after replace_string_in_file refresh"
    );
}

#[test]
fn hook_metadata_classifies_runtime_output_and_dedupes_repeated_input() {
    use crate::metadata::build_hook_event_metadata;
    use crate::policy::{HookMetadataContext, HookPayloadRouting};
    use atlas_session::SessionStoreConfig;

    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = repo.clone();
    let session_db = derive_session_db_path(&graph_db_path);
    if let Some(parent) = Path::new(&session_db).parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut store = SessionStore::open_with_config(
        &session_db,
        SessionStoreConfig {
            dedup_window_secs: 60,
            ..SessionStoreConfig::default()
        },
    )
    .unwrap();
    let session_id = SessionId::derive(&repo, "", "hook");
    store
        .upsert_session_meta(session_id.clone(), &repo, "hook", None)
        .unwrap();

    let payload = json!({
        "command": "cargo test",
        "status": "failed",
        "stderr": "test result: FAILED. 1 passed; 2 failed with token secret-123"
    });
    let policy = resolve_hook_policy("tool-failure").unwrap();
    let routed = HookPayloadRouting {
        event_payload: payload.clone(),
        source_id: None,
        storage_kind: None,
    };
    let metadata = build_hook_event_metadata(HookMetadataContext {
        repo: &repo,
        graph_db_path: &graph_db_path,
        store: &store,
        session_id: &session_id,
        policy,
        payload: &payload,
        routed: &routed,
        pending_resume: false,
        event_source: "hook",
        agent_id: None,
    });
    let classification = metadata.get("classification").expect("classification");
    assert_eq!(classification["kind"], "test_failure");
    assert_eq!(classification["severity"], "error");
    assert_eq!(classification["rule_id"], "text_test_failure");
    assert_eq!(classification["matched_fields"], json!(["stderr"]));
    let encoded = metadata.to_string();
    assert!(!encoded.contains("secret-123"));
    assert!(!encoded.contains("FAILED"));

    let make_parts = |metadata: Value| HookEventParts {
        frontend: "hook",
        event: "tool-failure",
        payload: payload.clone(),
        hook_metadata: metadata,
        source_id: None,
        storage_kind: None,
        pending_resume: false,
        event_source: "hook",
        agent_id: None,
    };
    let first = build_hook_event(&session_id, make_parts(metadata.clone()));
    let second = build_hook_event(&session_id, make_parts(metadata));
    assert!(store.append_event(first).unwrap().is_some());
    assert!(
        store.append_event(second).unwrap().is_none(),
        "identical classified payload must dedupe inside dedup_window_secs"
    );
}
