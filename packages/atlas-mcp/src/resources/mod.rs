//! MCP resources: static docs/health resources plus the canonical `atlas://`
//! URI namespace.

use anyhow::{Result, anyhow};
use serde::Serialize;
use serde_json::{Value, json};

use crate::spec;

mod docs;
mod registry;
mod uris;

#[cfg(test)]
mod tests;

pub(crate) use docs::docs_completion_items;
pub(crate) use registry::{resource_descriptors, resource_template_descriptors};
pub use uris::{
    grep_uri as atlas_uri_grep, list_uri as atlas_uri_list, read_uri as atlas_uri_read,
    search_uri as atlas_uri_search,
};
const DEFAULT_PAGE_LIMIT: usize = 50;
const RESOURCES_LIST_CACHE_TTL_MS: u64 = 300_000;
const RESOURCES_LIST_CACHE_SCOPE: &str = spec::CACHE_SCOPE_PUBLIC;
const RESOURCE_TEMPLATES_LIST_CACHE_TTL_MS: u64 = 300_000;
const RESOURCE_TEMPLATES_LIST_CACHE_SCOPE: &str = spec::CACHE_SCOPE_PUBLIC;
const RESOURCES_READ_CACHE_TTL_MS: u64 = 60_000;
const RESOURCES_READ_CACHE_SCOPE: &str = spec::CACHE_SCOPE_PRIVATE;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResourceContentsEnvelope {
    contents: Vec<ResourceContent>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResourceContent {
    uri: String,
    mime_type: String,
    text: String,
}

pub(crate) fn resources_list(args: Option<&Value>) -> Result<Value> {
    let resources = resource_descriptors();
    let (slice, next_cursor) = paginate(&resources, args)?;
    let mut result = serde_json::json!({
        "resources": slice,
    });
    if let Some(next_cursor) = next_cursor
        && let Some(object) = result.as_object_mut()
    {
        object.insert("nextCursor".to_owned(), Value::String(next_cursor));
    }
    spec::annotate_cacheable_result(
        &mut result,
        RESOURCES_LIST_CACHE_TTL_MS,
        RESOURCES_LIST_CACHE_SCOPE,
    );
    Ok(result)
}

pub(crate) fn resources_templates_list(args: Option<&Value>) -> Result<Value> {
    let templates = resource_template_descriptors();
    let (slice, next_cursor) = paginate(&templates, args)?;
    let mut result = serde_json::json!({
        "resourceTemplates": slice,
    });
    if let Some(next_cursor) = next_cursor
        && let Some(object) = result.as_object_mut()
    {
        object.insert("nextCursor".to_owned(), Value::String(next_cursor));
    }
    spec::annotate_cacheable_result(
        &mut result,
        RESOURCE_TEMPLATES_LIST_CACHE_TTL_MS,
        RESOURCE_TEMPLATES_LIST_CACHE_SCOPE,
    );
    Ok(result)
}

pub(crate) fn resources_read(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
) -> Result<Value> {
    let uri = args
        .and_then(|value| value.get("uri"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing resource uri"))?;
    let level = args
        .and_then(|value| value.get("level"))
        .and_then(Value::as_str);

    let content = if uri == "atlas://docs/index" {
        ResourceContent {
            uri: uri.to_owned(),
            mime_type: "text/markdown".to_owned(),
            text: docs::render_docs_index_markdown()?,
        }
    } else if uri == "atlas://health/status" {
        json_resource_content(
            uri,
            "application/json",
            extract_structured_or_whole(&crate::tools::call(
                "status",
                Some(&json!({})),
                repo_root,
                db_path,
            )?)?,
        )?
    } else if uri == "atlas://graph/provenance" {
        let status = crate::tools::call("status", Some(&json!({})), repo_root, db_path)?;
        let provenance = status
            .get("atlas_provenance")
            .cloned()
            .ok_or_else(|| anyhow!("status response missing atlas_provenance"))?;
        json_resource_content(uri, "application/json", provenance)?
    } else if let Some(source_id) = uri.strip_prefix("atlas://saved-context/") {
        let response = crate::tools::call(
            "read_saved_context",
            Some(&json!({
                "source_id": source_id
            })),
            repo_root,
            db_path,
        )?;
        let raw = extract_structured_or_whole(&response)?;
        if raw.get("found").and_then(Value::as_bool) == Some(true) {
            ResourceContent {
                uri: uri.to_owned(),
                mime_type: "text/plain".to_owned(),
                text: raw
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            }
        } else {
            json_resource_content(uri, "application/json", raw)?
        }
    } else if let Some(name) = uri.strip_prefix("atlas://tool-docs/") {
        if name.trim().is_empty() {
            return Err(anyhow!(
                "tool docs resource uri must include non-empty tool name"
            ));
        }
        let document = crate::tool_manual("mcp", name)?;
        ResourceContent {
            uri: uri.to_owned(),
            mime_type: "text/markdown".to_owned(),
            text: docs::render_tool_manual_markdown(&document),
        }
    } else if let Some(name) = uri.strip_prefix("atlas://prompt-docs/") {
        if name.trim().is_empty() {
            return Err(anyhow!(
                "prompt docs resource uri must include non-empty prompt name"
            ));
        }
        let rendered = crate::prompts::prompt_get(name, None)?;
        ResourceContent {
            uri: uri.to_owned(),
            mime_type: "text/markdown".to_owned(),
            text: docs::render_prompt_markdown(name, &rendered),
        }
    } else if let Some(path_with_heading) = uri.strip_prefix("atlas://docs/") {
        let (file, heading) = docs::split_docs_uri(path_with_heading)?;
        // URI segments are percent-encoded; decode both the path and heading
        // so `%20` and other escapes resolve to real documentation targets.
        let file = file
            .split('/')
            .map(uris::decode_segment)
            .collect::<Result<Vec<_>>>()?
            .join("/");
        let heading = uris::decode_segment(heading)?;
        let response = crate::tools::call(
            "get_docs_section",
            Some(&json!({
                "file": file,
                "selector": { "kind": "heading", "heading": heading }
            })),
            repo_root,
            db_path,
        )?;
        let raw = extract_structured_or_whole(&response)?;
        if raw.get("resolved").and_then(Value::as_bool) == Some(true) {
            ResourceContent {
                uri: uri.to_owned(),
                mime_type: "text/markdown".to_owned(),
                text: raw
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            }
        } else {
            json_resource_content(uri, "application/json", raw)?
        }
    } else {
        let (mime_type, text) = uris::read_uri(uri, level, repo_root, db_path)?;
        ResourceContent {
            uri: uri.to_owned(),
            mime_type,
            text,
        }
    };

    let mut result = serde_json::to_value(ResourceContentsEnvelope {
        contents: vec![content],
    })?;
    spec::annotate_cacheable_result(
        &mut result,
        RESOURCES_READ_CACHE_TTL_MS,
        RESOURCES_READ_CACHE_SCOPE,
    );
    Ok(result)
}

fn paginate<T: Clone>(items: &[T], args: Option<&Value>) -> Result<(Vec<T>, Option<String>)> {
    let limit = args
        .and_then(|value| value.get("limit"))
        .and_then(Value::as_u64)
        .map(|value| value as usize)
        .unwrap_or(DEFAULT_PAGE_LIMIT)
        .max(1);
    let start = args
        .and_then(|value| value.get("cursor"))
        .map(parse_cursor)
        .transpose()?
        .unwrap_or(0);
    let end = start.saturating_add(limit).min(items.len());
    let next_cursor = (end < items.len()).then(|| format!("offset:{end}"));
    Ok((items[start..end].to_vec(), next_cursor))
}

fn parse_cursor(value: &Value) -> Result<usize> {
    let raw = value
        .as_str()
        .ok_or_else(|| anyhow!("cursor must be string"))?;
    let offset = raw
        .strip_prefix("offset:")
        .ok_or_else(|| anyhow!("invalid cursor '{raw}'"))?
        .parse::<usize>()
        .map_err(|error| anyhow!("invalid cursor '{raw}': {error}"))?;
    Ok(offset)
}

fn extract_structured_or_whole(response: &Value) -> Result<Value> {
    crate::tool_result::structured_content(response)
        .cloned()
        .or_else(|| response.get("result").cloned())
        .or_else(|| response.as_object().map(|_| response.clone()))
        .ok_or_else(|| anyhow!("response missing structured content"))
}

fn json_resource_content(uri: &str, mime_type: &str, value: Value) -> Result<ResourceContent> {
    Ok(ResourceContent {
        uri: uri.to_owned(),
        mime_type: mime_type.to_owned(),
        text: serde_json::to_string_pretty(&value)?,
    })
}
