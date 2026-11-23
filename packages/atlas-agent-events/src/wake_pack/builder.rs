use super::*;

fn wake_session_id(repo_root: &str, frontend: &str, session_id: Option<&str>) -> SessionId {
    match session_id {
        Some(sid) if !sid.trim().is_empty() => SessionId(sid.trim().to_owned()),
        _ => SessionId::derive(repo_root, "", frontend),
    }
}

/// Normalize a resume-snapshot `recent_decisions` entry (or a topic-matched
/// decision hit) into one compact decision shape.
fn normalize_decision(
    summary: Option<&Value>,
    rationale: Option<&Value>,
    at: Option<&Value>,
    decision_id: Option<&str>,
    source_ids: &[String],
) -> Value {
    json!({
        "summary": summary.cloned().unwrap_or(Value::Null),
        "rationale": rationale.cloned().unwrap_or(Value::Null),
        "at": at.cloned().unwrap_or(Value::Null),
        "decision_id": decision_id.map(str::to_owned),
        "source_ids": source_ids,
    })
}

fn normalize_decision_hits(hits: &[DecisionSearchHit]) -> Vec<Value> {
    hits.iter()
        .map(|hit| {
            normalize_decision(
                Some(&Value::String(hit.decision.summary.clone())),
                hit.decision
                    .rationale
                    .as_deref()
                    .map(|r| Value::String(r.to_owned()))
                    .as_ref(),
                None,
                Some(&hit.decision.decision_id),
                &hit.decision.source_ids,
            )
        })
        .collect()
}

/// Merge snapshot decisions with topic-matched decision-memory hits,
/// deduplicated by summary and bounded by `max_items`.
fn merge_decisions(
    snapshot_entries: &[Value],
    topic_hits: Vec<Value>,
    max_items: usize,
) -> Vec<Value> {
    let mut merged: Vec<Value> = snapshot_entries.to_vec();
    for hit in topic_hits {
        let summary = hit.get("summary").and_then(|v| v.as_str());
        let already_present = summary.is_some_and(|s| {
            merged
                .iter()
                .any(|entry| entry.get("summary").and_then(|v| v.as_str()) == Some(s))
        });
        if !already_present {
            merged.push(hit);
        }
        if merged.len() >= max_items {
            break;
        }
    }
    merged.truncate(max_items);
    merged
}

/// Collect distinct concept strings (symbols, rules, workflows) for
/// `active_memoir_concepts`. There is no dedicated memoir store yet; this is
/// the closest bounded proxy until the ICM-E memoir surface ships.
fn collect_runtime_failures(snapshot_view: Option<&Value>, limit: usize) -> Vec<Value> {
    let Some(entries) = snapshot_view
        .and_then(|view| view.get("unresolved_errors"))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    let mut failures: Vec<Value> = entries
        .iter()
        .filter(|entry| {
            entry
                .get("classification")
                .and_then(|classification| classification.get("severity"))
                .and_then(Value::as_str)
                == Some("error")
        })
        .map(|entry| {
            json!({
                "command": entry.get("command").cloned().unwrap_or(Value::Null),
                "error": entry.get("error").cloned().unwrap_or(Value::Null),
                "classification": entry.get("classification").cloned().unwrap_or(Value::Null),
                "at": entry.get("at").cloned().unwrap_or(Value::Null),
            })
        })
        .collect();

    failures.sort_by(|left, right| {
        let left_at = left.get("at").and_then(Value::as_str).unwrap_or("");
        let right_at = right.get("at").and_then(Value::as_str).unwrap_or("");
        left_at.cmp(right_at).then_with(|| {
            let left_rule = left
                .pointer("/classification/rule_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            let right_rule = right
                .pointer("/classification/rule_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            left_rule.cmp(right_rule)
        })
    });
    failures.dedup();
    failures.truncate(limit);
    failures
}

fn collect_concepts(
    snapshot_view: Option<&Value>,
    store: Option<&SessionStore>,
    repo_root: &str,
    max_items: usize,
) -> Vec<String> {
    let mut concepts: Vec<String> = Vec::new();
    let push_unique = |concepts: &mut Vec<String>, value: String| {
        if !value.trim().is_empty() && !concepts.contains(&value) {
            concepts.push(value);
        }
    };

    if let Some(view) = snapshot_view {
        if let Some(symbols) = view.get("impacted_symbols").and_then(|v| v.as_array()) {
            for symbol in symbols {
                if let Some(s) = symbol.as_str() {
                    push_unique(&mut concepts, s.to_owned());
                }
                if concepts.len() >= max_items {
                    break;
                }
            }
        }
        if concepts.len() < max_items
            && let Some(rules) = view.get("active_rules").and_then(|v| v.as_array())
        {
            for rule in rules {
                if let Some(label) = rule.get("label").and_then(|v| v.as_str()) {
                    push_unique(&mut concepts, label.to_owned());
                }
                if concepts.len() >= max_items {
                    break;
                }
            }
        }
    }

    if let Some(store) = store {
        if concepts.len() < max_items
            && let Ok(symbols) = store.get_frequent_symbols(repo_root, max_items as u32)
        {
            for entry in symbols {
                push_unique(&mut concepts, entry.value);
                if concepts.len() >= max_items {
                    break;
                }
            }
        }
        if concepts.len() < max_items
            && let Ok(workflows) = store.get_recurring_workflows(repo_root, 3)
        {
            for workflow in workflows {
                push_unique(&mut concepts, workflow.pattern.join(" → "));
                if concepts.len() >= max_items {
                    break;
                }
            }
        }
    }

    concepts.truncate(max_items);
    concepts
}

/// Compact graph-readiness block for the wake-up pack.
fn graph_readiness_value(readiness: &atlas_core::GraphReadiness, max_pending: usize) -> Value {
    let pending: Vec<String> = readiness
        .pending_graph_changes
        .iter()
        .take(max_pending)
        .cloned()
        .collect();
    json!({
        "graph_built": readiness.graph_built,
        "graph_queryable": readiness.graph_queryable,
        "graph_current": readiness.graph_current,
        "stale_index": readiness.stale_index,
        "execution_state": readiness.execution_state.as_str(),
        "pending_graph_change_count": readiness.pending_graph_changes.len(),
        "pending_graph_changes": pending,
        "indexed_file_count": readiness.indexed_file_count,
        "last_indexed_at": readiness.last_indexed_at,
        "message": readiness.message,
    })
}

/// ISO-8601 UTC timestamp without an external time crate dependency.
fn format_now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = d.as_secs();
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86_400;
    // Approximate Gregorian date from epoch days.
    let (y, mo, da) = epoch_days_to_ymd(days);
    format!("{y:04}-{mo:02}-{da:02}T{h:02}:{m:02}:{s:02}Z")
}

fn epoch_days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    days += 719_468;
    let era = days / 146_097;
    let doe = days % 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let da = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    (y, mo, da)
}

/// Compact reference to a saved artifact: `source_id` only, never the body.
fn artifact_ref(content_store: Option<&ContentStore>, source_id: &str) -> Option<Value> {
    let cs = content_store?;
    let source = cs.get_source(source_id).ok().flatten()?;
    if source.source_type == "hook_event" {
        return None;
    }
    let chunk_count = cs
        .get_chunks(source_id)
        .map(|chunks| chunks.len())
        .unwrap_or(0);
    Some(json!({
        "kind": "artifact",
        "source_id": source.id,
        "label": source.label,
        "source_type": source.source_type,
        "agent_id": source.agent_id,
        "created_at": source.created_at,
        "chunk_count": chunk_count,
    }))
}

/// Compact reference to a stored memory record.
fn memory_ref(record: &MemoryRecord) -> Value {
    json!({
        "kind": "memory",
        "memory_id": record.id,
        "topic": record.topic,
        "title": record.title,
        "importance": record.importance,
        "scope": record.scope,
        "created_at": record.created_at,
        "updated_at": record.updated_at,
        "source_id": record.source_id,
    })
}

/// Critical/topic memories: memory rows ranked first (critical importance, or
/// topic recall when a topic is given), then recent saved artifacts as
/// `source_id`-only references. Bounded by `max_items`.
fn collect_critical_memories(
    store: Option<&SessionStore>,
    content_store: Option<&ContentStore>,
    repo_root: &str,
    session_id: &SessionId,
    frontend: &str,
    topic: Option<&str>,
    max_items: usize,
) -> Vec<Value> {
    let mut entries: Vec<Value> = Vec::new();

    if let Some(store) = store {
        let records: Vec<MemoryRecord> = if let Some(topic) = topic {
            let viewer = MemoryViewer {
                frontend: frontend.to_owned(),
                session_id: session_id.as_str().to_owned(),
            };
            store
                .recall_memories(
                    repo_root,
                    topic,
                    &MemoryListFilter::default(),
                    false,
                    &viewer,
                    max_items,
                )
                .map(|hits| hits.into_iter().map(|hit| hit.memory).collect())
                .unwrap_or_default()
        } else {
            let filter = MemoryListFilter {
                importance: Some(MemoryImportance::Critical),
                ..Default::default()
            };
            store.list_memories(repo_root, &filter).unwrap_or_default()
        };
        for record in records {
            if entries.len() >= max_items {
                break;
            }
            entries.push(memory_ref(&record));
        }
    }

    if entries.len() < max_items
        && let Some(cs) = content_store
    {
        let filters = SearchFilters {
            session_id: None,
            agent_id: None,
            source_type: None,
            repo_root: None,
            repo_roots: vec![repo_root.to_owned()],
            ..SearchFilters::default()
        };
        if let Ok(ids) = cs.recent_source_ids_by_prefix("", &filters, max_items) {
            for id in ids {
                if entries.len() >= max_items {
                    break;
                }
                if let Some(entry) = artifact_ref(content_store, &id) {
                    entries.push(entry);
                }
            }
        }
    }

    entries.truncate(max_items);
    entries
}

/// Recent feedback records: topic search hits first when a topic is given,
/// then the newest records, deduplicated by id and bounded by
/// `max_feedback_items`.
fn collect_recent_feedback(
    store: Option<&SessionStore>,
    repo_root: &str,
    topic: Option<&str>,
    max_feedback_items: usize,
) -> Vec<WakeFeedback> {
    let mut entries: Vec<WakeFeedback> = Vec::new();

    if let Some(store) = store {
        if let Some(topic) = topic
            && let Ok(hits) = store.search_feedback(
                repo_root,
                topic,
                &atlas_session::FeedbackSearchFilter::default(),
                max_feedback_items,
            )
        {
            for hit in hits {
                entries.push(WakeFeedback::from(&hit.feedback));
            }
        }
        if entries.len() < max_feedback_items
            && let Ok(recent) = store.recent_feedback(repo_root, max_feedback_items)
        {
            for record in recent {
                if entries.iter().any(|entry| entry.record_id == record.id) {
                    continue;
                }
                entries.push(WakeFeedback::from(&record));
                if entries.len() >= max_feedback_items {
                    break;
                }
            }
        }
    }

    entries.truncate(max_feedback_items);
    entries
}

/// Assemble the bounded session-start context pack. Best-effort: store
/// failures degrade the pack with warnings instead of failing the call.
pub fn build_wake_pack(opts: &WakePackOptions<'_>) -> WakePackBuild {
    let mut warnings: Vec<String> = Vec::new();
    let session_id = wake_session_id(opts.repo_root, opts.frontend, opts.session_id.as_deref());

    // ── graph readiness (independent of the session store) ──────────────────
    // SessionStore creates its parent dirs on open; Store does not, so create
    // the storage directory first so first-run wake-ups report `missing`
    // readiness instead of an open error.
    if let Some(parent) = std::path::Path::new(opts.graph_db_path).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let readiness = match Store::open(opts.graph_db_path) {
        Ok(store) => derive_graph_readiness(&store, opts.repo_root, opts.graph_db_path),
        Err(e) => {
            derive_graph_readiness_open_failed(opts.repo_root, opts.graph_db_path, &e.to_string())
        }
    };
    if !readiness.graph_built {
        warnings.push(
            "graph has not been built yet; call build_graph with {} before graph-backed queries"
                .to_owned(),
        );
    } else if readiness.stale_index {
        warnings.push(format!(
            "graph index is stale; {} graph-relevant file(s) changed since the last index",
            readiness.pending_graph_changes.len()
        ));
    }

    // ── session store: resume snapshot view, pending resume, global memory ───
    let session_db = derive_session_db_path(opts.graph_db_path);
    let store = match SessionStore::open(&session_db) {
        Ok(store) => Some(store),
        Err(e) => {
            warnings.push(format!("session store unavailable: {e}"));
            None
        }
    };

    let (snapshot_view, pending_resume, event_count, session_status) = match store.as_ref() {
        Some(store) => match store.get_session_meta(&session_id) {
            Ok(Some(_)) => {
                let pending_resume = store
                    .get_resume_snapshot(&session_id)
                    .ok()
                    .flatten()
                    .is_some_and(|snapshot| !snapshot.consumed);
                match store.build_resume_view(&session_id, opts.agent_id, opts.agent_id.is_none()) {
                    Ok(view) => {
                        let event_count = view
                            .get("event_count")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0);
                        (Some(view), pending_resume, event_count, "active")
                    }
                    Err(e) => {
                        warnings.push(format!("resume snapshot unavailable: {e}"));
                        (None, pending_resume, 0, "unavailable")
                    }
                }
            }
            Ok(None) => (None, false, 0, "no_session"),
            Err(e) => {
                warnings.push(format!("session metadata unavailable: {e}"));
                (None, false, 0, "unavailable")
            }
        },
        None => (None, false, 0, "unavailable"),
    };

    // ── current focus: last user intent + bounded recent reasoning ───────────
    let last_intent = snapshot_view
        .as_ref()
        .and_then(|view| view.get("last_user_intent"))
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .or_else(|| {
            // Events recorded through the shared service store prompt text under
            // `payload.prompt`, so `last_user_intent` stays empty; scan the most
            // recent UserIntent event for the prompt as a fallback.
            let events = store
                .as_ref()
                .and_then(|store| store.list_events(&session_id).ok())?;
            events.iter().rev().find_map(|event| {
                if event.event_type != SessionEventType::UserIntent {
                    return None;
                }
                let payload: Value = serde_json::from_str(&event.payload_json).ok()?;
                // The wrapper stores frontend/hook_event/metadata beside the
                // routed inner payload; only the inner payload carries the
                // prompt, so never scan the wrapper (it would match `frontend`).
                payload.get("payload").and_then(extract_prompt_text)
            })
        })
        .or_else(|| opts.topic.map(str::to_owned));
    let reasoning: Vec<Value> = snapshot_view
        .as_ref()
        .and_then(|view| view.get("recent_reasoning"))
        .and_then(|v| v.as_array())
        .map(|entries| {
            entries
                .iter()
                .take(opts.budget.max_items)
                .map(|entry| {
                    json!({
                        "summary": entry.get("summary").cloned().unwrap_or(Value::Null),
                        "source_id": entry.get("source_id").cloned().unwrap_or(Value::Null),
                        "at": entry.get("at").cloned().unwrap_or(Value::Null),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let current_focus = WakeFocus {
        intent: last_intent,
        reasoning,
    };

    // ── recent decisions: snapshot entries + topic-matched decision memory ───
    let snapshot_decisions: Vec<Value> = snapshot_view
        .as_ref()
        .and_then(|view| view.get("recent_decisions"))
        .and_then(|v| v.as_array())
        .map(|entries| {
            entries
                .iter()
                .map(|entry| {
                    normalize_decision(
                        entry.get("summary"),
                        entry.get("rationale"),
                        entry.get("at"),
                        None,
                        &[],
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    let topic_hits: Vec<Value> = if let Some(topic) = opts.topic {
        if let Some(store) = store.as_ref() {
            store
                .search_decisions(opts.repo_root, topic, None, opts.budget.max_items)
                .map(|hits| normalize_decision_hits(&hits))
                .unwrap_or_default()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    let recent_decisions = merge_decisions(&snapshot_decisions, topic_hits, opts.budget.max_items);

    // ── critical memories + recent feedback ──────────────────────────────────
    let content_db = atlas_adapters::derive_content_db_path(opts.graph_db_path);
    let content_store = ContentStore::open(&content_db).ok();
    let critical_memories = collect_critical_memories(
        store.as_ref(),
        content_store.as_ref(),
        opts.repo_root,
        &session_id,
        opts.frontend,
        opts.topic,
        opts.budget.max_items,
    );
    let recent_feedback = collect_recent_feedback(
        store.as_ref(),
        opts.repo_root,
        opts.topic,
        opts.budget.max_feedback_items,
    );

    // ── active concepts, changed files, retrieval hints ─────────────────────
    let active_memoir_concepts = collect_concepts(
        snapshot_view.as_ref(),
        store.as_ref(),
        opts.repo_root,
        opts.budget.max_items,
    );
    let recent_runtime_failures =
        collect_runtime_failures(snapshot_view.as_ref(), opts.budget.max_items);
    let changed_files: Vec<String> = snapshot_view
        .as_ref()
        .and_then(|view| view.get("changed_files"))
        .and_then(|v| v.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .take(opts.budget.max_items)
                .collect()
        })
        .unwrap_or_default();
    let retrieval_hints: Vec<Value> = snapshot_view
        .as_ref()
        .and_then(|view| view.get("retrieval_hints"))
        .and_then(|v| v.as_array())
        .map(|entries| {
            entries
                .iter()
                .take(opts.budget.max_items)
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    let status = if session_status == "unavailable" {
        "degraded"
    } else {
        "ok"
    };

    WakePackBuild {
        pack: WakePack {
            repo_root: opts.repo_root.to_owned(),
            session_id: session_id.as_str().to_owned(),
            frontend: opts.frontend.to_owned(),
            agent_id: opts.agent_id.map(str::to_owned),
            current_focus,
            recent_decisions,
            critical_memories,
            recent_feedback,
            active_memoir_concepts,
            changed_files,
            recent_runtime_failures,
            graph_readiness: graph_readiness_value(&readiness, opts.budget.max_pending_changes),
            retrieval_hints,
            generated_at: format_now_rfc3339(),
        },
        warnings,
        status,
        session_status: session_status.to_string(),
        event_count,
        pending_resume,
    }
}
