use super::*;

pub(crate) fn tool_build_or_update_graph(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let operation = match validate_build_operation_request(args) {
        Ok(operation) => operation,
        Err(payload) => return tool_execution_error_value(output_format, &payload),
    };
    let deprecated_operation_fields = operation.deprecated_input_fields.clone();
    let repo_root_path =
        find_repo_root(Utf8Path::new(repo_root)).context("cannot find git repo root")?;
    let repo_root_str = repo_root_path.as_str();

    fn build_status_json(db_path: &str, repo_root: &str) -> serde_json::Value {
        let Ok(store) = Store::open(db_path) else {
            return serde_json::Value::Null;
        };
        let Ok(Some(bs)) = store.get_build_status(repo_root) else {
            return serde_json::Value::Null;
        };
        let state_str = match bs.state {
            GraphBuildState::Building => "building",
            GraphBuildState::Built => "built",
            GraphBuildState::Degraded => "degraded",
            GraphBuildState::BuildFailed => "build_failed",
        };
        serde_json::json!({
            "state": state_str,
            "files_discovered": bs.files_discovered,
            "files_processed": bs.files_processed,
            "files_accepted": bs.files_accepted,
            "files_skipped_by_byte_budget": bs.files_skipped_by_byte_budget,
            "files_failed": bs.files_failed,
            "bytes_accepted": bs.bytes_accepted,
            "bytes_skipped": bs.bytes_skipped,
            "nodes_written": bs.nodes_written,
            "edges_written": bs.edges_written,
            "budget_stop_reason": bs.budget_stop_reason,
            "last_built_at": bs.last_built_at,
            "last_error": bs.last_error,
            "recovery_mode": bs.recovery_mode,
            "quarantine_path": bs.quarantine_path,
        })
    }

    fn budget_status_label(status: atlas_core::BudgetStatus) -> &'static str {
        match status {
            atlas_core::BudgetStatus::WithinBudget => "within_budget",
            atlas_core::BudgetStatus::OverrideClamped => "override_clamped",
            atlas_core::BudgetStatus::PartialResult => "partial_result",
            atlas_core::BudgetStatus::Blocked => "blocked",
        }
    }

    fn rebuild_result_label(
        recovery: Option<&atlas_store_sqlite::GraphStoreRecovery>,
        failure_reason: Option<&str>,
    ) -> &'static str {
        if failure_reason.is_some() {
            if recovery.is_some_and(|recovery| recovery.quarantine_path.is_some()) {
                "rebuild_failed"
            } else {
                "failed"
            }
        } else if recovery.is_some_and(|recovery| recovery.quarantine_path.is_some()) {
            "rebuilt_fresh"
        } else {
            "not_needed"
        }
    }

    fn rebuild_strategy_label(full_rebuild: bool) -> &'static str {
        if full_rebuild {
            "full_rebuild_from_source"
        } else {
            "incremental_update"
        }
    }

    fn readiness_summary(
        db_path: &str,
        repo_root: &str,
    ) -> (Option<String>, String, String, Vec<String>, String) {
        let readiness = match Store::open(db_path) {
            Ok(store) => super::super::shared::derive_graph_readiness(&store, repo_root, db_path),
            Err(error) => {
                use atlas_core::{GraphReadiness, GraphReadinessInput};
                GraphReadiness::derive(GraphReadinessInput {
                    repo_root,
                    db_path,
                    db_exists: std::path::Path::new(db_path).exists(),
                    db_open_error: Some(&error.to_string()),
                    build_state: None,
                    build_last_error: None,
                    graph_error: None,
                    recovery_mode: None,
                    quarantine_path: None,
                    pending_graph_changes: &[],
                    indexed_file_count: 0,
                    graph_has_content: false,
                    last_indexed_at: None,
                    retrieval_unavailable: true,
                })
            }
        };
        let error_code = readiness.error_code.clone();
        (
            readiness
                .health_class
                .map(|class| class.as_str().to_owned()),
            error_code.clone(),
            atlas_core::graph_health_error_message(&error_code).to_owned(),
            atlas_core::graph_health_error_suggestions(&error_code)
                .iter()
                .map(|item| (*item).to_owned())
                .collect(),
            atlas_core::error_code_docs_ref(&error_code),
        )
    }

    if operation.kind == BuildOperationKind::Update {
        let change_source = operation
            .change_source
            .clone()
            .expect("validated update operation requires change_source");
        let base = change_source.base.clone();
        let (target_kind, target) = match change_source.kind {
            ResolvedChangeSourceKind::Files => {
                ("files", UpdateTarget::Files(change_source.files.clone()))
            }
            ResolvedChangeSourceKind::Staged => ("staged", UpdateTarget::Staged),
            ResolvedChangeSourceKind::Base => (
                "base",
                UpdateTarget::BaseRef(base.clone().expect("base kind requires base ref")),
            ),
            ResolvedChangeSourceKind::WorkingTree => ("working_tree", UpdateTarget::WorkingTree),
        };

        let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root))
            .unwrap_or_default();
        let build_budget = config.build_run_budget()?;

        let recovery = match Store::prepare_graph_store_rebuild(
            db_path,
            atlas_store_sqlite::GraphRecoveryMode::AutoQuarantineAndRebuild,
            true,
        ) {
            Ok(recovery) => recovery,
            Err(error) => {
                let payload = json!({
                    "mode": operation.kind.as_str(),
                    "status": "error",
                    "health_class": error.health_class.as_str(),
                    "error_code": error.error_code,
                    "message": error.message,
                    "suggestions": error.suggestions,
                    "error_code_docs": atlas_core::error_code_docs_ref(error.health_class.as_str()),
                    "recovery_mode": error.recovery_mode.as_str(),
                    "quarantine_path": error.quarantine_path,
                    "rebuild_result": rebuild_result_label(None, error.failure_reason.as_deref()),
                    "failure_reason": error.failure_reason,
                    "rebuild_strategy": "full_rebuild_from_source",
                    "build_status": build_status_json(db_path, repo_root_str),
                });
                let envelope = ToolSuccessEnvelope::new("build_or_update_graph", payload);
                let mut response = normalized_tool_result_value(&envelope, output_format)?;
                inject_deprecated_input_fields(&mut response, &deprecated_operation_fields);
                return Ok(response);
            }
        };
        let full_rebuild = recovery.full_rebuild_required;

        if let Ok(s) = Store::open(db_path) {
            let _ = s.begin_build(repo_root_str);
        }

        crate::progress::report("detecting changed files", None);
        if crate::progress::is_canceled() {
            return Err(anyhow::anyhow!("canceled"));
        }
        crate::progress::report("updating graph", Some(10));

        let update_result = if full_rebuild {
            build_graph(
                repo_root_path.as_path(),
                db_path,
                &BuildOptions {
                    fail_fast: false,
                    dry_run: false,
                    batch_size: config.parse_batch_size(),
                    budget: build_budget,
                    source_repo_id: Some(stable_repo_id(repo_root_path.as_path())),
                    namespace_qualified_names: false,
                },
            )
            .map(|summary| atlas_engine::UpdateSummary {
                deleted: 0,
                renamed: 0,
                parsed: summary.parsed,
                skipped_unsupported: summary.skipped_unsupported,
                parse_errors: summary.parse_errors,
                chunk_upsert_failures: summary.chunk_upsert_failures,
                call_target_reconcile_failures: summary.call_target_reconcile_failures,
                nodes_updated: summary.nodes_inserted,
                edges_updated: summary.edges_inserted,
                warnings: summary.warnings,
                budget_counters: summary.budget_counters,
                budget: summary.budget,
                elapsed_ms: summary.elapsed_ms,
            })
        } else {
            update_graph(
                repo_root_path.as_path(),
                db_path,
                &UpdateOptions {
                    fail_fast: false,
                    dry_run: false,
                    batch_size: config.parse_batch_size(),
                    target,
                    budget: build_budget,
                    source_repo_id: Some(stable_repo_id(repo_root_path.as_path())),
                    namespace_qualified_names: false,
                },
            )
        };

        if let Ok(s) = Store::open(db_path) {
            match &update_result {
                Ok(sum) => {
                    let state =
                        if matches!(sum.budget.budget_status, atlas_core::BudgetStatus::Blocked) {
                            GraphBuildState::BuildFailed
                        } else if sum.is_degraded() {
                            GraphBuildState::Degraded
                        } else {
                            GraphBuildState::Built
                        };
                    let _ = s.finish_build(
                        repo_root_str,
                        BuildFinishStats {
                            state,
                            files_discovered: (sum.parsed + sum.deleted + sum.renamed) as i64,
                            files_processed: sum.parsed as i64,
                            files_accepted: sum.budget_counters.files_accepted as i64,
                            files_skipped_by_byte_budget: sum
                                .budget_counters
                                .files_skipped_by_byte_budget
                                as i64,
                            files_failed: sum.parse_errors as i64,
                            bytes_accepted: sum.budget_counters.bytes_accepted as i64,
                            bytes_skipped: sum.budget_counters.bytes_skipped as i64,
                            nodes_written: sum.nodes_updated as i64,
                            edges_written: sum.edges_updated as i64,
                            budget_stop_reason: sum.budget_counters.budget_stop_reason.clone(),
                        },
                    );
                    let _ = s.set_build_recovery_metadata(
                        repo_root_str,
                        Some(recovery.recovery_mode.as_str()),
                        recovery.quarantine_path.as_deref(),
                    );
                }
                Err(e) => {
                    let _ = s.fail_build(repo_root_str, &e.to_string());
                    let _ = s.set_build_recovery_metadata(
                        repo_root_str,
                        Some(recovery.recovery_mode.as_str()),
                        recovery.quarantine_path.as_deref(),
                    );
                }
            }
        }

        crate::progress::report("writing results", Some(90));
        let summary = match update_result {
            Ok(summary) => summary,
            Err(error) => {
                let payload = json!({
                    "mode": operation.kind.as_str(),
                    "status": "error",
                    "health_class": recovery.health_class.map(|class| class.as_str()),
                    "error_code": "failed_build",
                    "message": atlas_core::graph_health_error_message("failed_build"),
                    "suggestions": atlas_core::graph_health_error_suggestions("failed_build"),
                    "error_code_docs": atlas_core::error_code_docs_ref("failed_build"),
                    "recovery_mode": recovery.recovery_mode.as_str(),
                    "quarantine_path": recovery.quarantine_path,
                    "rebuild_result": rebuild_result_label(Some(&recovery), Some(&error.to_string())),
                    "failure_reason": error.to_string(),
                    "rebuild_strategy": rebuild_strategy_label(full_rebuild),
                    "build_status": build_status_json(db_path, repo_root_str),
                });
                let envelope = ToolSuccessEnvelope::new("build_or_update_graph", payload);
                let mut response = normalized_tool_result_value(&envelope, output_format)?;
                inject_deprecated_input_fields(&mut response, &deprecated_operation_fields);
                return Ok(response);
            }
        };
        crate::progress::report("update complete", Some(100));

        let status = if matches!(
            summary.budget.budget_status,
            atlas_core::BudgetStatus::Blocked
        ) {
            "blocked"
        } else if summary.is_degraded() {
            "degraded"
        } else {
            "completed"
        };
        let (health_class, error_code, message, suggestions, error_code_docs) =
            readiness_summary(db_path, repo_root_str);
        let warnings = summary.warnings.clone();
        let payload = json!({
            "mode": operation.kind.as_str(),
            "status": status,
            "health_class": health_class,
            "error_code": error_code,
            "message": message,
            "suggestions": suggestions,
            "error_code_docs": error_code_docs,
            "recovery_mode": recovery.recovery_mode.as_str(),
            "quarantine_path": recovery.quarantine_path,
            "rebuild_result": rebuild_result_label(Some(&recovery), None),
            "failure_reason": Value::Null,
            "rebuild_strategy": rebuild_strategy_label(full_rebuild),
            "source": {
                "target_kind": target_kind,
                "base_ref": base,
                "staged": matches!(change_source.kind, ResolvedChangeSourceKind::Staged),
            },
            "files_scanned": summary.parsed + summary.deleted + summary.renamed,
            "files_changed": summary.parsed + summary.deleted + summary.renamed,
            "files_parsed": summary.parsed,
            "files_deleted": summary.deleted,
            "files_renamed": summary.renamed,
            "files_skipped_unsupported": summary.skipped_unsupported,
            "files_skipped_unchanged": 0,
            "parse_error_count": summary.parse_errors,
            "chunk_upsert_failure_count": summary.chunk_upsert_failures,
            "call_target_reconcile_failure_count": summary.call_target_reconcile_failures,
            "nodes_written": summary.nodes_updated,
            "edges_written": summary.edges_updated,
            "duration_ms": summary.elapsed_ms as u64,
            "warnings": warnings,
            "stages": [
                {
                    "name": "detect_changes",
                    "status": "completed",
                    "item_count": summary.parsed + summary.deleted + summary.renamed,
                    "details": {
                        "target_kind": target_kind,
                    }
                },
                {
                    "name": "update_graph",
                    "status": status,
                    "item_count": summary.parsed,
                    "details": {
                        "skipped_unsupported": summary.skipped_unsupported,
                        "parse_errors": summary.parse_errors,
                    }
                },
                {
                    "name": "persist_graph",
                    "status": status,
                    "item_count": summary.nodes_updated + summary.edges_updated,
                    "details": {
                        "nodes_written": summary.nodes_updated,
                        "edges_written": summary.edges_updated,
                    }
                }
            ],
            "summary": {
                "budget_status": budget_status_label(summary.budget.budget_status),
                "budget_hit": summary.budget.budget_hit,
                "partial": summary.budget.partial,
                "safe_to_answer": summary.budget.safe_to_answer,
                "budget_counters": summary.budget_counters,
            },
            "build_status": build_status_json(db_path, repo_root_str),
        });
        let envelope = ToolSuccessEnvelope::new("build_or_update_graph", payload);
        let mut response = normalized_tool_result_value(&envelope, output_format)?;
        inject_budget_metadata(&mut response, &summary.budget);
        inject_deprecated_input_fields(&mut response, &deprecated_operation_fields);
        Ok(response)
    } else {
        let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root))
            .unwrap_or_default();
        let build_budget = config.build_run_budget()?;
        let recovery = match Store::prepare_graph_store_rebuild(
            db_path,
            atlas_store_sqlite::GraphRecoveryMode::AutoQuarantineAndRebuild,
            true,
        ) {
            Ok(recovery) => recovery,
            Err(error) => {
                let payload = json!({
                    "mode": operation.kind.as_str(),
                    "status": "error",
                    "health_class": error.health_class.as_str(),
                    "error_code": error.error_code,
                    "message": error.message,
                    "suggestions": error.suggestions,
                    "error_code_docs": atlas_core::error_code_docs_ref(error.health_class.as_str()),
                    "recovery_mode": error.recovery_mode.as_str(),
                    "quarantine_path": error.quarantine_path,
                    "rebuild_result": rebuild_result_label(None, error.failure_reason.as_deref()),
                    "failure_reason": error.failure_reason,
                    "rebuild_strategy": "full_rebuild_from_source",
                    "build_status": build_status_json(db_path, repo_root_str),
                });
                let envelope = ToolSuccessEnvelope::new("build_or_update_graph", payload);
                let mut response = normalized_tool_result_value(&envelope, output_format)?;
                inject_deprecated_input_fields(&mut response, &deprecated_operation_fields);
                return Ok(response);
            }
        };

        if let Ok(s) = Store::open(db_path) {
            let _ = s.begin_build(repo_root_str);
        }

        crate::progress::report("scanning repository files", None);
        if crate::progress::is_canceled() {
            return Err(anyhow::anyhow!("canceled"));
        }
        crate::progress::report("building graph", Some(10));

        let build_result = build_graph(
            repo_root_path.as_path(),
            db_path,
            &BuildOptions {
                fail_fast: false,
                dry_run: false,
                batch_size: config.parse_batch_size(),
                budget: build_budget,
                source_repo_id: Some(stable_repo_id(repo_root_path.as_path())),
                namespace_qualified_names: false,
            },
        );

        if let Ok(s) = Store::open(db_path) {
            match &build_result {
                Ok(sum) => {
                    let state =
                        if matches!(sum.budget.budget_status, atlas_core::BudgetStatus::Blocked) {
                            GraphBuildState::BuildFailed
                        } else if sum.is_degraded() {
                            GraphBuildState::Degraded
                        } else {
                            GraphBuildState::Built
                        };
                    let _ = s.finish_build(
                        repo_root_str,
                        BuildFinishStats {
                            state,
                            files_discovered: sum.scanned as i64,
                            files_processed: sum.parsed as i64,
                            files_accepted: sum.budget_counters.files_accepted as i64,
                            files_skipped_by_byte_budget: sum
                                .budget_counters
                                .files_skipped_by_byte_budget
                                as i64,
                            files_failed: sum.parse_errors as i64,
                            bytes_accepted: sum.budget_counters.bytes_accepted as i64,
                            bytes_skipped: sum.budget_counters.bytes_skipped as i64,
                            nodes_written: sum.nodes_inserted as i64,
                            edges_written: sum.edges_inserted as i64,
                            budget_stop_reason: sum.budget_counters.budget_stop_reason.clone(),
                        },
                    );
                    let _ = s.set_build_recovery_metadata(
                        repo_root_str,
                        Some(recovery.recovery_mode.as_str()),
                        recovery.quarantine_path.as_deref(),
                    );
                }
                Err(e) => {
                    let _ = s.fail_build(repo_root_str, &e.to_string());
                    let _ = s.set_build_recovery_metadata(
                        repo_root_str,
                        Some(recovery.recovery_mode.as_str()),
                        recovery.quarantine_path.as_deref(),
                    );
                }
            }
        }

        crate::progress::report("writing results", Some(90));
        let summary = match build_result {
            Ok(summary) => summary,
            Err(error) => {
                let payload = json!({
                    "mode": operation.kind.as_str(),
                    "status": "error",
                    "health_class": recovery.health_class.map(|class| class.as_str()),
                    "error_code": "failed_build",
                    "message": atlas_core::graph_health_error_message("failed_build"),
                    "suggestions": atlas_core::graph_health_error_suggestions("failed_build"),
                    "error_code_docs": atlas_core::error_code_docs_ref("failed_build"),
                    "recovery_mode": recovery.recovery_mode.as_str(),
                    "quarantine_path": recovery.quarantine_path,
                    "rebuild_result": rebuild_result_label(Some(&recovery), Some(&error.to_string())),
                    "failure_reason": error.to_string(),
                    "rebuild_strategy": "full_rebuild_from_source",
                    "build_status": build_status_json(db_path, repo_root_str),
                });
                let envelope = ToolSuccessEnvelope::new("build_or_update_graph", payload);
                let mut response = normalized_tool_result_value(&envelope, output_format)?;
                inject_deprecated_input_fields(&mut response, &deprecated_operation_fields);
                return Ok(response);
            }
        };
        crate::progress::report("build complete", Some(100));

        let status = if matches!(
            summary.budget.budget_status,
            atlas_core::BudgetStatus::Blocked
        ) {
            "blocked"
        } else if summary.is_degraded() {
            "degraded"
        } else {
            "completed"
        };
        let (health_class, error_code, message, suggestions, error_code_docs) =
            readiness_summary(db_path, repo_root_str);
        let warnings = summary.warnings.clone();
        let payload = json!({
            "mode": operation.kind.as_str(),
            "status": status,
            "health_class": health_class,
            "error_code": error_code,
            "message": message,
            "suggestions": suggestions,
            "error_code_docs": error_code_docs,
            "recovery_mode": recovery.recovery_mode.as_str(),
            "quarantine_path": recovery.quarantine_path,
            "rebuild_result": rebuild_result_label(Some(&recovery), None),
            "failure_reason": Value::Null,
            "rebuild_strategy": "full_rebuild_from_source",
            "source": {
                "target_kind": "full_build",
                "base_ref": Value::Null,
                "staged": false,
            },
            "files_scanned": summary.scanned,
            "files_changed": 0,
            "files_parsed": summary.parsed,
            "files_deleted": 0,
            "files_renamed": 0,
            "files_skipped_unsupported": summary.skipped_unsupported,
            "files_skipped_unchanged": summary.skipped_unchanged,
            "parse_error_count": summary.parse_errors,
            "chunk_upsert_failure_count": summary.chunk_upsert_failures,
            "call_target_reconcile_failure_count": summary.call_target_reconcile_failures,
            "nodes_written": summary.nodes_inserted,
            "edges_written": summary.edges_inserted,
            "duration_ms": summary.elapsed_ms as u64,
            "warnings": warnings,
            "stages": [
                {
                    "name": "scan_repo",
                    "status": "completed",
                    "item_count": summary.scanned,
                    "details": {
                        "files_scanned": summary.scanned,
                    }
                },
                {
                    "name": "parse_repo",
                    "status": status,
                    "item_count": summary.parsed,
                    "details": {
                        "skipped_unsupported": summary.skipped_unsupported,
                        "skipped_unchanged": summary.skipped_unchanged,
                        "parse_errors": summary.parse_errors,
                    }
                },
                {
                    "name": "persist_graph",
                    "status": status,
                    "item_count": summary.nodes_inserted + summary.edges_inserted,
                    "details": {
                        "nodes_written": summary.nodes_inserted,
                        "edges_written": summary.edges_inserted,
                    }
                }
            ],
            "summary": {
                "budget_status": budget_status_label(summary.budget.budget_status),
                "budget_hit": summary.budget.budget_hit,
                "partial": summary.budget.partial,
                "safe_to_answer": summary.budget.safe_to_answer,
                "budget_counters": summary.budget_counters,
            },
            "build_status": build_status_json(db_path, repo_root_str),
        });
        let envelope = ToolSuccessEnvelope::new("build_or_update_graph", payload);
        let mut response = normalized_tool_result_value(&envelope, output_format)?;
        inject_budget_metadata(&mut response, &summary.budget);
        inject_deprecated_input_fields(&mut response, &deprecated_operation_fields);
        Ok(response)
    }
}
