use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use atlas_core::{AtlasError, Result};
use atlas_store_markdown::Frontmatter;
use atlas_store_markdown::MarkdownRecord;
use atlas_store_markdown::Value;
use atlas_store_markdown::slug::slugify;
use serde_json::Value as JsonValue;

use super::MEMORIES_DIR;
use super::MEMORY_LINK_RELATIONS;
use super::MemoryLink;
use super::MemoryRecord;

// ── Paths ─────────────────────────────────────────────────────────────────────

/// Relative path for a record: `memories/<topic-slug>/<id>.md`.
pub(crate) fn relative_path(topic: &str, id: &str) -> PathBuf {
    Path::new(MEMORIES_DIR)
        .join(slugify(topic))
        .join(format!("{id}.md"))
}

// ── Encoding ──────────────────────────────────────────────────────────────────

/// Serialize a memory record into canonical markdown (frontmatter + body).
pub(crate) fn encode_record(record: &MemoryRecord) -> Result<MarkdownRecord> {
    let mut frontmatter = Frontmatter::new();
    frontmatter.set("id", Value::str(record.id.clone()));
    frontmatter.set("title", Value::str(record.title.clone()));
    frontmatter.set("topic", Value::str(record.topic.clone()));
    frontmatter.set("scope", Value::str(record.scope.as_str()));
    frontmatter.set("importance", Value::str(record.importance.as_str()));
    frontmatter.set(
        "tags",
        Value::list(record.tags.iter().cloned().map(Value::Str)),
    );
    frontmatter.set("session_id", optional_string(record.session_id.as_deref()));
    frontmatter.set("frontend", optional_string(record.frontend.as_deref()));
    frontmatter.set("created_at", Value::str(record.created_at.clone()));
    frontmatter.set("updated_at", Value::str(record.updated_at.clone()));
    frontmatter.set("source_id", optional_string(record.source_id.as_deref()));
    if record.links.is_empty() {
        frontmatter.set("links", Value::List(Vec::new()));
    } else {
        frontmatter.set(
            "links",
            Value::List(
                record
                    .links
                    .iter()
                    .map(|link| {
                        Value::Map(vec![
                            ("id".to_owned(), Value::Str(link.id.clone())),
                            ("relation".to_owned(), Value::Str(link.relation.clone())),
                        ])
                    })
                    .collect(),
            ),
        );
    }

    match metadata_as_flat_map(&record.metadata) {
        Some(map) => frontmatter.set("metadata", Value::Map(map)),
        None => {
            let json =
                serde_json::to_string(&public_metadata(&record.metadata)).map_err(|error| {
                    AtlasError::Other(format!("serialize memory metadata: {error}"))
                })?;
            frontmatter.set("metadata_json", Value::Str(json));
        }
    }

    if let Some(history) = record.metadata.get("__op_history") {
        let json = serde_json::to_string(history).map_err(|error| {
            AtlasError::Other(format!("serialize memory operator history: {error}"))
        })?;
        frontmatter.set("op_history", Value::Str(json));
    }
    if let Some(numeric) = record
        .metadata
        .get("__numeric_value")
        .and_then(JsonValue::as_f64)
    {
        frontmatter.set("numeric_value", Value::Float(numeric));
    }

    Ok(MarkdownRecord::new(frontmatter, record.body.as_str()))
}

/// Parse a markdown record into a [`MemoryRecord`].
pub(crate) fn decode_record(
    relative: &Path,
    record: &MarkdownRecord,
    repo_root: &str,
) -> Result<MemoryRecord> {
    let frontmatter = &record.frontmatter;
    let id = required_str(frontmatter, "id", relative)?;
    let scope_raw = required_str(frontmatter, "scope", relative)?;
    let importance_raw = required_str(frontmatter, "importance", relative)?;
    let scope = scope_raw.parse().map_err(|error: AtlasError| {
        invalid(relative, format!("invalid scope '{scope_raw}': {error}"))
    })?;
    let importance = importance_raw.parse().map_err(|error: AtlasError| {
        invalid(
            relative,
            format!("invalid importance '{importance_raw}': {error}"),
        )
    })?;
    if scope == super::types::MemoryScope::Session
        && frontmatter
            .get_str("session_id")
            .map(str::trim)
            .is_none_or(str::is_empty)
    {
        return Err(invalid(
            relative,
            "scope 'session' requires a session_id".to_owned(),
        ));
    }
    if scope == super::types::MemoryScope::Frontend
        && frontmatter
            .get_str("frontend")
            .map(str::trim)
            .is_none_or(str::is_empty)
    {
        return Err(invalid(
            relative,
            "scope 'frontend' requires a frontend".to_owned(),
        ));
    }

    let tags = dedupe_tags(
        frontmatter
            .get_list("tags")
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
    );
    let mut links = Vec::new();
    for entry in frontmatter.get_list("links").unwrap_or_default() {
        let Some(map) = entry.as_map() else {
            return Err(invalid(relative, "links entries must be maps".to_owned()));
        };
        let mut link_id = None;
        let mut relation = None;
        for (key, value) in map {
            match key.as_str() {
                "id" => link_id = value.as_str().map(str::to_owned),
                "relation" => relation = value.as_str().map(str::to_owned),
                _ => {}
            }
        }
        let (Some(link_id), Some(relation)) = (link_id, relation) else {
            return Err(invalid(
                relative,
                "links entries require id and relation".to_owned(),
            ));
        };
        if !MEMORY_LINK_RELATIONS.contains(&relation.as_str()) {
            return Err(invalid(
                relative,
                format!("invalid link relation '{relation}'"),
            ));
        }
        links.push(MemoryLink {
            id: link_id,
            relation,
        });
    }
    links.sort_by(|left, right| {
        left.id
            .cmp(&right.id)
            .then_with(|| left.relation.cmp(&right.relation))
    });
    links.dedup();

    let metadata = match frontmatter.get_str("metadata_json") {
        Some(raw) => serde_json::from_str(raw)
            .map_err(|error| invalid(relative, format!("malformed metadata_json: {error}")))?,
        None => metadata_from_map(frontmatter.get("metadata")),
    };
    let mut metadata = metadata;
    if let Some(raw) = frontmatter.get_str("op_history") {
        let history: JsonValue = serde_json::from_str(raw)
            .map_err(|error| invalid(relative, format!("malformed op_history: {error}")))?;
        if let Some(object) = metadata.as_object_mut() {
            object.insert("__op_history".to_owned(), history);
        }
    }
    if let Some(numeric) = frontmatter.get("numeric_value") {
        let numeric = match numeric {
            Value::Float(value) => Some(*value),
            Value::Int(value) => Some(*value as f64),
            _ => None,
        };
        if let Some(numeric) = numeric
            && let Some(object) = metadata.as_object_mut()
        {
            object.insert("__numeric_value".to_owned(), serde_json::json!(numeric));
        }
    }

    Ok(MemoryRecord {
        id,
        repo_root: repo_root.to_owned(),
        session_id: frontmatter.get_str("session_id").map(str::to_owned),
        frontend: frontmatter.get_str("frontend").map(str::to_owned),
        scope,
        topic: frontmatter.get_str("topic").unwrap_or_default().to_owned(),
        title: frontmatter.get_str("title").unwrap_or_default().to_owned(),
        body: record.body.clone(),
        importance,
        created_at: frontmatter
            .get_str("created_at")
            .unwrap_or_default()
            .to_owned(),
        updated_at: frontmatter
            .get_str("updated_at")
            .unwrap_or_default()
            .to_owned(),
        last_accessed_at: frontmatter
            .get_str("created_at")
            .unwrap_or_default()
            .to_owned(),
        decay_score: 0.0,
        source_id: frontmatter.get_str("source_id").map(str::to_owned),
        metadata,
        tags,
        links,
        superseded_by: None,
    })
}

fn optional_string(value: Option<&str>) -> Value {
    value.map_or(Value::Null, Value::str)
}

/// Strip internal (`__`-prefixed) bookkeeping keys before serializing durable
/// metadata; operator history and numeric state have dedicated fields.
pub(crate) fn public_metadata(metadata: &JsonValue) -> JsonValue {
    match metadata {
        JsonValue::Object(object) => JsonValue::Object(
            object
                .iter()
                .filter(|(key, _)| !key.starts_with("__"))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn required_str(frontmatter: &Frontmatter, key: &str, relative: &Path) -> Result<String> {
    frontmatter
        .get_str(key)
        .map(str::to_owned)
        .ok_or_else(|| invalid(relative, format!("missing required field '{key}'")))
}

fn invalid(relative: &Path, message: String) -> AtlasError {
    AtlasError::Other(format!(
        "memory frontmatter error at {}: {message}",
        relative.display()
    ))
}

fn metadata_as_flat_map(value: &JsonValue) -> Option<Vec<(String, Value)>> {
    let object = value.as_object()?;
    let mut out = Vec::with_capacity(object.len());
    for (key, value) in object {
        if key.starts_with("__") {
            continue;
        }
        let scalar = match value {
            JsonValue::Null => Value::Null,
            JsonValue::Bool(value) => Value::Bool(*value),
            JsonValue::Number(number) => match number.as_i64() {
                Some(value) => Value::Int(value),
                None => Value::Float(number.as_f64()?),
            },
            JsonValue::String(value) => Value::Str(value.clone()),
            JsonValue::Array(_) | JsonValue::Object(_) => return None,
        };
        out.push((key.clone(), scalar));
    }
    Some(out)
}

fn metadata_from_map(value: Option<&Value>) -> JsonValue {
    let Some(Value::Map(entries)) = value else {
        return JsonValue::Object(Default::default());
    };
    let mut object = serde_json::Map::new();
    for (key, value) in entries {
        let scalar = match value {
            Value::Null => JsonValue::Null,
            Value::Bool(value) => JsonValue::Bool(*value),
            Value::Int(value) => JsonValue::Number((*value).into()),
            Value::Float(value) => serde_json::Number::from_f64(*value)
                .map(JsonValue::Number)
                .unwrap_or(JsonValue::Null),
            Value::Str(value) => JsonValue::String(value.clone()),
            Value::List(_) | Value::Map(_) => continue,
        };
        object.insert(key.clone(), scalar);
    }
    JsonValue::Object(object)
}

pub(crate) fn dedupe_tags(tags: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for tag in tags {
        let tag = tag.trim().to_owned();
        if tag.is_empty() {
            continue;
        }
        if seen.insert(tag.to_lowercase()) {
            out.push(tag);
        }
    }
    out
}

pub(crate) fn strip_internal_metadata(metadata: &mut JsonValue) {
    if let Some(object) = metadata.as_object_mut() {
        object.retain(|key, _| !key.starts_with("__"));
    }
}

pub(crate) fn markdown_error(error: atlas_store_markdown::MarkdownError) -> AtlasError {
    AtlasError::Other(format!("markdown store error: {error}"))
}
