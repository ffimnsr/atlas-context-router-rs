//! ICM-E2 / ICM-H1 / ICM-H2 — CLI smoke coverage for the memory layer
//! surfaces added by these tracks: opt-in embeddings, hybrid recall, the
//! read-only TUI, and `atlas init --mode`.

use super::memory_fixtures::seed_memory_layer;
use super::*;

fn at(repo: &Path, args: &[&str]) -> Output {
    let mut full = vec!["--json"];
    full.extend_from_slice(args);
    run_atlas(repo, &full)
}

fn write_config(repo: &Path, body: &str) {
    fs::create_dir_all(repo.join(".atlas")).expect("atlas dir");
    write_repo_file(repo, ".atlas/config.toml", body);
}

fn failure_stderr(repo: &Path, args: &[&str]) -> String {
    let output = run_command_capture(repo, env!("CARGO_BIN_EXE_atlas"), args);
    assert!(
        !output.status.success(),
        "expected command to fail: atlas {args:?}"
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn lexical_recall_works_with_no_embedding_provider_configured() {
    let repo = setup_fixture_repo();
    seed_memory_layer(repo.path());

    let recall = read_json_data_output(
        "memory.recall",
        at(
            repo.path(),
            &["memory", "recall", "memory recall policy", "--limit", "5"],
        ),
    );
    assert_eq!(recall["mode"], json!("lexical"));
    let results = recall["results"].as_array().expect("results");
    assert!(!results.is_empty(), "lexical recall must stay the default");
    assert!(
        results
            .iter()
            .all(|hit| hit["ranking"].get("vector_score").is_none()),
        "lexical recall must not report vector scores"
    );
}

#[test]
fn hybrid_recall_without_opt_in_fails_clearly() {
    let repo = setup_fixture_repo();
    seed_memory_layer(repo.path());

    let stderr = failure_stderr(
        repo.path(),
        &["memory", "recall", "memory recall", "--hybrid"],
    );
    assert!(
        stderr.contains("memory.embedding.enabled is false"),
        "unexpected error: {stderr}"
    );
}

#[test]
fn embedding_config_requires_provider_dimension_and_backend_url() {
    let repo = setup_fixture_repo();
    write_config(
        repo.path(),
        "[memory.embedding]\nenabled = true\nmodel = \"nomic-embed-text\"\n",
    );
    let stderr = failure_stderr(repo.path(), &["memory", "recall", "hooks"]);
    assert!(
        stderr.contains("memory.embedding.provider is required"),
        "unexpected error: {stderr}"
    );

    write_config(
        repo.path(),
        "[memory.embedding]\nenabled = true\nprovider = \"search\"\ndimension = 0\n",
    );
    let stderr = failure_stderr(repo.path(), &["memory", "recall", "hooks"]);
    assert!(
        stderr.contains("memory.embedding.dimension"),
        "unexpected error: {stderr}"
    );

    write_config(
        repo.path(),
        "[memory.embedding]\nenabled = true\nprovider = \"search\"\ndimension = 768\n",
    );
    let stderr = failure_stderr(repo.path(), &["memory", "recall", "hooks"]);
    assert!(
        stderr.contains("search.embedding.url"),
        "unexpected error: {stderr}"
    );
}

#[test]
fn memory_embed_dry_run_lists_pending_rows_without_calling_the_backend() {
    let repo = setup_fixture_repo();
    let fixture = seed_memory_layer(repo.path());
    write_config(
        repo.path(),
        "[memory.embedding]\nenabled = true\nprovider = \"search\"\ndimension = 768\n\n[search.embedding]\nurl = \"http://127.0.0.1:9\"\nmodel = \"nomic-embed-text\"\n",
    );

    let embed = read_json_data_output(
        "memory.embed",
        at(repo.path(), &["memory", "embed", "--dry-run", "--limit", "10"]),
    );
    assert_eq!(embed["dry_run"], json!(true));
    assert_eq!(embed["dimension"], json!(768));
    // Three seeded memories, none embedded yet; the private one is still
    // recallable by its own frontend, so coverage counts it.
    assert_eq!(embed["pending_count"], json!(3));
    assert_eq!(embed["coverage"]["embedded"], json!(0));
    assert_eq!(embed["coverage"]["missing"], json!(3));
    let pending = embed["pending"].as_array().expect("pending rows");
    assert!(
        pending
            .iter()
            .any(|row| row["id"] == json!(fixture.critical_memory_id))
    );
}

#[test]
fn memory_tui_json_is_read_only_and_covers_every_section() {
    let repo = setup_fixture_repo();
    seed_memory_layer(repo.path());

    let tui = read_json_data_output("memory.tui", at(repo.path(), &["memory", "tui"]));
    assert_eq!(tui["mode"], json!("read_only"));
    let sections = tui["sections"].as_array().expect("sections");
    let names = sections
        .iter()
        .map(|section| section["section"].as_str().unwrap_or_default().to_owned())
        .collect::<Vec<_>>();
    for expected in [
        "memories",
        "topics",
        "feedback",
        "memoir",
        "overview",
        "health",
        "artifacts",
    ] {
        assert!(names.contains(&expected.to_owned()), "missing {expected}");
    }

    let memories = sections
        .iter()
        .find(|section| section["section"] == json!("memories"))
        .expect("memories section");
    assert!(memories["rows"].as_array().is_some_and(|rows| !rows.is_empty()));
}

#[test]
fn init_mode_dry_run_reports_assets_and_writes_nothing() {
    let repo = setup_fixture_repo();
    let init = read_json_data_output(
        "init",
        at(repo.path(), &["init", "--mode", "all", "--dry-run"]),
    );
    assert_eq!(init["dry_run"], json!(true));
    assert_eq!(init["mode"], json!("all"));
    let files = init["install"].as_array().expect("install plan");
    let paths = files
        .iter()
        .map(|entry| entry["path"].as_str().unwrap_or_default().to_owned())
        .collect::<Vec<_>>();
    assert!(paths.contains(&".atlas/agent/mcp.json".to_owned()));
    assert!(paths.contains(&"AGENTS.md".to_owned()));
    assert!(paths.iter().any(|path| path.starts_with(".git/hooks/")));

    assert!(!repo.path().join(".atlas").join("agent").join("mcp.json").exists());
    assert!(!repo.path().join("AGENTS.md").exists());
}

#[test]
fn init_mode_is_idempotent_across_runs() {
    let repo = setup_fixture_repo();
    let first = read_json_data_output("init", at(repo.path(), &["init", "--mode", "mcp"]));
    let action = |value: &Value| {
        value["install"]
            .as_array()
            .and_then(|files| {
                files
                    .iter()
                    .find(|entry| entry["path"] == json!(".atlas/agent/mcp.json"))
            })
            .and_then(|entry| entry["action"].as_str().map(str::to_owned))
            .unwrap_or_default()
    };
    assert_eq!(action(&first), "create");
    let artifact = repo.path().join(".atlas").join("agent").join("mcp.json");
    assert!(artifact.exists());

    let second = read_json_data_output("init", at(repo.path(), &["init", "--mode", "mcp"]));
    assert_eq!(action(&second), "unchanged");
}

#[test]
fn memory_and_init_surfaces_reject_unknown_modes() {
    let repo = setup_fixture_repo();
    let output = run_command_capture(
        repo.path(),
        env!("CARGO_BIN_EXE_atlas"),
        &["init", "--mode", "dashboard"],
    );
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid value") || stderr.contains("possible values"),
        "unexpected error: {stderr}"
    );
}
