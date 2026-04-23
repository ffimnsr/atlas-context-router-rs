use anyhow::{Context, Result};
use camino::Utf8Path;
use serde_json::{Map, Value, json};

use atlas_adapters::{
    ArtifactIdentity, derive_content_db_path, derive_session_db_path, generate_source_id,
};
use atlas_contentstore::{ContentStore, SourceMeta};
use atlas_core::model::{ChangeType, ContextIntent, ContextRequest, ContextTarget};
use atlas_engine::{Config, UpdateOptions, UpdateTarget, update_graph};
use atlas_impact::analyze as advanced_impact;
use atlas_review::{ContextEngine, build_explain_change_summary};
use atlas_session::{SessionId, SessionStore};
use atlas_store_sqlite::{BuildFinishStats, Store};

use super::metadata::{
    build_freshness_metadata, collect_context_hints, compute_prompt_routing, snapshot_to_value,
};
use super::payload::{
    extract_changed_files, extract_hook_command, extract_hook_status, extract_tool_name,
    tool_may_change_files,
};
use super::policy::{
    HookLifecycleAction, HookPersistence, HookPolicy, MAX_HOOK_REVIEW_REFRESH_DEPTH,
    MAX_HOOK_REVIEW_REFRESH_FILES, MAX_HOOK_REVIEW_REFRESH_NODES, ReviewRefreshArtifact,
    ReviewRefreshResult,
};

pub(crate) fn execute_hook_actions(
    repo: &str,
    graph_db_path: &str,
    frontend: &str,
    policy: &HookPolicy,
    persisted: &HookPersistence,
    payload: &Value,
) -> Value {
    let mut actions = Map::new();

    let lifecycle =
        execute_lifecycle_action(repo, graph_db_path, frontend, policy, &persisted.session_id);
    if !lifecycle.is_null() {
        actions.insert("lifecycle".to_owned(), lifecycle);
    }

    let prompt_routing = execute_prompt_routing_action(repo, graph_db_path, policy, payload);
    if !prompt_routing.is_null() {
        actions.insert("prompt_routing".to_owned(), prompt_routing);
    }

    let graph_refresh = execute_graph_refresh_action(repo, graph_db_path, policy, payload);
    if !graph_refresh.is_null() {
        actions.insert("graph_refresh".to_owned(), graph_refresh);
    }

    let freshness = build_freshness_metadata(policy, repo, payload);
    if !freshness.is_null() {
        actions.insert("freshness".to_owned(), freshness);
    }

    let review_refresh = execute_review_refresh_action(
        repo,
        graph_db_path,
        policy,
        actions.get("graph_refresh"),
        payload,
        &persisted.session_id,
    );
    if !review_refresh.is_null() {
        actions.insert("review_refresh".to_owned(), review_refresh);
    }

    if actions.is_empty() {
        Value::Null
    } else {
        Value::Object(actions)
    }
}

fn execute_lifecycle_action(
    repo: &str,
    graph_db_path: &str,
    frontend: &str,
    policy: &HookPolicy,
    session_id: &SessionId,
) -> Value {
    match policy.lifecycle {
        HookLifecycleAction::None => Value::Null,
        HookLifecycleAction::LoadRestore => load_restore_state(repo, graph_db_path, session_id),
        HookLifecycleAction::PersistHandoff => persist_handoff_artifact(
            repo,
            graph_db_path,
            frontend,
            session_id,
            policy.canonical_event,
        ),
        HookLifecycleAction::VerifyRestore => verify_restore_state(graph_db_path, session_id),
    }
}

fn load_restore_state(repo: &str, graph_db_path: &str, session_id: &SessionId) -> Value {
    let session_db_path = derive_session_db_path(graph_db_path);
    let mut store = match SessionStore::open(&session_db_path) {
        Ok(store) => store,
        Err(error) => {
            return json!({
                "status": "error",
                "reason": "session_store_unavailable",
                "error": error.to_string(),
            });
        }
    };

    let snapshot = match store.get_resume_snapshot(session_id) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return json!({
                "status": "error",
                "reason": "resume_snapshot_unavailable",
                "error": error.to_string(),
            });
        }
    };
    let pending_snapshot = snapshot.as_ref().is_some_and(|row| !row.consumed);
    let context_hints = collect_context_hints(repo, graph_db_path, &store, session_id);

    if pending_snapshot {
        let _ = store.mark_resume_consumed(session_id, true);
    }

    json!({
        "status": "loaded",
        "resume_loaded": pending_snapshot,
        "snapshot": snapshot.as_ref().map(snapshot_to_value),
        "context_hints": context_hints,
    })
}

fn persist_handoff_artifact(
    repo: &str,
    graph_db_path: &str,
    frontend: &str,
    session_id: &SessionId,
    trigger: &str,
) -> Value {
    let session_db_path = derive_session_db_path(graph_db_path);
    let mut store = match SessionStore::open(&session_db_path) {
        Ok(store) => store,
        Err(error) => {
            return json!({
                "status": "error",
                "reason": "session_store_unavailable",
                "error": error.to_string(),
            });
        }
    };

    let snapshot = match store.get_resume_snapshot(session_id) {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => match store.build_resume(session_id) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return json!({
                    "status": "error",
                    "reason": "resume_build_failed",
                    "error": error.to_string(),
                });
            }
        },
        Err(error) => {
            return json!({
                "status": "error",
                "reason": "resume_snapshot_unavailable",
                "error": error.to_string(),
            });
        }
    };

    let context_hints = collect_context_hints(repo, graph_db_path, &store, session_id);
    let artifact = json!({
        "hook_event": trigger,
        "frontend": frontend,
        "session_id": session_id.as_str(),
        "resume_snapshot": snapshot_to_value(&snapshot),
        "context_hints": context_hints,
    });
    let artifact_json = match serde_json::to_string(&artifact) {
        Ok(text) => text,
        Err(error) => {
            return json!({
                "status": "error",
                "reason": "handoff_serialize_failed",
                "error": error.to_string(),
            });
        }
    };

    let identity = ArtifactIdentity::artifact_label(format!("{repo}:{frontend}:{trigger}:handoff"));
    let source_id = generate_source_id(&identity, &artifact_json);
    let mut content_store = match ContentStore::open(&derive_content_db_path(graph_db_path)) {
        Ok(store) => store,
        Err(error) => {
            return json!({
                "status": "error",
                "reason": "content_store_unavailable",
                "error": error.to_string(),
            });
        }
    };
    if let Err(error) = content_store.migrate() {
        return json!({
            "status": "error",
            "reason": "content_store_migrate_failed",
            "error": error.to_string(),
        });
    }

    let meta = SourceMeta {
        id: source_id.clone(),
        session_id: Some(session_id.as_str().to_owned()),
        source_type: "hook_handoff".to_owned(),
        label: format!("hook:{frontend}:{trigger}:handoff"),
        repo_root: Some(repo.to_owned()),
        identity_kind: identity.kind_str().to_owned(),
        identity_value: identity.value().to_owned(),
    };

    if let Err(error) = content_store.index_artifact(meta, &artifact_json, "application/json") {
        return json!({
            "status": "error",
            "reason": "handoff_persist_failed",
            "error": error.to_string(),
        });
    }

    json!({
        "status": "persisted",
        "resume_source_id": source_id,
        "snapshot_event_count": snapshot.event_count,
        "context_hints": context_hints,
    })
}

fn verify_restore_state(graph_db_path: &str, session_id: &SessionId) -> Value {
    let session_db_path = derive_session_db_path(graph_db_path);
    let store = match SessionStore::open(&session_db_path) {
        Ok(store) => store,
        Err(error) => {
            return json!({
                "status": "error",
                "reason": "session_store_unavailable",
                "error": error.to_string(),
            });
        }
    };

    let snapshot = match store.get_resume_snapshot(session_id) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return json!({
                "status": "error",
                "reason": "resume_snapshot_unavailable",
                "error": error.to_string(),
            });
        }
    };
    let meta = store.get_session_meta(session_id).ok().flatten();

    json!({
        "status": "verified",
        "has_resume_snapshot": snapshot.is_some(),
        "snapshot_consumed": snapshot.as_ref().map(|row| row.consumed),
        "snapshot_event_count": snapshot.as_ref().map(|row| row.event_count),
        "last_resume_at": meta.as_ref().and_then(|row| row.last_resume_at.clone()),
        "last_compaction_at": meta.and_then(|row| row.last_compaction_at),
    })
}

fn execute_prompt_routing_action(
    repo: &str,
    graph_db_path: &str,
    policy: &HookPolicy,
    payload: &Value,
) -> Value {
    if !policy.prompt_routing {
        return Value::Null;
    }

    let Some(prompt_routing) = compute_prompt_routing(repo, graph_db_path, payload) else {
        return json!({
            "status": "skipped",
            "reason": "no_prompt_text",
        });
    };

    json!({
        "status": "routed",
        "prompt_excerpt": prompt_routing.prompt_excerpt,
        "query": prompt_routing.query,
        "intent": prompt_routing.intent,
        "target": prompt_routing.target,
        "saved_context_hits": prompt_routing.hits,
    })
}

fn execute_graph_refresh_action(
    repo: &str,
    graph_db_path: &str,
    policy: &HookPolicy,
    payload: &Value,
) -> Value {
    if !policy.graph_refresh {
        return Value::Null;
    }

    let tool_name = extract_tool_name(payload);
    let changed_files = extract_changed_files(repo, payload);
    if let Some(tool_name) = tool_name.as_deref()
        && !tool_may_change_files(tool_name)
        && changed_files.is_empty()
    {
        return json!({
            "status": "skipped",
            "reason": "tool_not_graph_relevant",
            "tool_name": tool_name,
        });
    }

    let target = if changed_files.is_empty() {
        UpdateTarget::WorkingTree
    } else {
        UpdateTarget::Files(changed_files.clone())
    };
    let config = Config::load(&atlas_engine::paths::atlas_dir(repo)).unwrap_or_default();
    let Ok(build_budget) = config.build_run_budget() else {
        return json!({
            "status": "error",
            "tool_name": tool_name,
            "changed_files": changed_files,
            "error": "invalid build budget config",
        });
    };
    if let Ok(store) = Store::open(graph_db_path) {
        let _ = store.begin_build(repo);
    }

    let result = update_graph(
        Utf8Path::new(repo),
        graph_db_path,
        &UpdateOptions {
            fail_fast: false,
            batch_size: config.parse_batch_size(),
            target,
            budget: build_budget,
        },
    );

    match result {
        Ok(summary) => {
            if let Ok(store) = Store::open(graph_db_path) {
                let state = if matches!(
                    summary.budget.budget_status,
                    atlas_core::BudgetStatus::Blocked
                ) {
                    atlas_store_sqlite::GraphBuildState::BuildFailed
                } else if summary.budget.partial {
                    atlas_store_sqlite::GraphBuildState::Degraded
                } else {
                    atlas_store_sqlite::GraphBuildState::Built
                };
                let _ = store.finish_build(
                    repo,
                    BuildFinishStats {
                        state,
                        files_discovered: (summary.parsed + summary.deleted + summary.renamed)
                            as i64,
                        files_processed: summary.parsed as i64,
                        files_accepted: summary.budget_counters.files_accepted as i64,
                        files_skipped_by_byte_budget: summary
                            .budget_counters
                            .files_skipped_by_byte_budget
                            as i64,
                        files_failed: summary.parse_errors as i64,
                        bytes_accepted: summary.budget_counters.bytes_accepted as i64,
                        bytes_skipped: summary.budget_counters.bytes_skipped as i64,
                        nodes_written: summary.nodes_updated as i64,
                        edges_written: summary.edges_updated as i64,
                        budget_stop_reason: summary.budget_counters.budget_stop_reason.clone(),
                    },
                );
            }
            json!({
                "status": "updated",
                "tool_name": tool_name,
                "changed_files": changed_files,
                "deleted": summary.deleted,
                "renamed": summary.renamed,
                "parsed": summary.parsed,
                "nodes_updated": summary.nodes_updated,
                "edges_updated": summary.edges_updated,
                "budget": summary.budget,
                "budget_counters": summary.budget_counters,
            })
        }
        Err(error) => {
            if let Ok(store) = Store::open(graph_db_path) {
                let _ = store.fail_build(repo, &error.to_string());
            }
            json!({
                "status": "error",
                "tool_name": tool_name,
                "changed_files": changed_files,
                "error": error.to_string(),
            })
        }
    }
}

fn execute_review_refresh_action(
    repo: &str,
    graph_db_path: &str,
    policy: &HookPolicy,
    graph_refresh: Option<&Value>,
    payload: &Value,
    session_id: &SessionId,
) -> Value {
    if !policy.review_refresh {
        return Value::Null;
    }

    let Some(refresh) = graph_refresh else {
        return json!({
            "status": "skipped",
            "reason": "graph_refresh_missing",
        });
    };

    if refresh.get("status") != Some(&json!("updated")) {
        return json!({
            "status": "skipped",
            "reason": "graph_refresh_not_updated",
        });
    }

    let Some(trigger) = classify_review_refresh_trigger(payload) else {
        return json!({
            "status": "skipped",
            "reason": "tool_not_review_relevant",
        });
    };

    let changed_files = extract_changed_files(repo, payload);
    if changed_files.is_empty() {
        return json!({
            "status": "skipped",
            "reason": "no_changed_files",
        });
    }

    match build_review_refresh_artifacts(repo, graph_db_path, session_id, trigger, &changed_files) {
        Ok(result) => json!({
            "status": "refreshed",
            "trigger": result.trigger,
            "changed_files": result.changed_files,
            "artifacts": result.artifacts.iter().map(|artifact| json!({
                "kind": artifact.kind,
                "source_id": artifact.source_id,
            })).collect::<Vec<_>>(),
            "max_depth": MAX_HOOK_REVIEW_REFRESH_DEPTH,
            "max_nodes": MAX_HOOK_REVIEW_REFRESH_NODES,
        }),
        Err(error) => json!({
            "status": "error",
            "reason": "review_refresh_failed",
            "error": format!("{error:#}"),
        }),
    }
}

fn build_review_refresh_artifacts(
    repo: &str,
    graph_db_path: &str,
    session_id: &SessionId,
    trigger: &'static str,
    changed_files: &[String],
) -> Result<ReviewRefreshResult> {
    let bounded_files: Vec<String> = changed_files
        .iter()
        .take(MAX_HOOK_REVIEW_REFRESH_FILES)
        .cloned()
        .collect();
    let changed: Vec<atlas_core::model::ChangedFile> = bounded_files
        .iter()
        .cloned()
        .map(|path| atlas_core::model::ChangedFile {
            path,
            change_type: ChangeType::Modified,
            old_path: None,
        })
        .collect();

    let store = Store::open(graph_db_path)
        .with_context(|| format!("cannot open database at {graph_db_path}"))?;
    let review_request = ContextRequest {
        intent: ContextIntent::Review,
        target: ContextTarget::ChangedFiles {
            paths: bounded_files.clone(),
        },
        max_nodes: Some(MAX_HOOK_REVIEW_REFRESH_NODES),
        depth: Some(MAX_HOOK_REVIEW_REFRESH_DEPTH),
        ..ContextRequest::default()
    };
    let review_context = ContextEngine::new(&store)
        .build(&review_request)
        .context("review context generation failed")?;
    let explain_change = build_explain_change_summary(
        &store,
        &changed,
        &bounded_files,
        MAX_HOOK_REVIEW_REFRESH_DEPTH,
        MAX_HOOK_REVIEW_REFRESH_NODES,
    )
    .context("explain change generation failed")?;
    let impact_result = advanced_impact(
        store
            .impact_radius(
                &bounded_files.iter().map(String::as_str).collect::<Vec<_>>(),
                MAX_HOOK_REVIEW_REFRESH_DEPTH,
                MAX_HOOK_REVIEW_REFRESH_NODES,
                atlas_core::BudgetPolicy::default()
                    .graph_traversal
                    .edges
                    .default_limit,
            )
            .context("impact radius generation failed")?,
    );

    let review_source_id = persist_named_hook_artifact(
        repo,
        graph_db_path,
        session_id,
        trigger,
        "review_context",
        &serde_json::to_value(&review_context).context("cannot serialize review context")?,
    )?;
    let explain_source_id = persist_named_hook_artifact(
        repo,
        graph_db_path,
        session_id,
        trigger,
        "explain_change",
        &serde_json::to_value(&explain_change).context("cannot serialize explain change")?,
    )?;
    let impact_source_id = persist_named_hook_artifact(
        repo,
        graph_db_path,
        session_id,
        trigger,
        "impact_result",
        &serde_json::to_value(&impact_result).context("cannot serialize impact result")?,
    )?;

    Ok(ReviewRefreshResult {
        trigger,
        changed_files: bounded_files,
        artifacts: vec![
            ReviewRefreshArtifact {
                kind: "review_context",
                source_id: review_source_id,
            },
            ReviewRefreshArtifact {
                kind: "explain_change",
                source_id: explain_source_id,
            },
            ReviewRefreshArtifact {
                kind: "impact_result",
                source_id: impact_source_id,
            },
        ],
    })
}

fn persist_named_hook_artifact(
    repo: &str,
    graph_db_path: &str,
    session_id: &SessionId,
    trigger: &str,
    kind: &str,
    value: &Value,
) -> Result<String> {
    let artifact_json =
        serde_json::to_string_pretty(value).context("cannot serialize hook artifact")?;
    let artifact_text = format!(
        "kind: {kind}\n{}",
        artifact_json
            .lines()
            .enumerate()
            .map(|(index, line)| format!("{:06}| {line}", index + 1))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let identity = ArtifactIdentity::artifact_label(format!("{repo}:{trigger}:{kind}"));
    let source_id = generate_source_id(&identity, &artifact_text);
    let mut content_store = ContentStore::open(&derive_content_db_path(graph_db_path))
        .context("cannot open hook artifact content store")?;
    content_store
        .migrate()
        .context("cannot migrate hook artifact content store")?;
    content_store
        .index_artifact(
            SourceMeta {
                id: source_id.clone(),
                session_id: Some(session_id.as_str().to_owned()),
                source_type: kind.to_owned(),
                label: format!("hook:{trigger}:{kind}"),
                repo_root: Some(repo.to_owned()),
                identity_kind: identity.kind_str().to_owned(),
                identity_value: identity.value().to_owned(),
            },
            &artifact_text,
            "text/plain",
        )
        .context("cannot persist hook artifact")?;
    Ok(source_id)
}

fn classify_review_refresh_trigger(payload: &Value) -> Option<&'static str> {
    let status = extract_hook_status(payload)?;
    if !matches!(status.as_str(), "ok" | "success" | "completed" | "passed") {
        return None;
    }

    let tool_name = extract_tool_name(payload)?;
    if !tool_name.eq_ignore_ascii_case("bash") {
        return None;
    }

    let command = extract_hook_command(payload)?;
    classify_build_test_command(&command)
}

fn classify_build_test_command(command: &str) -> Option<&'static str> {
    let command = command.to_ascii_lowercase();
    let build_markers = [
        "cargo build",
        "cargo check",
        "go build",
        "npm run build",
        "pnpm build",
        "yarn build",
        "bun build",
    ];
    if build_markers.iter().any(|marker| command.contains(marker)) {
        return Some("build");
    }

    let test_markers = [
        "cargo test",
        "cargo nextest",
        "pytest",
        "go test",
        "npm test",
        "pnpm test",
        "yarn test",
        "bun test",
        "mvn test",
        "gradle test",
    ];
    test_markers
        .iter()
        .any(|marker| command.contains(marker))
        .then_some("test")
}
