use super::*;

/// Construct a fully-healthy base input with no pending changes.
fn healthy_input() -> GraphReadinessInput<'static> {
    GraphReadinessInput {
        repo_root: "/repo",
        db_path: "/repo/.atlas/worldtree.db",
        db_exists: true,
        db_open_error: None,
        build_state: Some("built"),
        build_last_error: None,
        graph_error: None,
        recovery_mode: None,
        quarantine_path: None,
        pending_graph_changes: &[],
        indexed_file_count: 42,
        graph_has_content: true,
        last_indexed_at: Some("2024-01-01T00:00:00Z"),
        retrieval_unavailable: false,
    }
}

// ── built / clean / current ─────────────────────────────────────────────

#[test]
fn fresh_graph_is_fully_ready() {
    let r = GraphReadiness::derive(healthy_input());
    assert!(r.graph_built, "graph_built");
    assert!(r.graph_queryable, "graph_queryable");
    assert!(r.graph_current, "graph_current");
    assert!(!r.stale_index, "stale_index");
    assert!(r.pending_graph_changes.is_empty());
    assert_eq!(r.integrity_state, IntegrityState::Clean);
    assert_eq!(r.health_class, Some(GraphStoreHealthClass::Healthy));
    assert_eq!(r.error_code, "none");
    assert!(r.is_ok());
    assert_eq!(r.indexed_file_count, 42);
    assert_eq!(r.last_indexed_at.as_deref(), Some("2024-01-01T00:00:00Z"));
    assert_eq!(r.repo_root, "/repo");
    assert_eq!(r.db_path, "/repo/.atlas/worldtree.db");
}

// ── missing graph db ────────────────────────────────────────────────────

#[test]
fn missing_db_marks_not_built_not_queryable() {
    let r = GraphReadiness::derive(GraphReadinessInput {
        db_exists: false,
        build_state: None,
        graph_has_content: false,
        ..healthy_input()
    });
    assert!(!r.db_exists);
    assert!(!r.graph_built);
    assert!(!r.graph_queryable);
    assert!(!r.graph_current);
    assert_eq!(r.integrity_state, IntegrityState::Clean);
    assert_eq!(r.health_class, None);
    assert_eq!(r.error_code, "missing_graph_db");
    assert!(!r.is_ok());
}

// ── stale index ─────────────────────────────────────────────────────────

#[test]
fn pending_changes_make_graph_stale_but_queryable() {
    let pending = vec!["src/main.rs".to_owned(), "src/lib.rs".to_owned()];
    let r = GraphReadiness::derive(GraphReadinessInput {
        pending_graph_changes: &pending,
        ..healthy_input()
    });
    assert!(r.graph_built);
    assert!(r.graph_queryable, "queryable even when stale");
    assert!(!r.graph_current, "not current when stale");
    assert!(r.stale_index);
    assert_eq!(r.pending_graph_changes.len(), 2);
    assert_eq!(r.health_class, Some(GraphStoreHealthClass::Stale));
    assert_eq!(r.error_code, "stale_index");
    assert!(!r.is_ok());
}

// ── build lifecycle states ───────────────────────────────────────────────

#[test]
fn interrupted_build_state_marks_not_built() {
    let r = GraphReadiness::derive(GraphReadinessInput {
        build_state: Some("building"),
        graph_has_content: false,
        ..healthy_input()
    });
    assert!(!r.graph_built, "in-progress build is not built");
    assert!(!r.graph_queryable);
    assert!(!r.graph_current);
    assert_eq!(
        r.health_class,
        Some(GraphStoreHealthClass::InterruptedBuild)
    );
    assert_eq!(r.error_code, "interrupted_build");
    assert_eq!(r.build_state.as_deref(), Some("building"));
}

#[test]
fn degraded_build_marks_not_built() {
    let r = GraphReadiness::derive(GraphReadinessInput {
        build_state: Some("degraded"),
        graph_has_content: false,
        ..healthy_input()
    });
    assert!(!r.graph_built);
    assert!(!r.graph_queryable);
    assert_eq!(r.health_class, None);
    assert_eq!(r.error_code, "degraded_build");
}

#[test]
fn failed_build_marks_not_built() {
    let last_err = Some("parse error on file foo.rs");
    let r = GraphReadiness::derive(GraphReadinessInput {
        build_state: Some("build_failed"),
        build_last_error: last_err,
        graph_has_content: false,
        ..healthy_input()
    });
    assert!(!r.graph_built);
    assert!(!r.graph_queryable);
    assert_eq!(r.health_class, Some(GraphStoreHealthClass::FailedBuild));
    assert_eq!(r.error_code, "failed_build");
    assert_eq!(r.build_last_error.as_deref(), last_err);
}

// ── graph built inferred from content (no build record) ─────────────────

#[test]
fn legacy_graph_with_content_but_no_build_record_is_treated_as_built() {
    let r = GraphReadiness::derive(GraphReadinessInput {
        build_state: None,
        graph_has_content: true,
        ..healthy_input()
    });
    assert!(r.graph_built, "inferred from content");
    assert!(r.graph_queryable);
    assert!(r.graph_current);
    assert_eq!(r.health_class, Some(GraphStoreHealthClass::Healthy));
    assert_eq!(r.error_code, "none");
}

#[test]
fn no_build_record_and_no_content_is_not_built() {
    // db_exists=true but nothing has been indexed yet.  The health layer
    // does not treat an empty-but-present DB as "missing_graph_db"; the
    // error_code is "none" but graph_built=false so is_ok() is false.
    let r = GraphReadiness::derive(GraphReadinessInput {
        build_state: None,
        graph_has_content: false,
        ..healthy_input()
    });
    assert!(!r.graph_built, "no content means not built");
    assert!(!r.graph_queryable);
    assert!(!r.graph_current);
    // health.rs returns "none" when db_exists=true and no error: the DB
    // file is present but unpopulated, which is distinct from missing.
    assert_eq!(r.health_class, None);
    assert_eq!(r.error_code, "none");
    assert!(!r.is_ok(), "is_ok requires graph_built");
}

// ── integrity states ─────────────────────────────────────────────────────

#[test]
fn schema_mismatch_error_yields_schema_mismatch_integrity() {
    let err = "table nodes has no column named extra_json";
    let r = GraphReadiness::derive(GraphReadinessInput {
        db_open_error: Some(err),
        build_state: None,
        graph_has_content: false,
        ..healthy_input()
    });
    assert_eq!(r.integrity_state, IntegrityState::SchemaMismatch);
    assert_eq!(r.health_class, Some(GraphStoreHealthClass::SchemaMismatch));
    assert!(!r.graph_queryable, "blocked on integrity");
    assert!(!r.graph_built);
    assert_eq!(r.error_code, "schema_mismatch");
    assert_eq!(r.db_open_error.as_deref(), Some(err));
}

#[test]
fn corrupt_db_error_yields_sqlite_corrupt_integrity() {
    let err = "sqlite error: database disk image is malformed";
    let r = GraphReadiness::derive(GraphReadinessInput {
        db_open_error: Some(err),
        build_state: None,
        graph_has_content: false,
        ..healthy_input()
    });
    assert_eq!(r.integrity_state, IntegrityState::SqliteCorrupt);
    assert_eq!(r.health_class, Some(GraphStoreHealthClass::SqliteCorrupt));
    assert!(!r.graph_queryable);
    assert_eq!(r.error_code, "sqlite_corrupt");
}

#[test]
fn graph_query_error_makes_graph_not_queryable() {
    let err = "no such column: nodes.extra_json";
    let r = GraphReadiness::derive(GraphReadinessInput {
        graph_error: Some(err),
        ..healthy_input()
    });
    assert!(r.graph_built, "build record says built");
    assert!(!r.graph_queryable, "blocked by query error");
    assert!(!r.graph_current);
    assert_eq!(r.integrity_state, IntegrityState::SchemaMismatch);
    assert_eq!(r.health_class, Some(GraphStoreHealthClass::SchemaMismatch));
    assert_eq!(r.error_code, "schema_mismatch");
}

#[test]
fn graph_invariant_error_yields_logical_inconsistency_integrity() {
    let err = "dangling edge detected during graph invariant scan";
    let r = GraphReadiness::derive(GraphReadinessInput {
        graph_error: Some(err),
        ..healthy_input()
    });
    assert!(!r.graph_queryable);
    assert_eq!(r.integrity_state, IntegrityState::LogicalInconsistency);
    assert_eq!(
        r.health_class,
        Some(GraphStoreHealthClass::LogicalInconsistency)
    );
    assert_eq!(r.error_code, "logical_inconsistency");
}

// ── retrieval unavailable ────────────────────────────────────────────────

#[test]
fn retrieval_unavailable_surfaces_in_error_code() {
    let r = GraphReadiness::derive(GraphReadinessInput {
        retrieval_unavailable: true,
        ..healthy_input()
    });
    assert!(r.graph_built);
    assert_eq!(r.health_class, Some(GraphStoreHealthClass::Healthy));
    assert!(r.graph_queryable, "graph itself is queryable");
    assert!(r.graph_current);
    assert_eq!(r.error_code, "retrieval_index_unavailable");
    assert!(!r.is_ok());
}

// ── field derivation: error code takes priority ──────────────────────────

#[test]
fn stale_index_takes_priority_over_retrieval_unavailable() {
    let pending = vec!["src/foo.rs".to_owned()];
    let r = GraphReadiness::derive(GraphReadinessInput {
        pending_graph_changes: &pending,
        retrieval_unavailable: true,
        ..healthy_input()
    });
    // stale_index is checked before retrieval_unavailable in health.rs
    assert_eq!(r.error_code, "stale_index");
    assert!(r.stale_index);
}

#[test]
fn missing_db_takes_priority_over_everything() {
    let pending = vec!["src/foo.rs".to_owned()];
    let r = GraphReadiness::derive(GraphReadinessInput {
        db_exists: false,
        build_state: None,
        graph_has_content: false,
        pending_graph_changes: &pending,
        retrieval_unavailable: true,
        ..healthy_input()
    });
    assert_eq!(r.error_code, "missing_graph_db");
}

// ── integrity_state derive_integrity_state unit tests ───────────────────

#[test]
fn no_error_yields_clean_integrity() {
    assert_eq!(derive_integrity_state(None, None), IntegrityState::Clean);
}

#[test]
fn schema_mismatch_detection_from_db_open_error() {
    assert_eq!(
        derive_integrity_state(
            Some("table edges has no column named reachability_checked"),
            None
        ),
        IntegrityState::SchemaMismatch,
    );
}

#[test]
fn schema_mismatch_detection_from_graph_error() {
    assert_eq!(
        derive_integrity_state(None, Some("no such column: nodes.extra_json")),
        IntegrityState::SchemaMismatch,
    );
}

#[test]
fn db_open_error_takes_precedence_over_graph_error() {
    // db_open_error = schema mismatch, graph_error = corrupt  -> schema_mismatch wins
    let open_err = Some("table nodes has no column named foo");
    let graph_err = Some("constraint failed");
    assert_eq!(
        derive_integrity_state(open_err, graph_err),
        IntegrityState::SchemaMismatch,
    );
}

// ── IntegrityState helpers ───────────────────────────────────────────────

#[test]
fn integrity_state_as_str_round_trips() {
    assert_eq!(IntegrityState::Clean.as_str(), "clean");
    assert_eq!(IntegrityState::SchemaMismatch.as_str(), "schema_mismatch");
    assert_eq!(IntegrityState::SqliteCorrupt.as_str(), "sqlite_corrupt");
    assert_eq!(
        IntegrityState::LogicalInconsistency.as_str(),
        "logical_inconsistency"
    );
}

#[test]
fn integrity_state_is_clean_helper() {
    assert!(IntegrityState::Clean.is_clean());
    assert!(!IntegrityState::SqliteCorrupt.is_clean());
    assert!(!IntegrityState::SchemaMismatch.is_clean());
    assert!(!IntegrityState::LogicalInconsistency.is_clean());
}

// ── suggestions and message presence ────────────────────────────────────

#[test]
fn failed_build_includes_suggestions() {
    let r = GraphReadiness::derive(GraphReadinessInput {
        build_state: Some("build_failed"),
        graph_has_content: false,
        ..healthy_input()
    });
    assert!(
        !r.suggestions.is_empty(),
        "should have suggestions for failed build"
    );
    assert!(!r.message.is_empty());
}

#[test]
fn readiness_carries_recovery_metadata_and_rebuild_hint() {
    let r = GraphReadiness::derive(GraphReadinessInput {
        build_state: Some("build_failed"),
        recovery_mode: Some("auto_quarantine_and_rebuild"),
        quarantine_path: Some("/repo/.atlas/worldtree.db.quarantine.20260805T000000Z.0"),
        graph_has_content: false,
        ..healthy_input()
    });
    assert_eq!(
        r.recovery_mode.as_deref(),
        Some("auto_quarantine_and_rebuild")
    );
    assert_eq!(
        r.quarantine_path.as_deref(),
        Some("/repo/.atlas/worldtree.db.quarantine.20260805T000000Z.0")
    );
    assert_eq!(r.recommended_rebuild_command(), "atlas build");
}

#[test]
fn fresh_graph_has_empty_suggestions() {
    let r = GraphReadiness::derive(healthy_input());
    assert!(r.suggestions.is_empty());
}

// ── serde round-trip ─────────────────────────────────────────────────────

#[test]
fn graph_readiness_serializes_and_deserializes() {
    let r = GraphReadiness::derive(healthy_input());
    let json = serde_json::to_string(&r).expect("serialize");
    let r2: GraphReadiness = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(r.error_code, r2.error_code);
    assert_eq!(r.graph_built, r2.graph_built);
    assert_eq!(r.graph_queryable, r2.graph_queryable);
    assert_eq!(r.graph_current, r2.graph_current);
    assert_eq!(r.integrity_state, r2.integrity_state);
}

#[test]
fn integrity_state_serde_round_trips() {
    for state in [
        IntegrityState::Clean,
        IntegrityState::SchemaMismatch,
        IntegrityState::SqliteCorrupt,
        IntegrityState::LogicalInconsistency,
    ] {
        let json = serde_json::to_string(&state).expect("serialize");
        let back: IntegrityState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(state, back);
    }
}

// ── GraphExecutionState derivation ───────────────────────────────────────

#[test]
fn fresh_graph_execution_state_is_fresh() {
    let r = GraphReadiness::derive(healthy_input());
    assert_eq!(r.execution_state, GraphExecutionState::Fresh);
}

#[test]
fn stale_graph_execution_state_is_stale() {
    let pending = vec!["src/main.rs".to_owned()];
    let r = GraphReadiness::derive(GraphReadinessInput {
        pending_graph_changes: &pending,
        ..healthy_input()
    });
    assert_eq!(r.execution_state, GraphExecutionState::Stale);
}

#[test]
fn degraded_build_execution_state_is_partial() {
    let r = GraphReadiness::derive(GraphReadinessInput {
        build_state: Some("degraded"),
        graph_has_content: true,
        ..healthy_input()
    });
    assert_eq!(r.execution_state, GraphExecutionState::Partial);
}

#[test]
fn corrupt_db_execution_state_is_corrupt() {
    let r = GraphReadiness::derive(GraphReadinessInput {
        db_open_error: Some("database disk image is malformed"),
        ..healthy_input()
    });
    assert_eq!(r.execution_state, GraphExecutionState::Corrupt);
}

#[test]
fn schema_mismatch_execution_state_is_corrupt() {
    let r = GraphReadiness::derive(GraphReadinessInput {
        db_open_error: Some("noncanonical_path rows detected"),
        ..healthy_input()
    });
    assert_eq!(r.execution_state, GraphExecutionState::Corrupt);
}

#[test]
fn missing_graph_execution_state_is_missing() {
    let r = GraphReadiness::derive(GraphReadinessInput {
        db_exists: false,
        build_state: None,
        graph_has_content: false,
        ..healthy_input()
    });
    assert_eq!(r.execution_state, GraphExecutionState::Missing);
}

#[test]
fn unbuilt_graph_without_db_missing_execution_state() {
    let r = GraphReadiness::derive(GraphReadinessInput {
        build_state: Some("building"),
        graph_has_content: false,
        ..healthy_input()
    });
    assert_eq!(r.execution_state, GraphExecutionState::Missing);
}

#[test]
fn corrupt_trumps_missing_in_execution_state() {
    // A DB that exists but fails to open should yield Corrupt, not Missing.
    let r = GraphReadiness::derive(GraphReadinessInput {
        db_exists: true,
        db_open_error: Some("database disk image is malformed"),
        build_state: None,
        graph_has_content: false,
        ..healthy_input()
    });
    assert_eq!(r.execution_state, GraphExecutionState::Corrupt);
}

#[test]
fn corrupt_trumps_stale_in_execution_state() {
    let pending = vec!["src/main.rs".to_owned()];
    let r = GraphReadiness::derive(GraphReadinessInput {
        pending_graph_changes: &pending,
        graph_error: Some("database disk image is malformed"),
        ..healthy_input()
    });
    assert_eq!(r.execution_state, GraphExecutionState::Corrupt);
}

#[test]
fn partial_trumps_stale_in_execution_state() {
    let pending = vec!["src/main.rs".to_owned()];
    let r = GraphReadiness::derive(GraphReadinessInput {
        build_state: Some("degraded"),
        pending_graph_changes: &pending,
        graph_has_content: true,
        ..healthy_input()
    });
    assert_eq!(r.execution_state, GraphExecutionState::Partial);
}

#[test]
fn execution_state_as_str_values() {
    assert_eq!(GraphExecutionState::Fresh.as_str(), "fresh");
    assert_eq!(GraphExecutionState::Stale.as_str(), "stale");
    assert_eq!(GraphExecutionState::Partial.as_str(), "partial");
    assert_eq!(GraphExecutionState::Corrupt.as_str(), "corrupt");
    assert_eq!(GraphExecutionState::Missing.as_str(), "missing");
}

#[test]
fn is_queryable_helper() {
    assert!(GraphExecutionState::Fresh.is_queryable());
    assert!(GraphExecutionState::Stale.is_queryable());
    assert!(GraphExecutionState::Partial.is_queryable());
    assert!(!GraphExecutionState::Corrupt.is_queryable());
    assert!(!GraphExecutionState::Missing.is_queryable());
}

#[test]
fn execution_state_serde_round_trips() {
    for state in [
        GraphExecutionState::Fresh,
        GraphExecutionState::Stale,
        GraphExecutionState::Partial,
        GraphExecutionState::Corrupt,
        GraphExecutionState::Missing,
    ] {
        let json = serde_json::to_string(&state).expect("serialize");
        let back: GraphExecutionState = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(state, back);
    }
}

// ── check_tool — diagnostics always allowed ──────────────────────────────

#[test]
fn diagnostics_allowed_in_fresh_state() {
    let r = GraphReadiness::derive(healthy_input());
    let v = r.check_tool(
        GraphToolRequirement::Diagnostics,
        ReadinessOverride::default(),
    );
    assert!(v.is_allowed());
    assert_eq!(v.execution_state(), GraphExecutionState::Fresh);
}

#[test]
fn diagnostics_allowed_in_corrupt_state() {
    let r = GraphReadiness::derive(GraphReadinessInput {
        db_open_error: Some("database disk image is malformed"),
        ..healthy_input()
    });
    let v = r.check_tool(
        GraphToolRequirement::Diagnostics,
        ReadinessOverride::default(),
    );
    assert!(v.is_allowed(), "diagnostics must always be allowed");
}

#[test]
fn diagnostics_allowed_in_missing_state() {
    let r = GraphReadiness::derive(GraphReadinessInput {
        db_exists: false,
        build_state: None,
        graph_has_content: false,
        ..healthy_input()
    });
    let v = r.check_tool(
        GraphToolRequirement::Diagnostics,
        ReadinessOverride::default(),
    );
    assert!(v.is_allowed(), "diagnostics must always be allowed");
}

// ── check_tool — corrupt / missing block all non-diagnostic tools ────────

#[test]
fn symbol_lookup_blocked_when_corrupt() {
    let r = GraphReadiness::derive(GraphReadinessInput {
        db_open_error: Some("database disk image is malformed"),
        ..healthy_input()
    });
    let v = r.check_tool(
        GraphToolRequirement::SymbolLookup,
        ReadinessOverride {
            allow_partial: true,
            allow_stale: true,
        },
    );
    assert!(v.is_blocked(), "corrupt blocks even with override flags");
}

#[test]
fn traversal_blocked_when_missing() {
    let r = GraphReadiness::derive(GraphReadinessInput {
        db_exists: false,
        build_state: None,
        graph_has_content: false,
        ..healthy_input()
    });
    let v = r.check_tool(
        GraphToolRequirement::Traversal,
        ReadinessOverride {
            allow_stale: true,
            allow_partial: true,
        },
    );
    assert!(v.is_blocked(), "missing blocks even with override flags");
}

#[test]
fn analysis_blocked_when_corrupt() {
    let r = GraphReadiness::derive(GraphReadinessInput {
        graph_error: Some("malformed database"),
        ..healthy_input()
    });
    let v = r.check_tool(GraphToolRequirement::Analysis, ReadinessOverride::default());
    assert!(v.is_blocked());
    assert_eq!(v.execution_state(), GraphExecutionState::Corrupt);
}

// ── check_tool — partial state ───────────────────────────────────────────

#[test]
fn symbol_lookup_blocked_in_partial_without_override() {
    let r = GraphReadiness::derive(GraphReadinessInput {
        build_state: Some("degraded"),
        graph_has_content: true,
        ..healthy_input()
    });
    let v = r.check_tool(
        GraphToolRequirement::SymbolLookup,
        ReadinessOverride::default(),
    );
    assert!(
        v.is_blocked(),
        "symbol lookup blocked in partial without override"
    );
}

#[test]
fn symbol_lookup_allowed_in_partial_with_allow_partial() {
    let r = GraphReadiness::derive(GraphReadinessInput {
        build_state: Some("degraded"),
        graph_has_content: true,
        ..healthy_input()
    });
    let v = r.check_tool(
        GraphToolRequirement::SymbolLookup,
        ReadinessOverride {
            allow_partial: true,
            ..ReadinessOverride::default()
        },
    );
    assert!(v.is_allowed());
    if let ReadinessVerdict::Allowed {
        safe_to_answer,
        warning,
        ..
    } = v
    {
        assert!(!safe_to_answer, "partial is not safe_to_answer");
        assert!(warning.is_some(), "warning should be present");
    }
}

#[test]
fn traversal_blocked_in_partial_even_with_allow_partial() {
    let r = GraphReadiness::derive(GraphReadinessInput {
        build_state: Some("degraded"),
        graph_has_content: true,
        ..healthy_input()
    });
    let v = r.check_tool(
        GraphToolRequirement::Traversal,
        ReadinessOverride {
            allow_partial: true,
            ..ReadinessOverride::default()
        },
    );
    assert!(v.is_blocked(), "traversal always blocked when partial");
}

#[test]
fn analysis_blocked_in_partial_even_with_allow_partial() {
    let r = GraphReadiness::derive(GraphReadinessInput {
        build_state: Some("degraded"),
        graph_has_content: true,
        ..healthy_input()
    });
    let v = r.check_tool(
        GraphToolRequirement::Analysis,
        ReadinessOverride {
            allow_partial: true,
            ..ReadinessOverride::default()
        },
    );
    assert!(v.is_blocked(), "analysis always blocked when partial");
}

// ── check_tool — stale state ─────────────────────────────────────────────

#[test]
fn all_tools_allowed_when_stale_with_warning() {
    let pending = vec!["src/main.rs".to_owned()];
    let r = GraphReadiness::derive(GraphReadinessInput {
        pending_graph_changes: &pending,
        ..healthy_input()
    });
    for req in [
        GraphToolRequirement::SymbolLookup,
        GraphToolRequirement::Traversal,
        GraphToolRequirement::Analysis,
    ] {
        let v = r.check_tool(req, ReadinessOverride::default());
        assert!(v.is_allowed(), "{req:?} should be allowed when stale");
        if let ReadinessVerdict::Allowed {
            safe_to_answer,
            warning,
            ..
        } = v
        {
            assert!(!safe_to_answer, "stale is not safe_to_answer");
            assert!(warning.is_some(), "warning expected when stale");
        }
    }
}

// ── check_tool — fresh state ─────────────────────────────────────────────

#[test]
fn all_tools_allowed_when_fresh_and_safe() {
    let r = GraphReadiness::derive(healthy_input());
    for req in [
        GraphToolRequirement::SymbolLookup,
        GraphToolRequirement::Traversal,
        GraphToolRequirement::Analysis,
    ] {
        let v = r.check_tool(req, ReadinessOverride::default());
        assert!(v.is_allowed(), "{req:?} should be allowed when fresh");
        if let ReadinessVerdict::Allowed {
            safe_to_answer,
            warning,
            ..
        } = v
        {
            assert!(safe_to_answer, "fresh should be safe_to_answer");
            assert!(warning.is_none(), "no warning expected when fresh");
        }
    }
}

// ── ReadinessVerdict helpers ─────────────────────────────────────────────

#[test]
fn verdict_is_allowed_and_is_blocked_are_exclusive() {
    let r = GraphReadiness::derive(healthy_input());
    let allowed = r.check_tool(
        GraphToolRequirement::SymbolLookup,
        ReadinessOverride::default(),
    );
    let r2 = GraphReadiness::derive(GraphReadinessInput {
        db_exists: false,
        build_state: None,
        graph_has_content: false,
        ..healthy_input()
    });
    let blocked = r2.check_tool(
        GraphToolRequirement::SymbolLookup,
        ReadinessOverride::default(),
    );
    assert!(allowed.is_allowed() && !allowed.is_blocked());
    assert!(blocked.is_blocked() && !blocked.is_allowed());
}

#[test]
fn verdict_execution_state_accessor() {
    let r = GraphReadiness::derive(healthy_input());
    let v = r.check_tool(
        GraphToolRequirement::Diagnostics,
        ReadinessOverride::default(),
    );
    assert_eq!(v.execution_state(), GraphExecutionState::Fresh);
}

#[test]
fn readiness_override_default_is_strict() {
    let d = ReadinessOverride::default();
    assert!(!d.allow_stale);
    assert!(!d.allow_partial);
}
