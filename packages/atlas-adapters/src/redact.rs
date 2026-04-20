//! Payload redaction helpers for Atlas adapter event extraction.
//!
//! Strips environment variables, secret-pattern fields, and token-like string
//! values from structured JSON payloads **before** they are written to the
//! session event ledger or the content store index.
//!
//! Design constraints:
//! - Never log or return redacted values.
//! - Use allowlist-style key matching; err on the side of redacting more.
//! - Recursive so nested objects are also cleaned.

use serde_json::{Map, Value};

/// Sentinel string that replaces redacted values.
const REDACTED: &str = "[REDACTED]";

/// Minimum length for a string value to be tested as a token.
const TOKEN_MIN_LEN: usize = 16;
/// Minimum length for a hex string to be considered a secret.
const HEX_SECRET_MIN_LEN: usize = 32;
/// Minimum length for a base64-ish string to be considered a secret.
const B64_SECRET_MIN_LEN: usize = 40;

/// Key fragments that indicate a secret-bearing field (compared lowercase).
const SECRET_KEY_PATTERNS: &[&str] = &[
    "token",
    "secret",
    "password",
    "passwd",
    "credential",
    "api_key",
    "apikey",
    "auth",
    "authorization",
    "private_key",
    "access_key",
    "bearer",
    "refresh_token",
    "session_token",
    "client_secret",
];

/// Well-known token prefixes (checked lowercase).
const TOKEN_PREFIXES: &[&str] = &[
    "bearer ", "sk-",   // OpenAI secret key
    "ghp_",  // GitHub personal access token
    "gho_",  // GitHub OAuth token
    "ghs_",  // GitHub server-to-server token
    "xoxb-", // Slack bot token
    "xoxp-", // Slack user token
];

// ── Key classification ─────────────────────────────────────────────────────

/// Returns `true` when `key` looks like a shell environment variable name:
/// all uppercase ASCII letters, digits, or underscores, length ≥ 2.
fn is_env_var_key(key: &str) -> bool {
    key.len() >= 2
        && key
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Returns `true` when `key` (case-insensitive) matches a known secret field.
fn is_secret_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SECRET_KEY_PATTERNS.iter().any(|&pat| lower.contains(pat))
}

// ── Value classification ───────────────────────────────────────────────────

/// Returns `true` when `value` looks like a bearer token or opaque secret.
/// Heuristics (applied in order; any match returns true):
/// 1. Starts with a well-known token prefix.
/// 2. Long opaque hex string (≥ `HEX_SECRET_MIN_LEN` chars of hex digits only).
/// 3. Long base64-ish string (≥ `B64_SECRET_MIN_LEN` chars, no spaces, only b64 chars).
fn looks_like_token(value: &str) -> bool {
    if value.len() < TOKEN_MIN_LEN {
        return false;
    }
    let lower = value.to_ascii_lowercase();
    if TOKEN_PREFIXES.iter().any(|&p| lower.starts_with(p)) {
        return true;
    }
    if value.len() >= HEX_SECRET_MIN_LEN && value.chars().all(|c| c.is_ascii_hexdigit()) {
        return true;
    }
    if value.len() >= B64_SECRET_MIN_LEN
        && !value.contains(' ')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '-' | '_'))
    {
        return true;
    }
    false
}

// ── Recursive redaction ────────────────────────────────────────────────────

fn redact_value(value: Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(redact_map(map)),
        Value::Array(arr) => Value::Array(arr.into_iter().map(redact_value).collect()),
        Value::String(s) if looks_like_token(&s) => Value::String(REDACTED.to_string()),
        other => other,
    }
}

fn redact_map(map: Map<String, Value>) -> Map<String, Value> {
    map.into_iter()
        .map(|(k, v)| {
            if is_env_var_key(&k) || is_secret_key(&k) {
                (k, Value::String(REDACTED.to_string()))
            } else {
                (k, redact_value(v))
            }
        })
        .collect()
}

/// Redact a JSON payload before writing it to the event ledger.
///
/// Fields matching environment variable naming conventions (ALL_CAPS), known
/// secret key patterns, or string values that look like tokens are replaced
/// with `"[REDACTED]"`.  The rest of the payload structure is preserved.
///
/// Large blobs must be intercepted _before_ this function; this redactor
/// works on already-bounded payloads.
pub fn redact_payload(payload: Value) -> Value {
    redact_value(payload)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn env_var_keys_are_redacted() {
        let payload = json!({ "PATH": "/usr/bin", "HOME": "/root", "cmd": "build" });
        let out = redact_payload(payload);
        assert_eq!(out["PATH"], REDACTED, "env var key should be redacted");
        assert_eq!(out["HOME"], REDACTED, "env var key should be redacted");
        assert_eq!(out["cmd"], "build", "non-env key must be preserved");
    }

    #[test]
    fn secret_keys_are_redacted() {
        let payload = json!({
            "token": "abc123",
            "password": "hunter2",
            "api_key": "sk-secret",
            "command": "deploy",
        });
        let out = redact_payload(payload);
        assert_eq!(out["token"], REDACTED);
        assert_eq!(out["password"], REDACTED);
        assert_eq!(out["api_key"], REDACTED);
        assert_eq!(out["command"], "deploy", "unrelated key preserved");
    }

    #[test]
    fn token_prefixed_values_are_redacted() {
        let payload = json!({
            "header": "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9",
            "key": "sk-abcdefghij1234567890",
            "label": "normal text value",
        });
        let out = redact_payload(payload);
        assert_eq!(out["header"], REDACTED);
        assert_eq!(out["key"], REDACTED);
        assert_eq!(out["label"], "normal text value");
    }

    #[test]
    fn hex_secret_values_are_redacted() {
        // 64-char hex string → looks like a secret
        let hex = "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2";
        let payload = json!({ "value": hex, "short_hex": "deadbeef" });
        let out = redact_payload(payload);
        assert_eq!(out["value"], REDACTED, "long hex should be redacted");
        assert_eq!(
            out["short_hex"], "deadbeef",
            "short hex should be preserved"
        );
    }

    #[test]
    fn nested_secrets_are_redacted() {
        let payload = json!({
            "env": {
                "SECRET_KEY": "super_secret",
                "HOME": "/root",
                "normal_key": "ok-value",
            }
        });
        let out = redact_payload(payload);
        assert_eq!(out["env"]["SECRET_KEY"], REDACTED);
        assert_eq!(out["env"]["HOME"], REDACTED);
        assert_eq!(out["env"]["normal_key"], "ok-value");
    }

    #[test]
    fn arrays_are_recursively_scanned() {
        let token = "ghp_1234567890abcdefghijklmnopqrstuvwx";
        let payload = json!({ "args": ["safe-arg", token, "another-safe"] });
        let out = redact_payload(payload);
        let args = out["args"].as_array().unwrap();
        assert_eq!(args[0], "safe-arg");
        assert_eq!(args[1], REDACTED);
        assert_eq!(args[2], "another-safe");
    }

    #[test]
    fn short_strings_are_not_redacted_as_tokens() {
        let payload = json!({ "status": "ok", "code": "abc" });
        let out = redact_payload(payload);
        assert_eq!(out["status"], "ok");
        assert_eq!(out["code"], "abc");
    }

    #[test]
    fn is_env_var_key_classification() {
        assert!(is_env_var_key("HOME"));
        assert!(is_env_var_key("PATH"));
        assert!(is_env_var_key("AWS_SECRET_ACCESS_KEY"));
        assert!(!is_env_var_key("command"));
        assert!(!is_env_var_key("X")); // length < 2
        assert!(!is_env_var_key("CamelCase"));
    }
}
