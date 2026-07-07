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

use std::path::Path;

use anyhow::{Context, Result};
use regex::{Captures, RegexBuilder};
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RedactionRules {
    pub secret_key_patterns: Vec<String>,
    pub token_prefixes: Vec<String>,
    pub token_min_len: usize,
    pub hex_secret_min_len: usize,
    pub base64_secret_min_len: usize,
}

impl Default for RedactionRules {
    fn default() -> Self {
        Self {
            secret_key_patterns: SECRET_KEY_PATTERNS
                .iter()
                .map(|v| (*v).to_owned())
                .collect(),
            token_prefixes: TOKEN_PREFIXES.iter().map(|v| (*v).to_owned()).collect(),
            token_min_len: TOKEN_MIN_LEN,
            hex_secret_min_len: HEX_SECRET_MIN_LEN,
            base64_secret_min_len: B64_SECRET_MIN_LEN,
        }
    }
}

impl RedactionRules {
    pub fn validate(&self) -> Result<()> {
        if self.token_min_len == 0 {
            anyhow::bail!("redaction rules: token_min_len must be greater than 0");
        }
        if self.hex_secret_min_len == 0 {
            anyhow::bail!("redaction rules: hex_secret_min_len must be greater than 0");
        }
        if self.base64_secret_min_len == 0 {
            anyhow::bail!("redaction rules: base64_secret_min_len must be greater than 0");
        }
        for (index, value) in self.secret_key_patterns.iter().enumerate() {
            if value.trim().is_empty() {
                anyhow::bail!("redaction rules: secret_key_patterns[{index}] must not be empty");
            }
        }
        for (index, value) in self.token_prefixes.iter().enumerate() {
            if value.trim().is_empty() {
                anyhow::bail!("redaction rules: token_prefixes[{index}] must not be empty");
            }
        }
        Ok(())
    }
}

pub fn load_redaction_rules_file(path: &Path) -> Result<RedactionRules> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read redaction rules file {}", path.display()))?;
    let rules: RedactionRules = toml::from_str(&raw)
        .with_context(|| format!("cannot parse redaction rules file {}", path.display()))?;
    rules
        .validate()
        .with_context(|| format!("invalid redaction rules file {}", path.display()))?;
    Ok(rules)
}

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
fn is_secret_key(key: &str, rules: &RedactionRules) -> bool {
    let lower = key.to_ascii_lowercase();
    rules
        .secret_key_patterns
        .iter()
        .any(|pat| lower.contains(&pat.to_ascii_lowercase()))
}

// ── Value classification ───────────────────────────────────────────────────

/// Returns `true` when `value` looks like a bearer token or opaque secret.
/// Heuristics (applied in order; any match returns true):
/// 1. Starts with a well-known token prefix.
/// 2. Long opaque hex string (≥ configured chars of hex digits only).
/// 3. Long base64-ish string (≥ configured chars, no spaces, only b64 chars).
fn looks_like_token(value: &str, rules: &RedactionRules) -> bool {
    if value.len() < rules.token_min_len {
        return false;
    }
    let lower = value.to_ascii_lowercase();
    if rules
        .token_prefixes
        .iter()
        .any(|prefix| lower.starts_with(&prefix.to_ascii_lowercase()))
    {
        return true;
    }
    if value.len() >= rules.hex_secret_min_len && value.chars().all(|c| c.is_ascii_hexdigit()) {
        return true;
    }
    if value.len() >= rules.base64_secret_min_len
        && !value.contains(' ')
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '-' | '_'))
        && value
            .chars()
            .any(|c| c.is_ascii_digit() || matches!(c, '+' | '/' | '=' | '-' | '_'))
    {
        return true;
    }
    false
}

// ── Recursive redaction ────────────────────────────────────────────────────

fn redact_value(value: Value, rules: &RedactionRules) -> Value {
    match value {
        Value::Object(map) => Value::Object(redact_map(map, rules)),
        Value::Array(arr) => Value::Array(
            arr.into_iter()
                .map(|value| redact_value(value, rules))
                .collect(),
        ),
        Value::String(s) if looks_like_token(&s, rules) => Value::String(REDACTED.to_string()),
        other => other,
    }
}

fn redact_map(map: Map<String, Value>, rules: &RedactionRules) -> Map<String, Value> {
    map.into_iter()
        .map(|(k, v)| {
            if is_env_var_key(&k) || is_secret_key(&k, rules) {
                (k, Value::String(REDACTED.to_string()))
            } else {
                (k, redact_value(v, rules))
            }
        })
        .collect()
}

fn regex_case_insensitive(pattern: &str) -> RegexBuilder {
    let mut builder = RegexBuilder::new(pattern);
    builder.case_insensitive(true);
    builder
}

fn redact_text_key_value_pairs(input: &str, rules: &RedactionRules) -> String {
    if rules.secret_key_patterns.is_empty() {
        return input.to_owned();
    }

    let joined_patterns = rules
        .secret_key_patterns
        .iter()
        .map(|value| regex::escape(value))
        .collect::<Vec<_>>()
        .join("|");
    let pattern = format!(
        r#"(["']?[A-Za-z0-9_.-]*?(?:{joined_patterns})[A-Za-z0-9_.-]*["']?\s*[:=]\s*)(?:bearer\s+[^\s,;]+|"[^"]*"|'[^']*'|[^\s,;]+)"#
    );
    let regex = regex_case_insensitive(&pattern)
        .build()
        .expect("escaped key redaction regex must compile");
    regex
        .replace_all(input, |captures: &Captures<'_>| {
            format!("{}{}", &captures[1], REDACTED)
        })
        .into_owned()
}

fn redact_text_prefixed_tokens(input: &str, rules: &RedactionRules) -> String {
    if rules.token_prefixes.is_empty() {
        return input.to_owned();
    }

    let mut result = input.to_owned();
    let mut prefixes = rules.token_prefixes.clone();
    prefixes.sort_by_key(|prefix| std::cmp::Reverse(prefix.len()));
    for prefix in prefixes {
        let escaped = regex::escape(&prefix);
        let pattern = format!(r"{escaped}[A-Za-z0-9+/=_-]*");
        let regex = regex_case_insensitive(&pattern)
            .build()
            .expect("escaped token prefix regex must compile");
        result = regex.replace_all(&result, REDACTED).into_owned();
    }
    result
}

fn redact_text_hex_tokens(input: &str, rules: &RedactionRules) -> String {
    let pattern = format!(r"\b[0-9A-Fa-f]{{{},}}\b", rules.hex_secret_min_len);
    let regex = RegexBuilder::new(&pattern)
        .build()
        .expect("hex redaction regex must compile");
    regex.replace_all(input, REDACTED).into_owned()
}

fn redact_text_base64_tokens(input: &str, rules: &RedactionRules) -> String {
    let pattern = format!(r"\b[A-Za-z0-9+/=_-]{{{},}}\b", rules.base64_secret_min_len);
    let regex = RegexBuilder::new(&pattern)
        .build()
        .expect("base64 redaction regex must compile");
    regex
        .replace_all(input, |captures: &Captures<'_>| {
            let candidate = captures.get(0).map(|m| m.as_str()).unwrap_or_default();
            if looks_like_token(candidate, rules) {
                REDACTED.to_owned()
            } else {
                candidate.to_owned()
            }
        })
        .into_owned()
}

/// Redact plain text before writing it to searchable stores or inline previews.
///
/// Best-effort scan catches common `key=value`, `key: value`, bearer-token,
/// hex-token, and base64-like secret shapes. Text that does not match these
/// patterns is preserved as-is.
pub fn redact_text_with_rules(input: &str, rules: &RedactionRules) -> String {
    let after_pairs = redact_text_key_value_pairs(input, rules);
    let after_prefixes = redact_text_prefixed_tokens(&after_pairs, rules);
    let after_hex = redact_text_hex_tokens(&after_prefixes, rules);
    redact_text_base64_tokens(&after_hex, rules)
}

pub fn redact_text(input: &str) -> String {
    redact_text_with_rules(input, &RedactionRules::default())
}

/// Redact a JSON payload before writing it to the event ledger.
///
/// Fields matching environment variable naming conventions (ALL_CAPS), known
/// secret key patterns, or string values that look like tokens are replaced
/// with `"[REDACTED]"`. The rest of the payload structure is preserved.
///
/// Large blobs must be intercepted _before_ this function; this redactor
/// works on already-bounded payloads.
pub fn redact_payload_with_rules(payload: Value, rules: &RedactionRules) -> Value {
    redact_value(payload, rules)
}

pub fn redact_payload(payload: Value) -> Value {
    redact_payload_with_rules(payload, &RedactionRules::default())
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

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
        assert!(!is_env_var_key("X"));
        assert!(!is_env_var_key("CamelCase"));
    }

    #[test]
    fn redact_text_redacts_key_value_and_prefixed_tokens() {
        let text =
            "token=abcd1234 password: hunter2 Authorization: Bearer abcdefghijklmnopqrstuvwxyz";
        let out = redact_text(text);
        assert!(out.contains("token=[REDACTED]"));
        assert!(out.contains("password: [REDACTED]"));
        assert!(out.contains("Authorization: [REDACTED]"));
        assert!(!out.contains("abcdefghijklmnopqrstuvwxyz"));
    }

    #[test]
    fn custom_rules_can_redact_new_shapes() {
        let rules = RedactionRules {
            secret_key_patterns: vec!["sessionid".to_owned()],
            token_prefixes: vec!["zz-".to_owned()],
            token_min_len: 3,
            hex_secret_min_len: 32,
            base64_secret_min_len: 40,
        };
        let payload = json!({"sessionId": "keep_me?", "safe": "ok"});
        let out = redact_payload_with_rules(payload, &rules);
        assert_eq!(out["sessionId"], REDACTED);

        let text = "sessionId=abc zz-12345 plain";
        let out = redact_text_with_rules(text, &rules);
        assert!(out.contains("sessionId=[REDACTED]"));
        assert!(out.contains("[REDACTED]"));
        assert!(!out.contains("zz-12345"));
    }

    #[test]
    fn load_redaction_rules_file_reads_valid_toml() {
        let dir = tempdir().expect("tempdir");
        let path = dir.path().join("rules.toml");
        std::fs::write(
            &path,
            "token_prefixes = [\"zz-\"]\nsecret_key_patterns = [\"sessionid\"]\ntoken_min_len = 3\nhex_secret_min_len = 16\nbase64_secret_min_len = 20\n",
        )
        .expect("write rules");

        let rules = load_redaction_rules_file(&path).expect("load rules");
        assert_eq!(rules.token_prefixes, vec!["zz-"]);
        assert_eq!(rules.secret_key_patterns, vec!["sessionid"]);
        assert_eq!(rules.token_min_len, 3);
    }
}
