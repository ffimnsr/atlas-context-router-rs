use super::*;

#[test]
fn memory_consolidate_dry_run_then_apply_supersedes_duplicates() {
    let repo = setup_fixture_repo();

    let first = store_memory(
        repo.path(),
        &[
            "helm deploy steps",
            "--topic",
            "deploy",
            "--title",
            "Deploy runbook",
        ],
    );
    let second = store_memory(
        repo.path(),
        &[
            "helm rollback steps",
            "--topic",
            "deploy",
            "--title",
            "Deploy runbook",
        ],
    );

    let plan = read_json_data_output(
        "memory.consolidate",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "memory",
                "consolidate",
                "--topic",
                "deploy",
                "--dry-run",
            ],
        ),
    );
    assert_eq!(plan["dry_run"], json!(true));
    assert_eq!(plan["group_count"], json!(1));
    assert_eq!(plan["merged_count"], json!(1));
    // With identical timestamps the kept row is the lexicographically smallest
    // id, so the test must not assume store order.
    let kept_id = plan["groups"][0]["kept_memory_id"]
        .as_str()
        .expect("kept id")
        .to_owned();
    let merged_id = plan["groups"][0]["merged_memory_ids"][0]
        .as_str()
        .expect("merged id")
        .to_owned();
    assert!(
        kept_id == memory_id(&first) || kept_id == memory_id(&second),
        "kept row must be one of the stored memories"
    );
    assert!(
        merged_id == memory_id(&first) || merged_id == memory_id(&second),
        "merged row must be the other stored memory"
    );
    assert_ne!(kept_id, merged_id);
    assert_eq!(plan["groups"][0]["consolidated_id"], json!(null));

    // Dry-run is read-only: both rows still active.
    let conn = Connection::open(repo.path().join(".atlas").join("session.db")).expect("open db");
    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE superseded_by IS NULL",
            [],
            |row| row.get(0),
        )
        .expect("active count");
    assert_eq!(active, 2);

    // Apply: consolidated row created, merged row superseded.
    let applied = read_json_data_output(
        "memory.consolidate",
        run_atlas(
            repo.path(),
            &["--json", "memory", "consolidate", "--topic", "deploy"],
        ),
    );
    let consolidated_id = applied["groups"][0]["consolidated_id"]
        .as_str()
        .expect("consolidated id")
        .to_owned();
    assert_eq!(applied["group_count"], json!(1));
    assert_eq!(applied["kept_count"], json!(1));

    let merged = {
        let session_db = repo.path().join(".atlas").join("session.db");
        let store = atlas_session::SessionStore::open(session_db.to_str().expect("db path"))
            .expect("open session store");
        let repo_root = memory_repo_root(repo.path());
        store
            .get_memory(&repo_root, &merged_id)
            .expect("read merged")
            .expect("merged memory exists")
    };
    assert_eq!(merged.superseded_by.as_deref(), Some(consolidated_id.as_str()));
    let consolidated = {
        let session_db = repo.path().join(".atlas").join("session.db");
        let store = atlas_session::SessionStore::open(session_db.to_str().expect("db path"))
            .expect("open session store");
        let repo_root = memory_repo_root(repo.path());
        store
            .get_memory(&repo_root, &consolidated_id)
            .expect("read consolidated")
            .expect("consolidated memory exists")
    };
    assert!(
        consolidated
            .links
            .iter()
            .any(|link| link.id == merged_id && link.relation == "supersedes"),
        "consolidated record must link the merged memory"
    );

    // Default list hides the superseded row; the consolidated row remains.
    let listed = read_json_data_output(
        "memory.list",
        run_atlas(
            repo.path(),
            &["--json", "memory", "list", "--topic", "deploy"],
        ),
    );
    assert_eq!(listed["count"], json!(1));
    assert!(
        !listed["memories"]
            .as_array()
            .expect("memories")
            .iter()
            .any(|memory| memory["id"] == json!(merged_id)),
        "superseded rows must be hidden from default listing"
    );
    assert!(
        listed["memories"]
            .as_array()
            .expect("memories")
            .iter()
            .any(|memory| memory["id"] == json!(consolidated_id)),
        "the consolidated memory stays active"
    );
    assert!(
        listed["memories"]
            .as_array()
            .expect("memories")
            .iter()
            .any(|memory| memory["id"] == json!(consolidated_id)),
        "consolidated row must be listed"
    );

    // Recall hides superseded rows entirely; the consolidated row carries the
    // merged facts.
    let recall = read_json_data_output(
        "memory.recall",
        run_atlas(repo.path(), &["--json", "memory", "recall", "helm"]),
    );
    let recalled = recall["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|hit| hit["memory"]["id"].as_str().unwrap_or_default().to_owned())
        .collect::<Vec<_>>();
    assert!(
        recalled.contains(&consolidated_id),
        "consolidated row in recall"
    );
    assert!(
        !recalled.contains(&merged_id),
        "superseded rows must not be surfaced by recall"
    );
}

#[test]
fn memory_store_applies_eua_patch_text() {
    let repo = setup_fixture_repo();
    let stored = store_memory(repo.path(), &["deploy uses helm", "--topic", "deploy"]);
    let id = memory_id(&stored);

    let patched = read_json_data_output(
        "memory.store",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "memory",
                "store",
                "--patch-id",
                &id,
                "--patch-text",
                "<<<< SEARCH\nhelm\n====\nhelm charts\n>>>> REPLACE",
            ],
        ),
    );
    assert_eq!(patched["patched"], json!(true));
    assert_eq!(patched["patch_count"], json!(1));
    assert_eq!(
        patched["memory"]["body"],
        json!("deploy uses helm charts")
    );

    let error = run_atlas_capture(
        repo.path(),
        &["memory", "store", "--patch-id", &id],
    );
    assert!(!error.status.success());
    let stderr = String::from_utf8(error.stderr).expect("stderr utf-8");
    assert!(stderr.contains("patch-text"), "got: {stderr}");
}

#[test]
fn memory_link_and_tag_filters_work_end_to_end() {
    let repo = setup_fixture_repo();
    let source = store_memory(
        repo.path(),
        &[
            "storage uses markdown",
            "--topic",
            "storage",
            "--tag",
            "architecture",
        ],
    );
    let target = store_memory(
        repo.path(),
        &[
            "sqlite is derived",
            "--topic",
            "storage",
            "--tag",
            "sqlite",
        ],
    );
    let source_id = memory_id(&source);
    let target_id = memory_id(&target);

    let linked = read_json_data_output(
        "memory.link",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "memory",
                "link",
                &source_id,
                &target_id,
                "--relation",
                "refines",
            ],
        ),
    );
    assert_eq!(linked["relation"], json!("refines"));
    assert!(linked["links"]
        .as_array()
        .expect("links")
        .iter()
        .any(|link| link["id"] == json!(target_id)));

    let tagged = read_json_data_output(
        "memory.recall",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "memory",
                "recall",
                "storage",
                "--tag",
                "architecture",
            ],
        ),
    );
    assert_eq!(tagged["count"], json!(1));
    assert_eq!(tagged["results"][0]["memory"]["id"], json!(source_id));

    let none = read_json_data_output(
        "memory.recall",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "memory",
                "recall",
                "storage",
                "--tag",
                "missing",
            ],
        ),
    );
    assert_eq!(none["count"], json!(0));
}
