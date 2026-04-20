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
