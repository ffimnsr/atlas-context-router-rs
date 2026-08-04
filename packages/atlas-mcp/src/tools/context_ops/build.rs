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

        if let Ok(s) = Store::open(db_path) {
            let _ = s.begin_build(repo_root_str);
        }

        crate::progress::report("detecting changed files", None);
        if crate::progress::is_canceled() {
            return Err(anyhow::anyhow!("canceled"));
        }
        crate::progress::report("updating graph", Some(10));

        let update_result = update_graph(
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
        );

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
                }
                Err(e) => {
                    let _ = s.fail_build(repo_root_str, &e.to_string());
                }
            }
        }

        crate::progress::report("writing results", Some(90));
        let summary = update_result?;
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
        let warnings = summary.warnings.clone();
        let payload = json!({
            "mode": operation.kind.as_str(),
            "status": status,
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
                }
                Err(e) => {
                    let _ = s.fail_build(repo_root_str, &e.to_string());
                }
            }
        }

        crate::progress::report("writing results", Some(90));
        let summary = build_result?;
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
        let warnings = summary.warnings.clone();
        let payload = json!({
            "mode": operation.kind.as_str(),
            "status": status,
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
