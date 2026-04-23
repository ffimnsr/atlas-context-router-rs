use std::collections::BTreeSet;
use std::path::Path;

use serde_json::{Map, Value};

use super::policy::{FILE_CHANGED_INLINE_CONTENT_KEYS, HookPolicy};

pub(crate) fn extract_hook_status(payload: &Value) -> Option<String> {
    find_first_string_by_key(payload, &["status", "result"])
        .map(|status| status.to_ascii_lowercase())
}

pub(crate) fn extract_hook_command(payload: &Value) -> Option<String> {
    find_first_string_by_key(
        payload,
        &["command", "cmd", "shell_command", "shellCommand"],
    )
}

pub(crate) fn extract_prompt_text(payload: &Value) -> Option<String> {
    find_first_string_by_key(
        payload,
        &["prompt", "query", "message", "text", "content", "raw"],
    )
}

pub(crate) fn extract_tool_name(payload: &Value) -> Option<String> {
    find_first_string_by_key(payload, &["tool_name", "toolName", "tool"])
}

pub(crate) fn tool_may_change_files(tool_name: &str) -> bool {
    matches!(
        tool_name.to_ascii_lowercase().as_str(),
        "edit" | "write" | "multiedit" | "bash" | "patch"
    )
}

pub(crate) fn extract_changed_files(repo: &str, payload: &Value) -> Vec<String> {
    let mut candidates = Vec::new();
    collect_strings_for_keys(
        payload,
        &[
            "files",
            "paths",
            "changed_files",
            "changedFiles",
            "file",
            "file_path",
            "filePath",
            "path",
            "target_file",
            "targetPath",
        ],
        &mut candidates,
    );

    let mut normalized = BTreeSet::new();
    for candidate in candidates {
        if let Some(path) = normalize_hook_path(repo, &candidate) {
            normalized.insert(path);
        }
    }
    normalized.into_iter().collect()
}

pub(crate) fn collect_source_ids(payload: &Value, source_ids: &mut BTreeSet<String>) {
    let mut candidates = Vec::new();
    collect_strings_for_keys(payload, &["source_id"], &mut candidates);
    for source_id in candidates {
        if !source_id.trim().is_empty() {
            source_ids.insert(source_id);
        }
    }
}

pub(crate) fn find_first_string_by_key(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        Value::Array(values) => values
            .iter()
            .find_map(|nested| find_first_string_by_key(nested, keys)),
        Value::Object(map) => {
            for key in keys {
                if let Some(found) = map.get(*key).and_then(extract_string_value) {
                    return Some(found);
                }
            }
            map.values()
                .find_map(|nested| find_first_string_by_key(nested, keys))
        }
        _ => None,
    }
}

pub(crate) fn collect_strings_for_keys(value: &Value, keys: &[&str], out: &mut Vec<String>) {
    match value {
        Value::Array(values) => {
            for nested in values {
                collect_strings_for_keys(nested, keys, out);
            }
        }
        Value::Object(map) => {
            for key in keys {
                if let Some(value) = map.get(*key) {
                    collect_all_strings(value, out);
                }
            }
            for nested in map.values() {
                collect_strings_for_keys(nested, keys, out);
            }
        }
        _ => {}
    }
}

pub(crate) fn collect_all_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_owned());
            }
        }
        Value::Array(values) => {
            for nested in values {
                collect_all_strings(nested, out);
            }
        }
        Value::Object(map) => {
            for nested in map.values() {
                collect_all_strings(nested, out);
            }
        }
        _ => {}
    }
}

pub(crate) fn extract_string_value(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        Value::Array(values) => values.iter().find_map(extract_string_value),
        Value::Object(map) => [
            "text", "content", "value", "prompt", "query", "message", "path", "file",
        ]
        .iter()
        .find_map(|key| map.get(*key).and_then(extract_string_value)),
        _ => None,
    }
}

pub(crate) fn normalize_hook_path(repo: &str, candidate: &str) -> Option<String> {
    let trimmed = candidate.trim();
    if trimmed.is_empty() || trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return None;
    }

    let repo_path = Path::new(repo);
    let candidate_path = Path::new(trimmed);
    let normalized = if candidate_path.is_absolute() {
        candidate_path
            .strip_prefix(repo_path)
            .ok()?
            .to_string_lossy()
            .into_owned()
    } else {
        trimmed.trim_start_matches("./").replace('\\', "/")
    };

    let normalized = normalized.trim_matches('/').to_owned();
    if normalized.is_empty() || normalized.starts_with(".atlas/") {
        return None;
    }
    Some(normalized)
}

pub(crate) fn strip_inline_file_content(value: Value) -> Value {
    match value {
        Value::Array(values) => {
            Value::Array(values.into_iter().map(strip_inline_file_content).collect())
        }
        Value::Object(map) => {
            let mut sanitized = Map::new();
            for (key, value) in map {
                if FILE_CHANGED_INLINE_CONTENT_KEYS.contains(&key.as_str()) {
                    continue;
                }
                sanitized.insert(key, strip_inline_file_content(value));
            }
            Value::Object(sanitized)
        }
        other => other,
    }
}

pub(crate) fn sanitize_payload_for_storage(policy: &HookPolicy, payload: Value) -> Value {
    if policy.canonical_event == "file-changed" {
        strip_inline_file_content(payload)
    } else {
        payload
    }
}
