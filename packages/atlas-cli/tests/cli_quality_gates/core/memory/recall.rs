use super::*;

#[test]
fn memory_recall_ranks_exact_topic_first_and_enforces_visibility() {
    let repo = setup_fixture_repo();

    store_memory(
        repo.path(),
        &[
            "deploy pipeline runs weekly",
            "--topic",
            "hooks",
            "--importance",
            "critical",
        ],
    );
    let exact = store_memory(
        repo.path(),
        &[
            "deploy notes",
            "--topic",
            "deploy",
            "--importance",
            "critical",
        ],
    );
    let frontend_private = store_memory(
        repo.path(),
        &[
            "deploy secrets",
            "--topic",
            "deploy",
            "--scope",
            "frontend",
            "--frontend",
            "codex",
        ],
    );
    let session_scoped = store_memory(
        repo.path(),
        &[
            "deploy session note",
            "--topic",
            "deploy",
            "--scope",
            "session",
        ],
    );

    let recall = read_json_data_output(
        "memory.recall",
        run_atlas(repo.path(), &["--json", "memory", "recall", "deploy"]),
    );
    assert_eq!(recall["query"], json!("deploy"));
    let results = recall["results"].as_array().expect("recall results");
    let result_ids = results
        .iter()
        .map(|hit| hit["memory"]["id"].as_str().expect("id"))
        .collect::<Vec<_>>();

    // The cli viewer sees its own session memory but NOT codex-frontend ones.
    assert_eq!(results.len(), 3);
    assert!(!result_ids.contains(&memory_id(&frontend_private).as_str()));
    assert!(result_ids.contains(&memory_id(&session_scoped).as_str()));

    // Exact topic matches rank above broad body matches.
    assert_eq!(results[0]["memory"]["id"], json!(memory_id(&exact)));
    assert_eq!(results[0]["relevance_score"], json!(0));
    assert_eq!(results[0]["ranking"]["topic_match"], json!("exact_topic"));
    assert!(results[0]["recall_score"].as_f64().unwrap() > 0.0);
    assert_eq!(
        results[1]["memory"]["id"],
        json!(memory_id(&session_scoped))
    );
    assert_eq!(results[2]["relevance_score"], json!(2));

    // --shared excludes session and frontend scoped memories entirely.
    let shared = read_json_data_output(
        "memory.recall",
        run_atlas(
            repo.path(),
            &["--json", "memory", "recall", "deploy", "--shared"],
        ),
    );
    let shared_ids = shared["results"]
        .as_array()
        .expect("shared results")
        .iter()
        .map(|hit| hit["memory"]["id"].as_str().expect("id"))
        .collect::<Vec<_>>();
    assert!(!shared_ids.contains(&memory_id(&frontend_private).as_str()));
    assert!(!shared_ids.contains(&memory_id(&session_scoped).as_str()));
    assert_eq!(shared_ids.len(), 2);

    // --scope frontend from the cli viewer shows only cli-frontend memories.
    let before = read_json_data_output(
        "memory.recall",
        run_atlas(
            repo.path(),
            &[
                "--json", "memory", "recall", "deploy", "--scope", "frontend",
            ],
        ),
    );
    assert_eq!(before["count"], json!(0));

    let cli_private = store_memory(
        repo.path(),
        &[
            "cli deploy note",
            "--topic",
            "deploy",
            "--scope",
            "frontend",
            "--frontend",
            "cli",
        ],
    );
    let after = read_json_data_output(
        "memory.recall",
        run_atlas(
            repo.path(),
            &[
                "--json", "memory", "recall", "deploy", "--scope", "frontend",
            ],
        ),
    );
    let frontend_ids = after["results"]
        .as_array()
        .expect("frontend results")
        .iter()
        .map(|hit| hit["memory"]["id"].as_str().expect("id"))
        .collect::<Vec<_>>();
    assert_eq!(frontend_ids, vec![memory_id(&cli_private).as_str()]);
    assert!(!frontend_ids.contains(&memory_id(&frontend_private).as_str()));
}

#[test]
fn memory_frontend_identities_are_normalized_and_unknown_ones_rejected() {
    let repo = setup_fixture_repo();

    // Known aliases normalize to canonical identities.
    let stored = store_memory(
        repo.path(),
        &[
            "claude note",
            "--scope",
            "frontend",
            "--frontend",
            "Claude Code",
        ],
    );
    assert_eq!(stored["memory"]["frontend"], json!("claude"));

    // Unknown frontends fail with a clear validation error.
    let unknown = run_atlas_capture(
        repo.path(),
        &[
            "memory",
            "store",
            "zed note",
            "--scope",
            "frontend",
            "--frontend",
            "zed",
        ],
    );
    assert!(!unknown.status.success());
    let stderr = String::from_utf8(unknown.stderr).expect("stderr utf-8");
    assert!(stderr.contains("unknown frontend: zed"), "got: {stderr}");
    assert!(
        stderr.contains("codex"),
        "error must list known frontends: {stderr}"
    );

    // Config explicitly allowing custom frontends accepts them verbatim.
    fs::write(
        repo.path().join(".atlas").join("config.toml"),
        "[memory]\nallow_custom_frontends = true\n",
    )
    .expect("write config");
    let custom = store_memory(
        repo.path(),
        &["zed note", "--scope", "frontend", "--frontend", "Zed Agent"],
    );
    assert_eq!(custom["memory"]["frontend"], json!("zed agent"));

    // The cli viewer never sees claude- or zed-frontend memories.
    let recall = read_json_data_output(
        "memory.recall",
        run_atlas(
            repo.path(),
            &["--json", "memory", "recall", "note", "--scope", "frontend"],
        ),
    );
    assert_eq!(recall["count"], json!(0));
}

#[test]
fn memory_list_filters_and_sorts_by_updated_at_desc() {
    let repo = setup_fixture_repo();

    let critical = store_memory(
        repo.path(),
        &[
            "critical hooks note",
            "--topic",
            "hooks",
            "--importance",
            "critical",
        ],
    );
    let low = store_memory(
        repo.path(),
        &["low hooks note", "--topic", "hooks", "--importance", "low"],
    );
    let global = store_memory(
        repo.path(),
        &["global note", "--topic", "other", "--scope", "global"],
    );

    // Backdate rows so updated_at ordering is observable.
    rewrite_memory(repo.path(), &memory_id(&critical), |record| {
        record.updated_at = "2026-01-01T00:00:00Z".to_owned();
    });

    let all = read_json_data_output(
        "memory.list",
        run_atlas(repo.path(), &["--json", "memory", "list"]),
    );
    assert_eq!(all["count"], json!(3));
    let ids = all["memories"]
        .as_array()
        .expect("memories array")
        .iter()
        .map(|memory| memory["id"].as_str().expect("id"))
        .collect::<Vec<_>>();
    // newest first; the backdated critical memory must sort last.
    assert_eq!(ids[2], memory_id(&critical).as_str());
    assert!(ids.contains(&memory_id(&low).as_str()));
    assert!(ids.contains(&memory_id(&global).as_str()));

    let critical_only = read_json_data_output(
        "memory.list",
        run_atlas(
            repo.path(),
            &["--json", "memory", "list", "--importance", "critical"],
        ),
    );
    assert_eq!(critical_only["count"], json!(1));
    assert_eq!(
        critical_only["memories"][0]["id"],
        json!(memory_id(&critical))
    );

    let old_only = read_json_data_output(
        "memory.list",
        run_atlas(
            repo.path(),
            &["--json", "memory", "list", "--older-than", "2026-02-01"],
        ),
    );
    assert_eq!(old_only["count"], json!(1));
    assert_eq!(old_only["memories"][0]["id"], json!(memory_id(&critical)));

    let invalid_date = run_atlas_capture(
        repo.path(),
        &["--json", "memory", "list", "--older-than", "not-a-date"],
    );
    assert!(!invalid_date.status.success());
    let stderr = String::from_utf8(invalid_date.stderr).expect("stderr utf-8");
    assert!(stderr.contains("expected YYYY-MM-DD"), "got: {stderr}");
}

#[test]
fn memory_delete_requires_exact_id_and_respects_dry_run() {
    let repo = setup_fixture_repo();

    let stored = store_memory(repo.path(), &["delete me", "--topic", "hooks"]);
    let id = memory_id(&stored);

    let dry = read_json_data_output(
        "memory.delete",
        run_atlas(
            repo.path(),
            &["--json", "memory", "delete", &id, "--dry-run"],
        ),
    );
    assert_eq!(dry["memory_id"], json!(id));
    assert_eq!(dry["deleted"], json!(false));
    assert_eq!(dry["dry_run"], json!(true));

    let conn = Connection::open(repo.path().join(".atlas").join("session.db")).expect("open db");
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 1, "dry-run must not mutate storage");

    let removed = read_json_data_output(
        "memory.delete",
        run_atlas(repo.path(), &["--json", "memory", "delete", &id]),
    );
    assert_eq!(removed["deleted"], json!(true));

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 0);

    // Missing ids fail with a clear error (exact id required).
    let missing = run_atlas_capture(
        repo.path(),
        &["--json", "memory", "delete", "does-not-exist"],
    );
    assert!(!missing.status.success());
    let stderr = String::from_utf8(missing.stderr).expect("stderr utf-8");
    assert!(
        stderr.contains("no memory with id does-not-exist"),
        "got: {stderr}"
    );
}

// ── ICM-B — decay, stale, prune, health, consolidate ──────────────────────────
