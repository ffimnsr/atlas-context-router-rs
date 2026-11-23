use super::*;

#[test]
fn memory_decay_dry_run_reports_scores_without_writing() {
    let repo = setup_fixture_repo();

    let stored = store_memory(
        repo.path(),
        &["old low fact", "--topic", "hooks", "--importance", "low"],
    );
    let id = memory_id(&stored);
    backdate_memory(repo.path(), &id, "2026-01-01T00:00:00Z");

    // Dry-run reports the score but must not persist it.
    let plan = read_json_data_output(
        "memory.decay",
        run_atlas(
            repo.path(),
            &["--json", "memory", "decay", "--topic", "hooks", "--dry-run"],
        ),
    );
    assert_eq!(plan["dry_run"], json!(true));
    assert_eq!(plan["enabled"], json!(true));
    assert_eq!(plan["count"], json!(1));
    assert_eq!(plan["reports"][0]["memory"]["id"], json!(id));
    assert_eq!(plan["reports"][0]["stale"], json!(true));
    assert_eq!(plan["reports"][0]["updated_decay_score"], json!(1.0));

    let conn = Connection::open(repo.path().join(".atlas").join("session.db")).expect("open db");
    let score: f64 = conn
        .query_row(
            "SELECT decay_score FROM memories WHERE id = ?1",
            [&id],
            |row| row.get(0),
        )
        .expect("score");
    assert_eq!(score, 0.0, "dry-run must not write decay scores");

    // Apply mode persists the score.
    let applied = read_json_data_output(
        "memory.decay",
        run_atlas(
            repo.path(),
            &["--json", "memory", "decay", "--topic", "hooks"],
        ),
    );
    assert_eq!(applied["dry_run"], json!(false));
    let score: f64 = conn
        .query_row(
            "SELECT decay_score FROM memories WHERE id = ?1",
            [&id],
            |row| row.get(0),
        )
        .expect("score");
    assert_eq!(score, 1.0);
}

#[test]
fn memory_decay_protects_critical_rows_and_respects_disabled_config() {
    let repo = setup_fixture_repo();

    let critical = store_memory(
        repo.path(),
        &[
            "old critical decision",
            "--topic",
            "deploy",
            "--importance",
            "critical",
        ],
    );
    backdate_memory(repo.path(), &memory_id(&critical), "2020-01-01T00:00:00Z");

    let decay = read_json_data_output(
        "memory.decay",
        run_atlas(repo.path(), &["--json", "memory", "decay", "--dry-run"]),
    );
    let critical_report = decay["reports"]
        .as_array()
        .expect("reports")
        .iter()
        .find(|report| report["memory"]["id"] == json!(memory_id(&critical)))
        .expect("critical report");
    assert_eq!(critical_report["protected"], json!(true));
    assert_eq!(critical_report["stale"], json!(false));
    assert_eq!(critical_report["retention_days"], json!(null));
    assert_eq!(critical_report["updated_decay_score"], json!(0.0));

    // Config disabling decay makes every curation command a no-op.
    fs::write(
        repo.path().join(".atlas").join("config.toml"),
        "[memory.decay]\nenabled = false\n",
    )
    .expect("write config");
    let disabled = read_json_data_output(
        "memory.decay",
        run_atlas(repo.path(), &["--json", "memory", "decay", "--dry-run"]),
    );
    assert_eq!(disabled["enabled"], json!(false));
    assert_eq!(disabled["count"], json!(0));
}

#[test]
fn memory_stale_lists_only_pruneable_rows() {
    let repo = setup_fixture_repo();

    let low = store_memory(
        repo.path(),
        &["stale low note", "--topic", "hooks", "--importance", "low"],
    );
    backdate_memory(repo.path(), &memory_id(&low), "2026-01-01T00:00:00Z");
    let fresh = store_memory(
        repo.path(),
        &["fresh note", "--topic", "hooks", "--importance", "low"],
    );

    let stale = read_json_data_output(
        "memory.stale",
        run_atlas(
            repo.path(),
            &["--json", "memory", "stale", "--topic", "hooks"],
        ),
    );
    assert_eq!(stale["count"], json!(1));
    assert_eq!(stale["reports"][0]["memory"]["id"], json!(memory_id(&low)));
    assert!(
        stale["reports"]
            .as_array()
            .expect("reports")
            .iter()
            .all(|report| report["memory"]["id"].as_str() != Some(memory_id(&fresh).as_str())),
        "fresh memories must never be stale candidates"
    );
}

#[test]
fn memory_prune_dry_run_then_apply_only_prunes_low_rows() {
    let repo = setup_fixture_repo();

    let low = store_memory(
        repo.path(),
        &[
            "pruneable low note",
            "--topic",
            "hooks",
            "--importance",
            "low",
        ],
    );
    backdate_memory(repo.path(), &memory_id(&low), "2026-01-01T00:00:00Z");
    let fresh = store_memory(
        repo.path(),
        &["keep me", "--topic", "hooks", "--importance", "low"],
    );

    // --importance low --dry-run reports only pruneable low rows.
    let plan = read_json_data_output(
        "memory.prune",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "memory",
                "prune",
                "--importance",
                "low",
                "--dry-run",
            ],
        ),
    );
    assert_eq!(plan["dry_run"], json!(true));
    assert_eq!(plan["candidate_count"], json!(1));
    assert_eq!(plan["candidates"][0]["id"], json!(memory_id(&low)));

    let conn = Connection::open(repo.path().join(".atlas").join("session.db")).expect("open db");
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 2, "dry-run must not delete");

    let applied = read_json_data_output(
        "memory.prune",
        run_atlas(
            repo.path(),
            &["--json", "memory", "prune", "--importance", "low"],
        ),
    );
    assert_eq!(applied["deleted_count"], json!(1));

    let remaining: Vec<String> = conn
        .prepare("SELECT id FROM memories")
        .expect("prepare")
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("rows");
    assert_eq!(remaining, vec![memory_id(&fresh).as_str()]);
}

#[test]
fn memory_prune_requires_allow_critical_override() {
    let repo = setup_fixture_repo();

    let critical = store_memory(
        repo.path(),
        &[
            "critical note",
            "--topic",
            "hooks",
            "--importance",
            "critical",
        ],
    );
    backdate_memory(repo.path(), &memory_id(&critical), "2020-01-01T00:00:00Z");

    // Filtering by critical without the override fails validation.
    let blocked = run_atlas_capture(
        repo.path(),
        &[
            "--json",
            "memory",
            "prune",
            "--importance",
            "critical",
            "--dry-run",
        ],
    );
    assert!(!blocked.status.success());
    let stderr = String::from_utf8(blocked.stderr).expect("stderr utf-8");
    assert!(
        stderr.contains("--allow-critical"),
        "override must be required: {stderr}"
    );

    // Without an importance filter, protected critical rows are never candidates.
    let plan = read_json_data_output(
        "memory.prune",
        run_atlas(repo.path(), &["--json", "memory", "prune", "--dry-run"]),
    );
    assert_eq!(plan["protected_count"], json!(1));
    assert_eq!(plan["candidate_count"], json!(0));
}

#[test]
fn memory_health_reports_deterministic_findings() {
    let repo = setup_fixture_repo();

    let duplicate_a = store_memory(repo.path(), &["deploy uses helm", "--topic", "deploy"]);
    let duplicate_b = store_memory(repo.path(), &["deploy uses helm", "--topic", "deploy"]);
    let orphan = store_memory(
        repo.path(),
        &[
            "references removed artifact",
            "--topic",
            "deploy",
            "--source-id",
            "artifact-gone",
        ],
    );
    let stale = store_memory(
        repo.path(),
        &["stale low note", "--topic", "hooks", "--importance", "low"],
    );
    backdate_memory(repo.path(), &memory_id(&stale), "2026-01-01T00:00:00Z");

    let health = read_json_data_output(
        "memory.health",
        run_atlas(repo.path(), &["--json", "memory", "health"]),
    );
    assert_eq!(health["total_memories"], json!(4));
    let findings = health["findings"].as_array().expect("findings");
    let kinds = findings
        .iter()
        .map(|finding| {
            (
                finding["kind"].as_str().expect("kind"),
                finding["memory_id"].as_str().unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>();
    assert!(kinds.contains(&("stale_memory", memory_id(&stale).as_str())));
    // With identical timestamps the duplicate id tiebreak is nondeterministic
    // across runs, so exactly one of the two same-body rows must be flagged.
    let duplicated = kinds
        .iter()
        .filter(|(kind, _)| *kind == "duplicated_memory")
        .map(|(_, id)| *id)
        .collect::<Vec<_>>();
    assert_eq!(duplicated.len(), 1);
    assert!(
        duplicated[0] == memory_id(&duplicate_a).as_str()
            || duplicated[0] == memory_id(&duplicate_b).as_str(),
        "one same-body duplicate must be flagged"
    );
    assert!(kinds.contains(&("orphaned_source", memory_id(&orphan).as_str())));
    assert!(
        findings.iter().any(|finding| {
            finding["command"]
                .as_str()
                .is_some_and(|command| command.starts_with("atlas memory "))
        }),
        "findings must carry actionable follow-up commands"
    );
    assert!(health["by_category"]["duplicated"].as_u64().unwrap_or(0) >= 1);
    assert!(health["by_category"]["orphaned"].as_u64().unwrap_or(0) >= 1);
}
