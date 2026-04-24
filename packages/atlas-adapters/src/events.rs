//! Event extraction API for Atlas adapters.
//!
//! Produces bounded, normalized `PendingEvent` values for each operation class.
//! Payloads exceeding `MAX_EVENT_PAYLOAD_BYTES` are replaced with a truncation
//! marker so that continuity storage never receives oversized blobs.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use atlas_session::{NewSessionEvent, SessionEventType, SessionId};

use crate::redact::redact_payload;

/// Maximum payload bytes allowed inline.  Matches `MAX_INLINE_EVENT_PAYLOAD_BYTES`
/// in `atlas-session` so that `SessionStore::append_event` never rejects a payload
/// produced here because of size.
pub const MAX_EVENT_PAYLOAD_BYTES: usize = 8 * 1024;

// ---------------------------------------------------------------------------
// PendingEvent
// ---------------------------------------------------------------------------

/// An event not yet bound to a specific session.
///
/// Created by the extraction functions, then promoted to a `NewSessionEvent`
/// inside an adapter by calling [`PendingEvent::bind`].
#[derive(Debug, Clone)]
pub struct PendingEvent {
    pub event_type: SessionEventType,
    pub priority: i32,
    pub payload: Value,
}

impl PendingEvent {
    /// Bind a session id, yielding a `NewSessionEvent` ready for the store.
    pub fn bind(self, session_id: SessionId) -> NewSessionEvent {
        NewSessionEvent {
            session_id,
            event_type: self.event_type,
            priority: self.priority,
            payload: self.payload,
            created_at: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Extraction functions
// ---------------------------------------------------------------------------

/// Build a CLI command event.
///
/// `status` should be `"start"`, `"ok"`, or `"fail"`.  `extra` carries any
/// additional structured fields and must not embed large stdout blobs.
pub fn extract_cli_event(command: &str, status: &str, extra: Value) -> PendingEvent {
    let event_type = if status == "fail" {
        SessionEventType::CommandFail
    } else {
        SessionEventType::CommandRun
    };
    let payload = normalize_payload(redact_payload(json!({
        "command": command,
        "status": status,
        "extra": extra,
    })));
    PendingEvent {
        event_type,
        priority: 2,
        payload,
    }
}

/// Build a graph operation event (build or update).
///
/// `event_type` should be `GraphBuild` or `GraphUpdate`.
/// `payload` carries operation-specific metrics (file counts, elapsed time, …).
pub fn extract_graph_event(event_type: SessionEventType, payload: Value) -> PendingEvent {
    PendingEvent {
        event_type,
        priority: 3,
        payload: normalize_payload(payload),
    }
}

/// Build a context-request event.
///
/// `query_hint` is a short, human-readable label (query text or intent name).
/// Never embed raw output blobs here; reference them by `source_id` instead.
pub fn extract_context_event(query_hint: &str, file_count: usize) -> PendingEvent {
    let payload = json!({
        "query_hint": query_hint,
        "file_count": file_count,
    });
    PendingEvent {
        event_type: SessionEventType::ContextRequest,
        priority: 2,
        payload,
    }
}

/// Build a reasoning-result event.
///
/// `source_id` must reference the saved artifact in the content store so the
/// payload itself stays small.  `summary` must be a short human-readable label.
pub fn extract_reasoning_event(source_id: Option<&str>, summary: &str) -> PendingEvent {
    let payload = normalize_payload(json!({
        "source_id": source_id,
        "summary": summary,
    }));
    PendingEvent {
        event_type: SessionEventType::ReasoningResult,
        priority: 3,
        payload,
    }
}

/// Build a user-intent event.
///
/// `intent` should be a short label, never raw user text.
pub fn extract_user_event(intent: &str) -> PendingEvent {
    PendingEvent {
        event_type: SessionEventType::UserIntent,
        priority: 3,
        payload: json!({ "intent": intent }),
    }
}

/// Build an MCP tool-handler event.
///
/// `status` should be `"start"`, `"ok"`, or `"fail"`.  `extra` carries any
/// structured metadata that fits within the payload budget.
pub fn extract_tool_event(tool_name: &str, status: &str, extra: Value) -> PendingEvent {
    let event_type = if status == "fail" {
        SessionEventType::CommandFail
    } else {
        SessionEventType::ContextRequest
    };
    let payload = normalize_payload(redact_payload(json!({
        "tool": tool_name,
        "status": status,
        "extra": extra,
    })));
    PendingEvent {
        event_type,
        priority: 2,
        payload,
    }
}

/// Record a deliberate decision taken during a session.
///
/// `summary` is a short human-readable label (e.g. "chose approach A over B").
/// `rationale` is an optional one-sentence explanation.
/// Neither field should embed raw output blobs.
pub fn extract_decision_event(summary: &str, rationale: Option<&str>) -> PendingEvent {
    extract_decision_event_with_details(summary, rationale, Value::Null)
}

pub fn extract_decision_event_with_details(
    summary: &str,
    rationale: Option<&str>,
    details: Value,
) -> PendingEvent {
    let mut payload = json!({
        "summary": summary,
        "rationale": rationale,
    });
    if let (Some(payload_obj), Some(details_obj)) = (payload.as_object_mut(), details.as_object()) {
        for (key, value) in details_obj {
            payload_obj.insert(key.clone(), value.clone());
        }
    }
    let payload = normalize_payload(payload);
    PendingEvent {
        event_type: SessionEventType::Decision,
        priority: 4,
        payload,
    }
}

/// Record an active rule or instruction governing agent behaviour.
///
/// `label` uniquely identifies the rule within the session (e.g.
/// `"prefer_composition"`).  Later calls with the same `label` replace the
/// earlier record in the resume snapshot.  `rule` carries the short rule text.
/// `source` is an optional reference to where the rule was loaded from (e.g.
/// a file path or MCP tool result).
pub fn extract_rule_event(label: &str, rule: &str, source: Option<&str>) -> PendingEvent {
    let payload = normalize_payload(json!({
        "label": label,
        "rule": rule,
        "source": source,
    }));
    PendingEvent {
        event_type: SessionEventType::RuleInstruction,
        priority: 4,
        payload,
    }
}

// ---------------------------------------------------------------------------
// Normalize / hash
// ---------------------------------------------------------------------------

/// Construct a `PendingEvent` with explicit fields.
///
/// The payload is normalized to `MAX_EVENT_PAYLOAD_BYTES` before being stored.
pub fn normalize_event(
    event_type: SessionEventType,
    priority: i32,
    payload: Value,
) -> PendingEvent {
    PendingEvent {
        event_type,
        priority,
        payload: normalize_payload(payload),
    }
}

/// Compute a hex-encoded SHA-256 hash of the event identity triple.
///
/// Mirrors the hash computed inside `SessionStore::append_event` so callers
/// can detect duplicates before writing to the store.
pub fn hash_event(event_type: &SessionEventType, priority: i32, payload_json: &str) -> String {
    let mut h = Sha256::new();
    h.update(event_type.as_str().as_bytes());
    h.update(priority.to_le_bytes());
    h.update(payload_json.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Truncate the payload when it would exceed the inline budget.
///
/// Replaces the full payload with a compact marker that includes the original
/// byte count and a brief preview.  Large artifacts must be stored via the
/// content store and referenced by `source_id`.
fn normalize_payload(payload: Value) -> Value {
    let serialized = serde_json::to_string(&payload).unwrap_or_default();
    if serialized.len() <= MAX_EVENT_PAYLOAD_BYTES {
        return payload;
    }
    let preview_len = serialized.len().min(256);
    json!({
        "truncated": true,
        "original_bytes": serialized.len(),
        "preview": &serialized[..preview_len],
    })
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_session::SessionEventType;

    #[test]
    fn extract_cli_event_redacts_env_var_in_extra() {
        let extra = serde_json::json!({
            "PATH": "/usr/bin:/usr/local/bin",
            "HOME": "/root",
            "safe_field": "ok",
        });
        let event = extract_cli_event("atlas build", "ok", extra);
        assert_eq!(event.event_type, SessionEventType::CommandRun);
        // Extra is nested inside payload; env var keys must be redacted.
        assert_eq!(event.payload["extra"]["PATH"], "[REDACTED]");
        assert_eq!(event.payload["extra"]["HOME"], "[REDACTED]");
        assert_eq!(event.payload["extra"]["safe_field"], "ok");
        // Command and status are top-level — not env-var-like keys.
        assert_eq!(event.payload["command"], "atlas build");
        assert_eq!(event.payload["status"], "ok");
    }

    #[test]
    fn extract_cli_event_redacts_secret_keys_in_extra() {
        let extra = serde_json::json!({
            "api_key": "super-secret-key-1234",
            "token": "bearer abc123",
            "message": "no secret here",
        });
        let event = extract_cli_event("deploy", "ok", extra);
        assert_eq!(event.payload["extra"]["api_key"], "[REDACTED]");
        assert_eq!(event.payload["extra"]["token"], "[REDACTED]");
        assert_eq!(event.payload["extra"]["message"], "no secret here");
    }

    #[test]
    fn extract_cli_fail_event_has_correct_type() {
        let event = extract_cli_event("atlas build", "fail", serde_json::Value::Null);
        assert_eq!(event.event_type, SessionEventType::CommandFail);
        assert_eq!(event.priority, 2);
    }

    #[test]
    fn extract_tool_event_redacts_secrets() {
        let extra = serde_json::json!({ "Authorization": "Bearer sk-secret" });
        let event = extract_tool_event("search_saved_context", "ok", extra);
        assert_eq!(event.event_type, SessionEventType::ContextRequest);
        // "Authorization" matches "auth" substring in SECRET_KEY_PATTERNS.
        assert_eq!(event.payload["extra"]["Authorization"], "[REDACTED]");
    }

    #[test]
    fn extract_tool_fail_event() {
        let event = extract_tool_event("get_review_context", "fail", serde_json::Value::Null);
        assert_eq!(event.event_type, SessionEventType::CommandFail);
    }

    #[test]
    fn oversized_payload_is_truncated_not_rejected() {
        // 10 KB of text should be normalized to a truncation marker.
        let big = "x".repeat(10 * 1024);
        let event = extract_cli_event("cmd", "ok", serde_json::json!({ "output": big }));
        // If truncated, the payload must contain the truncation marker.
        let s = serde_json::to_string(&event.payload).unwrap();
        assert!(
            event.payload.get("truncated").is_some() || s.len() <= MAX_EVENT_PAYLOAD_BYTES,
            "payload must be bounded"
        );
    }

    #[test]
    fn hash_event_is_deterministic() {
        let h1 = hash_event(&SessionEventType::CommandRun, 2, r#"{"cmd":"build"}"#);
        let h2 = hash_event(&SessionEventType::CommandRun, 2, r#"{"cmd":"build"}"#);
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_event_differs_on_type_or_priority() {
        let h1 = hash_event(&SessionEventType::CommandRun, 2, r#"{"cmd":"build"}"#);
        let h2 = hash_event(&SessionEventType::CommandFail, 2, r#"{"cmd":"build"}"#);
        let h3 = hash_event(&SessionEventType::CommandRun, 3, r#"{"cmd":"build"}"#);
        assert_ne!(h1, h2);
        assert_ne!(h1, h3);
    }

    #[test]
    fn extract_decision_event_has_correct_type_and_priority() {
        let event = extract_decision_event("prefer composition", Some("simpler design"));
        assert_eq!(event.event_type, SessionEventType::Decision);
        assert_eq!(event.priority, 4);
        assert_eq!(event.payload["summary"], "prefer composition");
        assert_eq!(event.payload["rationale"], "simpler design");
    }

    #[test]
    fn extract_decision_event_with_details_merges_extra_fields() {
        let event = extract_decision_event_with_details(
            "reuse prior context",
            Some("matched prior decision"),
            serde_json::json!({
                "query": "src/lib.rs",
                "source_ids": ["src-1"],
            }),
        );
        assert_eq!(event.payload["summary"], "reuse prior context");
        assert_eq!(event.payload["query"], "src/lib.rs");
        assert_eq!(event.payload["source_ids"][0], "src-1");
    }

    #[test]
    fn extract_rule_event_captures_all_fields() {
        let event = extract_rule_event("no_mut", "avoid global mutation", Some("AGENTS.md"));
        assert_eq!(event.event_type, SessionEventType::RuleInstruction);
        assert_eq!(event.payload["label"], "no_mut");
        assert_eq!(event.payload["rule"], "avoid global mutation");
        assert_eq!(event.payload["source"], "AGENTS.md");
    }

    #[test]
    fn extract_user_event_captures_intent() {
        let event = extract_user_event("review PR");
        assert_eq!(event.event_type, SessionEventType::UserIntent);
        assert_eq!(event.payload["intent"], "review PR");
    }

    #[test]
    fn bind_produces_new_session_event() {
        let session_id = SessionId::derive("/repo", "main", "cli");
        let pending = extract_user_event("refactor");
        let bound = pending.bind(session_id.clone());
        assert_eq!(bound.session_id, session_id);
        assert_eq!(bound.event_type, SessionEventType::UserIntent);
    }
}
