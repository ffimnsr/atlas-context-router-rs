//! ICM-D — bounded wake-up pack model and builder.
//!
//! A wake-up pack summarizes session start: current focus, critical memories,
//! recent decisions, recent feedback, active concepts, changed files, graph
//! readiness, and retrieval hints. It is compact by design: every list is
//! bounded by a budget, large artifacts are referenced by `source_id` only,
//! and generation never fails hard — store errors degrade the pack with
//! warnings instead of blocking session start.
//!
//! The builder is shared by the MCP `wake_up` tool, the `atlas wake-up` CLI
//! command, and the `SessionStart` hook path so all surfaces produce the same
//! stable JSON shape.

use serde::Serialize;
use serde_json::{Value, json};

use atlas_adapters::derive_session_db_path;
use atlas_contentstore::{ContentStore, SearchFilters};
use atlas_session::{
    DecisionSearchHit, FeedbackRecord, MemoryImportance, MemoryListFilter, MemoryRecord,
    MemoryViewer, SessionEventType, SessionId, SessionStore,
};
use atlas_store_sqlite::Store;

use crate::graph_readiness::{derive_graph_readiness, derive_graph_readiness_open_failed};
use crate::payload::extract_prompt_text;

/// Default cap for every list in the wake-up pack.
pub const DEFAULT_MAX_ITEMS: usize = 10;
/// Hard ceiling for `max_items`; protects the response from unbounded growth.
pub const HARD_MAX_ITEMS: usize = 25;
/// Default cap for feedback records surfaced per pack (smallest list by design).
pub const DEFAULT_MAX_FEEDBACK_ITEMS: usize = 3;
/// Hard ceiling for feedback records per pack.
pub const HARD_MAX_FEEDBACK_ITEMS: usize = 10;
/// Default cap for pending graph-relevant changes in the readiness block.
pub const DEFAULT_MAX_PENDING_CHANGES: usize = 20;
/// Hard ceiling for pending graph-relevant changes.
pub const HARD_MAX_PENDING_CHANGES: usize = 100;

/// Central wake-up pack budget: every list in the pack is capped by these
/// values. Mirrors the `[memory.wake_up]` config section; config validation in
/// `atlas-engine` enforces the same hard caps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WakeBudget {
    /// Items per pack list (decisions, memories, concepts, changes, hints).
    pub max_items: usize,
    /// Feedback records surfaced per pack.
    pub max_feedback_items: usize,
    /// Pending graph-relevant changes listed in the readiness block.
    pub max_pending_changes: usize,
}

impl Default for WakeBudget {
    fn default() -> Self {
        Self {
            max_items: DEFAULT_MAX_ITEMS,
            max_feedback_items: DEFAULT_MAX_FEEDBACK_ITEMS,
            max_pending_changes: DEFAULT_MAX_PENDING_CHANGES,
        }
    }
}

impl WakeBudget {
    /// Budget from `[memory.wake_up]` config; safe defaults when the section
    /// is absent, clamped to hard caps so a misconfigured file can never
    /// produce an unbounded pack.
    pub fn from_config(config: &atlas_engine::Config) -> Self {
        let wake_up = &config.memory.wake_up;
        Self {
            max_items: wake_up.max_items.clamp(1, HARD_MAX_ITEMS),
            max_feedback_items: wake_up.max_feedback_items.clamp(1, HARD_MAX_FEEDBACK_ITEMS),
            max_pending_changes: wake_up
                .max_pending_changes
                .clamp(1, HARD_MAX_PENDING_CHANGES),
        }
    }

    /// Budget with `max_items` overridden (CLI `--max-items` / MCP `max_items`
    /// argument), clamped to the hard ceiling.
    pub fn with_max_items(&self, max_items: usize) -> Self {
        Self {
            max_items: max_items.clamp(1, HARD_MAX_ITEMS),
            ..*self
        }
    }
}

/// Options for [`build_wake_pack`].
#[derive(Debug, Clone)]
pub struct WakePackOptions<'a> {
    /// Canonical repo root (path-identity invariant).
    pub repo_root: &'a str,
    /// Graph database path (`worldtree.db`).
    pub graph_db_path: &'a str,
    /// Frontend identity recorded in the pack.
    pub frontend: &'a str,
    /// Explicit session id; when `None` the derived session for the repo +
    /// frontend is used.
    pub session_id: Option<String>,
    /// Optional agent memory partition label.
    pub agent_id: Option<&'a str>,
    /// Focus topic; topic-relevant decisions, memories, and feedback rank
    /// first in their lists.
    pub topic: Option<&'a str>,
    /// Pack size budget (config-backed at call sites).
    pub budget: WakeBudget,
}

/// Stable JSON model of a wake-up pack (ICM-D1). All lists are bounded by the
/// budget; large artifacts appear only as `source_id` references.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct WakePack {
    pub repo_root: String,
    pub session_id: String,
    pub frontend: String,
    pub agent_id: Option<String>,
    pub current_focus: WakeFocus,
    pub recent_decisions: Vec<Value>,
    pub critical_memories: Vec<Value>,
    pub recent_feedback: Vec<WakeFeedback>,
    pub active_memoir_concepts: Vec<String>,
    pub changed_files: Vec<String>,
    pub graph_readiness: Value,
    pub retrieval_hints: Vec<Value>,
    pub generated_at: String,
}

impl WakePack {
    /// Stable JSON serialization of the pack.
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("WakePack serializes to JSON")
    }
}

/// Current focus: last user intent plus bounded recent reasoning.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct WakeFocus {
    pub intent: Option<String>,
    pub reasoning: Vec<Value>,
}

/// One feedback record surfaced in the pack (ICM-C record shape, compact).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct WakeFeedback {
    pub record_id: String,
    pub tool_name: String,
    pub analysis_kind: String,
    pub predicted: String,
    pub actual: String,
    pub correction: String,
    pub related_symbol: Option<String>,
    pub related_file: Option<String>,
    pub source_id: Option<String>,
    pub created_at: String,
}

impl From<&FeedbackRecord> for WakeFeedback {
    fn from(record: &FeedbackRecord) -> Self {
        Self {
            record_id: record.id.clone(),
            tool_name: record.tool_name.clone(),
            analysis_kind: record.analysis_kind.clone(),
            predicted: record.predicted.clone(),
            actual: record.actual.clone(),
            correction: record.correction.clone(),
            related_symbol: record.related_symbol.clone(),
            related_file: record.related_file.clone(),
            source_id: record.source_id.clone(),
            created_at: record.created_at.clone(),
        }
    }
}

/// Result of a best-effort wake-up pack build. Generation never fails hard:
/// store errors surface as warnings with a degraded status.
#[derive(Debug, Clone)]
pub struct WakePackBuild {
    pub pack: WakePack,
    pub warnings: Vec<String>,
    /// `ok` when the session store was usable, `degraded` otherwise.
    pub status: &'static str,
    /// Session status: `active`, `no_session`, or `unavailable`.
    pub session_status: String,
    pub event_count: i64,
    pub pending_resume: bool,
}

/// Derive the session id: explicit `session_id` wins, otherwise the stable
/// session for the repo + frontend.
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
            "graph has not been built yet; run build_graph before graph-backed queries".to_owned(),
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

#[cfg(test)]
mod tests {
    use std::fs;

    use atlas_adapters::{
        ArtifactIdentity, derive_content_db_path, derive_session_db_path, generate_source_id,
    };
    use atlas_contentstore::{ContentStore, SourceMeta};
    use atlas_session::{
        MemoryImportance, MemoryScope, NewFeedback, NewMemory, SessionId, SessionStore,
    };
    use serde_json::{Value, json};
    use tempfile::TempDir;

    use super::*;

    fn setup_db_path(dir: &TempDir) -> String {
        dir.path()
            .join(".atlas")
            .join("worldtree.db")
            .to_string_lossy()
            .into_owned()
    }

    fn options<'a>(
        repo: &'a str,
        db_path: &'a str,
        frontend: &'a str,
        budget: WakeBudget,
    ) -> WakePackOptions<'a> {
        WakePackOptions {
            repo_root: repo,
            graph_db_path: db_path,
            frontend,
            session_id: None,
            agent_id: None,
            topic: None,
            budget,
        }
    }

    fn open_session(db_path: &str) -> SessionStore {
        let session_db = derive_session_db_path(db_path);
        if let Some(parent) = std::path::Path::new(&session_db).parent() {
            fs::create_dir_all(parent).unwrap();
        }
        SessionStore::open(&session_db).unwrap()
    }

    /// Save an artifact above the small-output threshold so it gets indexed
    /// and returns a `source_id` (pointer routing).
    fn save_artifact(repo: &str, db_path: &str, label: &str, source_type: &str) -> String {
        let payload = std::iter::repeat_n("safe wake-pack artifact payload", 60)
            .collect::<Vec<_>>()
            .join(" ");
        let raw = format!("{label}: {payload}");
        let content_db = derive_content_db_path(db_path);
        if let Some(parent) = std::path::Path::new(&content_db).parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut store = ContentStore::open(&content_db).unwrap();
        store.migrate().unwrap();
        let identity = ArtifactIdentity::artifact_label(format!("{repo}:{label}"));
        let meta = SourceMeta {
            id: generate_source_id(&identity, &raw),
            session_id: None,
            agent_id: None,
            source_type: source_type.to_owned(),
            label: label.to_owned(),
            repo_root: Some(repo.to_owned()),
            repo_roots: vec![repo.to_owned()],
            repo_id: None,
            repo_ids: vec![],
            identity_kind: identity.kind_str().to_owned(),
            identity_value: identity.value().to_owned(),
        };
        match store.route_output(meta, &raw, "text/plain").unwrap() {
            atlas_contentstore::OutputRouting::Pointer { source_id } => source_id,
            atlas_contentstore::OutputRouting::Preview { source_id, .. } => source_id,
            _ => panic!("medium artifact must be indexed"),
        }
    }

    #[test]
    fn empty_session_returns_normalized_pack() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);

        let build = build_wake_pack(&options(&repo, &db_path, "cli", WakeBudget::default()));
        let pack = &build.pack;
        assert_eq!(pack.repo_root, repo);
        assert_eq!(pack.frontend, "cli");
        assert_eq!(
            pack.session_id,
            SessionId::derive(&repo, "", "cli").as_str()
        );
        assert_eq!(build.session_status, "no_session");
        assert_eq!(build.status, "ok");
        assert!(pack.current_focus.intent.is_none());
        assert!(pack.recent_decisions.is_empty());
        assert!(pack.critical_memories.is_empty());
        assert!(pack.recent_feedback.is_empty());
        assert!(pack.active_memoir_concepts.is_empty());
        assert!(pack.changed_files.is_empty());
        assert!(pack.retrieval_hints.is_empty());
        assert_eq!(pack.graph_readiness["graph_built"], false);
        assert_eq!(pack.graph_readiness["execution_state"], "missing");
        assert!(
            build
                .warnings
                .iter()
                .any(|w| w.contains("graph has not been built"))
        );
        assert!(pack.generated_at.contains('T'));
    }

    #[test]
    fn pack_surfaces_memories_feedback_and_artifact_refs() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);

        let mut store = open_session(&db_path);
        store
            .store_memory(&NewMemory {
                repo_root: repo.clone(),
                session_id: None,
                frontend: Some("cli".to_owned()),
                scope: MemoryScope::Project,
                topic: "hooks".to_owned(),
                title: "session start".to_owned(),
                body: "wake the agent with a bounded pack".to_owned(),
                importance: MemoryImportance::Critical,
                source_id: None,
                metadata: json!({}),
            })
            .unwrap();
        let feedback_id = store
            .store_feedback(&NewFeedback {
                repo_root: repo.clone(),
                session_id: None,
                tool_name: "cli".to_owned(),
                analysis_kind: "dead_code".to_owned(),
                predicted: "dead code".to_owned(),
                actual: "used by tests".to_owned(),
                correction: "".to_owned(),
                related_symbol: None,
                related_file: None,
                source_id: None,
                metadata: json!({}),
            })
            .unwrap()
            .id;
        drop(store);
        let artifact_id = save_artifact(&repo, &db_path, "handoff-note", "handoff");

        let build = build_wake_pack(&options(&repo, &db_path, "cli", WakeBudget::default()));
        let pack = &build.pack;
        let memories = &pack.critical_memories;
        assert!(
            memories
                .iter()
                .any(|m| m["kind"] == "memory" && m["topic"] == "hooks"),
            "critical memory must be in the pack: {memories:?}"
        );
        assert!(
            memories
                .iter()
                .any(|m| m["kind"] == "artifact" && m["source_id"] == artifact_id),
            "artifact ref must be in the pack: {memories:?}"
        );
        assert!(
            pack.recent_feedback
                .iter()
                .any(|f| f.record_id == feedback_id && f.predicted == "dead code"),
            "feedback record must be in the pack: {:?}",
            pack.recent_feedback
        );
        assert_eq!(build.session_status, "no_session");
    }

    #[test]
    fn topic_prioritizes_relevant_memories_and_feedback() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);

        let mut store = open_session(&db_path);
        store
            .store_memory(&NewMemory {
                repo_root: repo.clone(),
                session_id: None,
                frontend: Some("cli".to_owned()),
                scope: MemoryScope::Project,
                topic: "hooks".to_owned(),
                title: "hook rules".to_owned(),
                body: "hooks must be non-blocking".to_owned(),
                importance: MemoryImportance::High,
                source_id: None,
                metadata: json!({}),
            })
            .unwrap();
        let hooks_feedback = store
            .store_feedback(&NewFeedback {
                repo_root: repo.clone(),
                session_id: None,
                tool_name: "cli".to_owned(),
                analysis_kind: "dead_code".to_owned(),
                predicted: "hooks are dead".to_owned(),
                actual: "hooks fire on session start".to_owned(),
                correction: "session-start hooks are installed".to_owned(),
                related_symbol: None,
                related_file: None,
                source_id: None,
                metadata: json!({}),
            })
            .unwrap()
            .id;
        // Unrelated feedback must still appear after topic hits when the topic
        // search yields fewer than the budget, but the topic hit comes first.
        let other_feedback = store
            .store_feedback(&NewFeedback {
                repo_root: repo.clone(),
                session_id: None,
                tool_name: "cli".to_owned(),
                analysis_kind: "remove".to_owned(),
                predicted: "removable".to_owned(),
                actual: "blocked by plugin".to_owned(),
                correction: "".to_owned(),
                related_symbol: None,
                related_file: None,
                source_id: None,
                metadata: json!({}),
            })
            .unwrap()
            .id;
        drop(store);

        let build = build_wake_pack(&WakePackOptions {
            topic: Some("hooks"),
            ..options(&repo, &db_path, "cli", WakeBudget::default())
        });
        let pack = &build.pack;
        assert_eq!(pack.current_focus.intent.as_deref(), Some("hooks"));
        assert!(
            pack.critical_memories.iter().any(|m| m["topic"] == "hooks"),
            "topic must surface topic-relevant memories"
        );
        let feedback_ids: Vec<&str> = pack
            .recent_feedback
            .iter()
            .map(|f| f.record_id.as_str())
            .collect();
        assert!(
            feedback_ids.contains(&hooks_feedback.as_str()),
            "topic feedback must be present: {feedback_ids:?}"
        );
        assert!(feedback_ids.contains(&other_feedback.as_str()));
        assert_eq!(feedback_ids[0], hooks_feedback, "topic hit ranks first");
    }

    #[test]
    fn budget_clamps_all_lists() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);

        for i in 0..5 {
            save_artifact(&repo, &db_path, &format!("artifact-{i}"), "mcp_artifact");
        }
        let mut store = open_session(&db_path);
        for i in 0..5 {
            store
                .store_feedback(&NewFeedback {
                    repo_root: repo.clone(),
                    session_id: None,
                    tool_name: "cli".to_owned(),
                    analysis_kind: "dead_code".to_owned(),
                    predicted: format!("prediction {i}"),
                    actual: "reality".to_owned(),
                    correction: "".to_owned(),
                    related_symbol: None,
                    related_file: None,
                    source_id: None,
                    metadata: json!({}),
                })
                .unwrap();
        }
        drop(store);

        let budget = WakeBudget {
            max_items: 2,
            max_feedback_items: 2,
            max_pending_changes: 1,
        };
        let build = build_wake_pack(&options(&repo, &db_path, "cli", budget));
        let pack = &build.pack;
        assert_eq!(pack.critical_memories.len(), 2);
        assert_eq!(pack.recent_feedback.len(), 2);
    }

    #[test]
    fn artifact_refs_never_inline_bodies() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);

        let source_id = save_artifact(&repo, &db_path, "oversized-handoff", "handoff");
        assert!(!source_id.is_empty());

        let build = build_wake_pack(&options(&repo, &db_path, "cli", WakeBudget::default()));
        let value: Value = build.pack.to_json();
        let serialized = serde_json::to_string(&value).unwrap();
        assert!(
            !serialized.contains("safe wake-pack artifact payload"),
            "artifact body must never be inlined"
        );
        assert!(serialized.contains(&source_id));
    }

    #[test]
    fn budget_from_config_maps_section_and_clamps() {
        let mut config = atlas_engine::Config::default();
        config.memory.wake_up.max_items = 200;
        config.memory.wake_up.max_feedback_items = 0;
        config.memory.wake_up.max_pending_changes = 40;
        let budget = WakeBudget::from_config(&config);
        assert_eq!(budget.max_items, HARD_MAX_ITEMS, "oversized values clamp");
        assert_eq!(budget.max_feedback_items, 1, "zero values clamp to minimum");
        assert_eq!(budget.max_pending_changes, 40);

        let budget = WakeBudget::from_config(&atlas_engine::Config::default());
        assert_eq!(budget.max_items, DEFAULT_MAX_ITEMS);
        assert_eq!(budget.max_feedback_items, DEFAULT_MAX_FEEDBACK_ITEMS);
        assert_eq!(budget.max_pending_changes, DEFAULT_MAX_PENDING_CHANGES);

        let clamped = WakeBudget::default().with_max_items(500);
        assert_eq!(clamped.max_items, HARD_MAX_ITEMS);
    }
}
