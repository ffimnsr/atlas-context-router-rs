use super::*;

#[test]
fn recall_memories_enforces_session_and_frontend_visibility() {
    let (_dir, store) = open_store(16, 1024);
    seed_memory(
        &store,
        "p",
        "2026-01-01T00:00:00Z",
        "project body",
        "hooks",
        MemoryImportance::Normal,
        MemoryScope::Project,
    );
    seed_memory(
        &store,
        "g",
        "2026-01-01T00:00:00Z",
        "global body",
        "hooks",
        MemoryImportance::Normal,
        MemoryScope::Global,
    );
    seed_memory(
        &store,
        "s",
        "2026-01-01T00:00:00Z",
        "session body",
        "hooks",
        MemoryImportance::Normal,
        MemoryScope::Session,
    );
    seed_memory(
        &store,
        "f",
        "2026-01-01T00:00:00Z",
        "frontend body",
        "hooks",
        MemoryImportance::Normal,
        MemoryScope::Frontend,
    );

    let recall_ids = |viewer: &MemoryViewer| -> Vec<String> {
        let mut ids = store
            .recall_memories(
                "/repo",
                "body",
                &MemoryListFilter::default(),
                false,
                viewer,
                10,
            )
            .unwrap()
            .iter()
            .map(|hit| hit.memory.id.clone())
            .collect::<Vec<_>>();
        ids.sort();
        ids
    };

    // Same session + same frontend: everything is visible.
    assert_eq!(
        recall_ids(&MemoryViewer {
            frontend: "codex".to_owned(),
            session_id: "s1".to_owned(),
        }),
        vec!["f", "g", "p", "s"]
    );

    // Same session, different frontend: frontend-scoped memory hidden.
    assert_eq!(
        recall_ids(&cli_viewer()),
        vec!["g", "p", "s"],
        "session visible to same session; frontend hidden from other frontends"
    );

    // Same frontend, different session: session-scoped memory hidden.
    assert_eq!(
        recall_ids(&MemoryViewer {
            frontend: "codex".to_owned(),
            session_id: "other".to_owned(),
        }),
        vec!["f", "g", "p"],
        "frontend visible to same frontend; session hidden from other sessions"
    );

    // Different session AND different frontend: only project + global remain.
    assert_eq!(
        recall_ids(&MemoryViewer {
            frontend: "claude".to_owned(),
            session_id: "other".to_owned(),
        }),
        vec!["g", "p"],
        "unrelated viewers see only project and global memories"
    );
}

#[test]
fn recall_memories_shared_excludes_own_session_and_frontend_memories() {
    let (_dir, store) = open_store(16, 1024);
    for (id, scope) in [
        ("p", MemoryScope::Project),
        ("s", MemoryScope::Session),
        ("f", MemoryScope::Frontend),
    ] {
        seed_memory(
            &store,
            id,
            "2026-01-01T00:00:00Z",
            "hooks body",
            "hooks",
            MemoryImportance::Normal,
            scope,
        );
    }

    // Even the owner's own session/frontend memories are excluded by --shared.
    let hits = store
        .recall_memories(
            "/repo",
            "hooks",
            &MemoryListFilter::default(),
            true,
            &cli_viewer(),
            10,
        )
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].memory.id, "p");
}
