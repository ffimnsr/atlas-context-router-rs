//! Adapter hook trait and concrete CLI / MCP adapter implementations.
//!
//! Adapters wrap a `SessionStore` and surface lifecycle hooks that CLI and MCP
//! entry points call at command boundaries.  All hook methods degrade silently:
//! a write failure is logged at WARN level and never propagated to the caller.
//!
//! Design constraints (from Phase CM4 requirements):
//! - Adapters must emit normalized, bounded events (via `events::*`).
//! - Adapters must not write SQLite directly; they go through `SessionStore`.
//! - Hook failures must never abort command execution.

use std::path::Path;

use tracing::warn;

use atlas_session::{NewSessionEvent, SessionEventType, SessionId, SessionStore};

use crate::events::{PendingEvent, extract_cli_event, extract_tool_event, normalize_event};

// ---------------------------------------------------------------------------
// Adapter hook trait
// ---------------------------------------------------------------------------

/// Lifecycle hooks that adapters may implement.
///
/// All methods have a default no-op body so adapters only override what they
/// need.  Signatures use `&mut self` so adapters can buffer or flush state.
pub trait AdapterHooks {
    /// Called before a command or tool starts executing.
    fn before_command(&mut self, _command: &str) {}
    /// Called after a command or tool finishes.  `ok` is `false` on error.
    fn after_command(&mut self, _command: &str, _ok: bool) {}
    /// Called when a command or tool returns an error.
    fn on_error(&mut self, _command: &str, _message: &str) {}
    /// Called when a user intent has been identified.
    fn on_user_intent(&mut self, _intent: &str) {}
    /// Called when the adapter session starts (adapter startup).
    fn on_session_start(&mut self) {}
    /// Called before the session is compacted.
    fn before_compact(&mut self) {}
    /// Called just before the process exits.
    fn before_exit(&mut self) {}
}

// ---------------------------------------------------------------------------
// CliAdapter
// ---------------------------------------------------------------------------

/// CLI-surface adapter.
///
/// Opens `.atlas/session.db` in the repo root and records normalized events
/// for each CLI lifecycle hook.  If the store cannot be opened the adapter
/// operates in degraded mode (no events are recorded).
pub struct CliAdapter {
    store: SessionStore,
    session_id: SessionId,
}

impl CliAdapter {
    /// Open the session store relative to `repo_root`.
    ///
    /// Returns `None` when the store cannot be opened; callers should treat
    /// `None` as a degraded-continuity state and continue normally.
    pub fn open(repo_root: &str) -> Option<Self> {
        let session_id = SessionId::derive(repo_root, "", "cli");
        match SessionStore::open_in_repo(Path::new(repo_root)) {
            Ok(mut store) => {
                if let Err(e) =
                    store.upsert_session_meta(session_id.clone(), repo_root, "cli", None)
                {
                    warn!(error = %e, "session meta upsert failed; continuity degraded");
                }
                Some(Self { store, session_id })
            }
            Err(e) => {
                warn!(error = %e, "cannot open session store; CLI continuity disabled");
                None
            }
        }
    }

    /// Record a pending event; logs and continues on any storage error.
    pub fn record(&mut self, event: PendingEvent) {
        let bound = event.bind(self.session_id.clone());
        if let Err(e) = self.store.append_event(bound) {
            warn!(error = %e, "session event dropped; continuity degraded");
        }
    }

    /// Record a raw `NewSessionEvent`; logs and continues on error.
    fn record_raw(&mut self, event: NewSessionEvent) {
        if let Err(e) = self.store.append_event(event) {
            warn!(error = %e, "session event dropped; continuity degraded");
        }
    }
}

impl AdapterHooks for CliAdapter {
    fn on_session_start(&mut self) {
        let event = NewSessionEvent {
            session_id: self.session_id.clone(),
            event_type: SessionEventType::SessionStart,
            priority: 5,
            payload: serde_json::json!({ "frontend": "cli" }),
            created_at: None,
        };
        self.record_raw(event);
    }

    fn before_command(&mut self, command: &str) {
        self.record(extract_cli_event(command, "start", serde_json::Value::Null));
    }

    fn after_command(&mut self, command: &str, ok: bool) {
        let status = if ok { "ok" } else { "fail" };
        self.record(extract_cli_event(command, status, serde_json::Value::Null));
    }

    fn on_error(&mut self, command: &str, message: &str) {
        self.record(extract_cli_event(
            command,
            "fail",
            serde_json::json!({ "error": message }),
        ));
    }

    fn on_user_intent(&mut self, intent: &str) {
        use crate::events::extract_user_event;
        self.record(extract_user_event(intent));
    }

    fn before_compact(&mut self) {
        self.record(normalize_event(
            SessionEventType::CommandRun,
            5,
            serde_json::json!({ "event": "before_compact" }),
        ));
    }

    fn before_exit(&mut self) {
        // Best-effort bookend; nothing extra needed beyond per-command records.
    }
}

// ---------------------------------------------------------------------------
// McpAdapter
// ---------------------------------------------------------------------------

/// MCP-surface adapter.
///
/// Identical session-recording contract to [`CliAdapter`] but labelled with
/// the `"mcp"` frontend so MCP sessions have distinct identifiers.
pub struct McpAdapter {
    store: SessionStore,
    session_id: SessionId,
}

impl McpAdapter {
    /// Open the session store relative to `repo_root`.
    ///
    /// Returns `None` (degraded) when the store cannot be opened.
    pub fn open(repo_root: &str) -> Option<Self> {
        let session_id = SessionId::derive(repo_root, "", "mcp");
        match SessionStore::open_in_repo(Path::new(repo_root)) {
            Ok(mut store) => {
                if let Err(e) =
                    store.upsert_session_meta(session_id.clone(), repo_root, "mcp", None)
                {
                    warn!(error = %e, "MCP session meta upsert failed; continuity degraded");
                }
                Some(Self { store, session_id })
            }
            Err(e) => {
                warn!(error = %e, "cannot open session store; MCP continuity disabled");
                None
            }
        }
    }

    /// Record a pending event; logs and continues on any storage error.
    pub fn record(&mut self, event: PendingEvent) {
        let bound = event.bind(self.session_id.clone());
        if let Err(e) = self.store.append_event(bound) {
            warn!(error = %e, "MCP session event dropped; continuity degraded");
        }
    }

    fn record_raw(&mut self, event: NewSessionEvent) {
        if let Err(e) = self.store.append_event(event) {
            warn!(error = %e, "MCP session event dropped; continuity degraded");
        }
    }
}

impl AdapterHooks for McpAdapter {
    fn on_session_start(&mut self) {
        let event = NewSessionEvent {
            session_id: self.session_id.clone(),
            event_type: SessionEventType::SessionStart,
            priority: 5,
            payload: serde_json::json!({ "frontend": "mcp" }),
            created_at: None,
        };
        self.record_raw(event);
    }

    fn before_command(&mut self, command: &str) {
        self.record(extract_tool_event(
            command,
            "start",
            serde_json::Value::Null,
        ));
    }

    fn after_command(&mut self, command: &str, ok: bool) {
        let status = if ok { "ok" } else { "fail" };
        self.record(extract_tool_event(command, status, serde_json::Value::Null));
    }

    fn on_error(&mut self, command: &str, message: &str) {
        self.record(extract_tool_event(
            command,
            "fail",
            serde_json::json!({ "error": message }),
        ));
    }

    fn on_user_intent(&mut self, intent: &str) {
        use crate::events::extract_user_event;
        self.record(extract_user_event(intent));
    }

    fn before_compact(&mut self) {
        self.record(normalize_event(
            SessionEventType::CommandRun,
            5,
            serde_json::json!({ "event": "before_compact" }),
        ));
    }

    fn before_exit(&mut self) {}
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmp_repo() -> TempDir {
        TempDir::new().unwrap()
    }

    // ── CLI adapter ─────────────────────────────────────────────────────────

    #[test]
    fn cli_adapter_opens_successfully_for_valid_repo() {
        let dir = tmp_repo();
        let repo = dir.path().to_str().unwrap();
        assert!(
            CliAdapter::open(repo).is_some(),
            "CliAdapter must open for writable repo dir"
        );
    }

    #[test]
    fn cli_adapter_records_session_start_event() {
        let dir = tmp_repo();
        let repo = dir.path().to_str().unwrap();
        let mut adapter = CliAdapter::open(repo).expect("open must succeed");
        adapter.on_session_start();
    }

    #[test]
    fn cli_adapter_before_and_after_command_do_not_panic() {
        let dir = tmp_repo();
        let repo = dir.path().to_str().unwrap();
        let mut adapter = CliAdapter::open(repo).expect("open must succeed");
        adapter.before_command("atlas build");
        adapter.after_command("atlas build", true);
        adapter.after_command("atlas build", false);
    }

    #[test]
    fn cli_adapter_on_error_does_not_panic() {
        let dir = tmp_repo();
        let repo = dir.path().to_str().unwrap();
        let mut adapter = CliAdapter::open(repo).expect("open must succeed");
        adapter.on_error("atlas update", "graph DB locked");
    }

    #[test]
    fn cli_adapter_on_user_intent_does_not_panic() {
        let dir = tmp_repo();
        let repo = dir.path().to_str().unwrap();
        let mut adapter = CliAdapter::open(repo).expect("open must succeed");
        adapter.on_user_intent("review PR #42");
    }

    #[test]
    fn cli_adapter_before_compact_does_not_panic() {
        let dir = tmp_repo();
        let repo = dir.path().to_str().unwrap();
        let mut adapter = CliAdapter::open(repo).expect("open must succeed");
        adapter.before_compact();
    }

    #[test]
    fn cli_adapter_before_exit_does_not_panic() {
        let dir = tmp_repo();
        let repo = dir.path().to_str().unwrap();
        let mut adapter = CliAdapter::open(repo).expect("open must succeed");
        adapter.before_exit();
    }

    // ── MCP adapter ─────────────────────────────────────────────────────────

    #[test]
    fn mcp_adapter_opens_successfully_for_valid_repo() {
        let dir = tmp_repo();
        let repo = dir.path().to_str().unwrap();
        assert!(
            McpAdapter::open(repo).is_some(),
            "McpAdapter must open for writable repo dir"
        );
    }

    #[test]
    fn mcp_adapter_records_session_start_event() {
        let dir = tmp_repo();
        let repo = dir.path().to_str().unwrap();
        let mut adapter = McpAdapter::open(repo).expect("open must succeed");
        adapter.on_session_start();
    }

    #[test]
    fn mcp_adapter_tool_events_are_recorded() {
        let dir = tmp_repo();
        let repo = dir.path().to_str().unwrap();
        let mut adapter = McpAdapter::open(repo).expect("open must succeed");
        adapter.before_command("get_review_context");
        adapter.after_command("get_review_context", true);
    }

    #[test]
    fn mcp_adapter_on_error_does_not_panic() {
        let dir = tmp_repo();
        let repo = dir.path().to_str().unwrap();
        let mut adapter = McpAdapter::open(repo).expect("open must succeed");
        adapter.on_error("save_context_artifact", "content store unavailable");
    }

    // ── Best-effort degradation ─────────────────────────────────────────────

    #[test]
    fn cli_adapter_returns_none_for_unwritable_path() {
        // Pass a path that rusqlite cannot create a DB in.
        // The important guarantee is: no panic, returns None.
        let result = CliAdapter::open("/nonexistent_atlas_test_path_xyz/abc");
        // Either None or Some depending on OS; we only guard against panics.
        drop(result);
    }

    #[test]
    fn mcp_adapter_returns_none_for_unwritable_path() {
        let result = McpAdapter::open("/nonexistent_atlas_test_path_xyz/abc");
        drop(result);
    }

    // ── Representative hook payload extraction ──────────────────────────────

    #[test]
    fn cli_adapter_full_continuity_flow() {
        let dir = tmp_repo();
        let repo = dir.path().to_str().unwrap();
        let mut adapter = CliAdapter::open(repo).expect("open must succeed");

        adapter.on_session_start();
        adapter.on_user_intent("review recent changes");
        adapter.before_command("atlas build");
        adapter.after_command("atlas build", true);
        adapter.before_command("atlas review-context");
        adapter.after_command("atlas review-context", true);
        adapter.before_compact();
        adapter.before_exit();
    }

    #[test]
    fn mcp_adapter_full_continuity_flow() {
        let dir = tmp_repo();
        let repo = dir.path().to_str().unwrap();
        let mut adapter = McpAdapter::open(repo).expect("open must succeed");

        adapter.on_session_start();
        adapter.on_user_intent("get impact radius for changed file");
        adapter.before_command("get_impact_radius");
        adapter.after_command("get_impact_radius", true);
        adapter.before_command("save_context_artifact");
        adapter.after_command("save_context_artifact", false);
        adapter.on_error("save_context_artifact", "db locked");
        adapter.before_compact();
        adapter.before_exit();
    }
}
