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
    pub recent_runtime_failures: Vec<Value>,
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
mod builder;

pub use self::builder::build_wake_pack;

#[cfg(test)]
mod tests {
    use std::fs;

    use atlas_adapters::{
        ArtifactIdentity, derive_content_db_path, derive_session_db_path, generate_source_id,
    };
    use atlas_contentstore::{ContentStore, SourceMeta};
    use atlas_session::{
        MemoryImportance, MemoryScope, NewFeedback, NewMemory, NewSessionEvent, SessionEventType,
        SessionId, SessionStore,
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
        assert!(pack.recent_runtime_failures.is_empty());
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
    fn pack_surfaces_bounded_classified_runtime_failures() {
        let dir = TempDir::new().unwrap();
        let repo = dir.path().to_string_lossy().into_owned();
        let db_path = setup_db_path(&dir);
        let session_id = SessionId::derive(&repo, "", "cli");
        let mut store = open_session(&db_path);
        store
            .upsert_session_meta(session_id.clone(), &repo, "cli", None)
            .unwrap();

        for (at, severity, kind) in [
            ("2026-01-01T00:00:03Z", "error", "test_failure"),
            ("2026-01-01T00:00:02Z", "warning", "dead_code_warning"),
            ("2026-01-01T00:00:01Z", "error", "compiler_error"),
        ] {
            store
                .append_event(NewSessionEvent {
                    session_id: session_id.clone(),
                    event_type: SessionEventType::CommandFail,
                    priority: 2,
                    payload: json!({
                        "command": "cargo check",
                        "hook_metadata": {
                            "classification": {
                                "kind": kind,
                                "severity": severity,
                                "rule_id": format!("rule_{kind}"),
                                "matched_fields": ["stderr"]
                            }
                        }
                    }),
                    created_at: Some(at.to_owned()),
                })
                .unwrap();
        }

        let budget = WakeBudget::default().with_max_items(1);
        let build = build_wake_pack(&options(&repo, &db_path, "cli", budget));
        let failures = &build.pack.recent_runtime_failures;
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0]["at"], "2026-01-01T00:00:01Z");
        assert_eq!(failures[0]["classification"]["severity"], "error");

        let full = build_wake_pack(&options(&repo, &db_path, "cli", WakeBudget::default()));
        let failures = &full.pack.recent_runtime_failures;
        assert_eq!(failures.len(), 2);
        assert_eq!(failures[0]["classification"]["kind"], "compiler_error");
        assert_eq!(failures[1]["classification"]["kind"], "test_failure");
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
                tags: Vec::new(),
                links: Vec::new(),
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
                tags: Vec::new(),
                links: Vec::new(),
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
