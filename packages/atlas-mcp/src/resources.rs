use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use serde::Serialize;
use serde_json::{Value, json};

use crate::descriptors::{
    ResourceDescriptor, ResourceTemplateDescriptor, descriptor_meta, human_title,
    validate_descriptor_name,
};
use crate::prompts::{prompt_descriptors, prompt_get};
use crate::tool_result::structured_content;
use crate::{tool_list, tool_manual};

const DEFAULT_PAGE_LIMIT: usize = 50;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResourceRegistry {
    resources: Vec<ResourceDescriptor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ResourceTemplateRegistry {
    resource_templates: Vec<ResourceTemplateDescriptor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_cursor: Option<String>,
}

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
    serde_json::to_value(ResourceRegistry {
        resources: slice,
        next_cursor,
    })
    .map_err(Into::into)
}

pub(crate) fn resources_templates_list(args: Option<&Value>) -> Result<Value> {
    let templates = resource_template_descriptors();
    let (slice, next_cursor) = paginate(&templates, args)?;
    serde_json::to_value(ResourceTemplateRegistry {
        resource_templates: slice,
        next_cursor,
    })
    .map_err(Into::into)
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

    let content = if uri == "atlas://docs/index" {
        ResourceContent {
            uri: uri.to_owned(),
            mime_type: "text/markdown".to_owned(),
            text: render_docs_index_markdown()?,
        }
    } else if uri == "atlas://health/status" {
        json_resource_content(
            uri,
            "application/json",
            extract_structured_or_whole(&crate::tools::call(
                "status",
                Some(&json!({"output_format": "json"})),
                repo_root,
                db_path,
            )?)?,
        )?
    } else if uri == "atlas://graph/provenance" {
        let status = crate::tools::call(
            "status",
            Some(&json!({"output_format": "json"})),
            repo_root,
            db_path,
        )?;
        let provenance = status
            .get("atlas_provenance")
            .cloned()
            .ok_or_else(|| anyhow!("status response missing atlas_provenance"))?;
        json_resource_content(uri, "application/json", provenance)?
    } else if let Some(source_id) = uri.strip_prefix("atlas://saved-context/") {
        let response = crate::tools::call(
            "read_saved_context",
            Some(&json!({
                "source_id": source_id,
                "output_format": "json"
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
        let document = tool_manual("mcp", name)?;
        ResourceContent {
            uri: uri.to_owned(),
            mime_type: "text/markdown".to_owned(),
            text: render_tool_manual_markdown(&document),
        }
    } else if let Some(name) = uri.strip_prefix("atlas://prompt-docs/") {
        if name.trim().is_empty() {
            return Err(anyhow!(
                "prompt docs resource uri must include non-empty prompt name"
            ));
        }
        let rendered = prompt_get(name, None)?;
        ResourceContent {
            uri: uri.to_owned(),
            mime_type: "text/markdown".to_owned(),
            text: render_prompt_markdown(name, &rendered),
        }
    } else if let Some(path_with_heading) = uri.strip_prefix("atlas://docs/") {
        let (file, heading) = split_docs_uri(path_with_heading)?;
        let response = crate::tools::call(
            "get_docs_section",
            Some(&json!({
                "file": file,
                "heading": heading,
                "output_format": "json"
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
        return Err(anyhow!("unknown resource uri: {uri}"));
    };

    serde_json::to_value(ResourceContentsEnvelope {
        contents: vec![content],
    })
    .map_err(Into::into)
}

pub(crate) fn resource_descriptors() -> Vec<ResourceDescriptor> {
    let mut resources = vec![
        ResourceDescriptor {
            uri: "atlas://docs/index".to_owned(),
            name: "docs_index".to_owned(),
            title: human_title("docs_index"),
            description: "Atlas docs index with resource usage, tool discovery flow, and per-tool docs links."
                .to_owned(),
            mime_type: "text/markdown".to_owned(),
            icons: Vec::new(),
            meta: descriptor_meta("resource", "content"),
        },
        ResourceDescriptor {
            uri: "atlas://graph/provenance".to_owned(),
            name: "graph_provenance".to_owned(),
            title: human_title("graph_provenance"),
            description: "Atlas graph provenance metadata for current repo and DB.".to_owned(),
            mime_type: "application/json".to_owned(),
            icons: Vec::new(),
            meta: descriptor_meta("resource", "health"),
        },
        ResourceDescriptor {
            uri: "atlas://health/status".to_owned(),
            name: "health_status".to_owned(),
            title: human_title("health_status"),
            description: "Compact Atlas graph health summary for current repo.".to_owned(),
            mime_type: "application/json".to_owned(),
            icons: Vec::new(),
            meta: descriptor_meta("resource", "health"),
        },
    ];
    resources.sort_by(|left, right| left.uri.cmp(&right.uri));
    for resource in &resources {
        validate_descriptor_name(&resource.name).expect("resource name must satisfy MCP guidance");
    }
    resources
}

pub(crate) fn resource_template_descriptors() -> Vec<ResourceTemplateDescriptor> {
    let mut templates = vec![
        ResourceTemplateDescriptor {
            uri_template: "atlas://docs/{file}#{heading}".to_owned(),
            name: "docs_section".to_owned(),
            title: human_title("docs_section"),
            description: "Read Markdown docs section by repo-relative file and heading path/slug."
                .to_owned(),
            mime_type: "text/markdown".to_owned(),
            icons: Vec::new(),
            meta: json!({
                "atlas:descriptorKind": "resource_template",
                "atlas:category": "content",
                "atlas:variables": [
                    {"name": "file", "description": "Repo-relative Markdown path."},
                    {"name": "heading", "description": "Heading path or slug."}
                ]
            }),
        },
        ResourceTemplateDescriptor {
            uri_template: "atlas://saved-context/{source_id}".to_owned(),
            name: "saved_context".to_owned(),
            title: human_title("saved_context"),
            description: "Read saved artifact content by source_id.".to_owned(),
            mime_type: "text/plain".to_owned(),
            icons: Vec::new(),
            meta: json!({
                "atlas:descriptorKind": "resource_template",
                "atlas:category": "memory",
                "atlas:variables": [
                    {"name": "source_id", "description": "Saved artifact source identifier."}
                ]
            }),
        },
        ResourceTemplateDescriptor {
            uri_template: "atlas://tool-docs/{name}".to_owned(),
            name: "tool_docs".to_owned(),
            title: human_title("tool_docs"),
            description: "Read generated Markdown docs, examples, and usage for one exported MCP tool by exact name."
                .to_owned(),
            mime_type: "text/markdown".to_owned(),
            icons: Vec::new(),
            meta: json!({
                "atlas:descriptorKind": "resource_template",
                "atlas:category": "content",
                "atlas:variables": [
                    {"name": "name", "description": "Exact exported MCP tool name."}
                ]
            }),
        },
        ResourceTemplateDescriptor {
            uri_template: "atlas://prompt-docs/{name}".to_owned(),
            name: "prompt_docs".to_owned(),
            title: human_title("prompt_docs"),
            description: "Read generated Markdown docs and default prompt body for one exported MCP prompt by exact name."
                .to_owned(),
            mime_type: "text/markdown".to_owned(),
            icons: Vec::new(),
            meta: json!({
                "atlas:descriptorKind": "resource_template",
                "atlas:category": "content",
                "atlas:variables": [
                    {"name": "name", "description": "Exact exported MCP prompt name."}
                ]
            }),
        },
    ];
    templates.sort_by(|left, right| left.uri_template.cmp(&right.uri_template));
    for template in &templates {
        validate_descriptor_name(&template.name).expect("template name must satisfy MCP guidance");
    }
    templates
}

pub(crate) fn docs_completion_items(
    repo_root: &str,
    variable: &str,
    prefix: &str,
    context: Option<&Value>,
) -> Result<Vec<String>> {
    match variable {
        "file" => markdown_file_candidates(repo_root, prefix),
        "heading" => {
            let Some(file) = docs_context_file(context) else {
                return Ok(Vec::new());
            };
            markdown_heading_candidates(repo_root, &file, prefix)
        }
        _ => Ok(Vec::new()),
    }
}

fn markdown_file_candidates(repo_root: &str, prefix: &str) -> Result<Vec<String>> {
    let mut matches = Vec::new();
    let root = Path::new(repo_root);
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(error) => {
                if path == root {
                    return Err(error)
                        .with_context(|| format!("cannot read repo root {repo_root}"));
                }
                continue;
            }
        };
        for entry in entries.flatten() {
            let candidate = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == ".git" || name == "target" || name == "node_modules" {
                continue;
            }
            if candidate.is_dir() {
                stack.push(candidate);
                continue;
            }
            if candidate.extension().and_then(|ext| ext.to_str()) != Some("md") {
                continue;
            }
            if let Ok(relative) = candidate.strip_prefix(root) {
                let value = relative.to_string_lossy().replace('\\', "/");
                if prefix.is_empty() || value.starts_with(prefix) {
                    matches.push(value);
                }
            }
        }
    }
    matches.sort();
    matches.dedup();
    Ok(matches)
}

fn markdown_heading_candidates(repo_root: &str, file: &str, prefix: &str) -> Result<Vec<String>> {
    let path = Path::new(repo_root).join(file);
    let contents = fs::read_to_string(&path)
        .with_context(|| format!("cannot read markdown file '{}'", path.display()))?;
    let mut stack: Vec<String> = Vec::new();
    let mut headings = Vec::new();

    for line in contents.lines() {
        let trimmed = line.trim();
        let level = trimmed.chars().take_while(|ch| *ch == '#').count();
        if level == 0 {
            continue;
        }
        let title = trimmed[level..].trim();
        if title.is_empty() {
            continue;
        }
        while stack.len() >= level {
            stack.pop();
        }
        stack.push(slugify_heading(title));
        let path = format!("document.{}", stack.join("."));
        if prefix.is_empty() || path.starts_with(prefix) {
            headings.push(path);
        }
    }

    headings.sort();
    headings.dedup();
    Ok(headings)
}

fn docs_context_file(context: Option<&Value>) -> Option<String> {
    let context = context?;
    context
        .pointer("/arguments/file")
        .and_then(Value::as_str)
        .or_else(|| context.pointer("/variables/file").and_then(Value::as_str))
        .or_else(|| context.get("file").and_then(Value::as_str))
        .map(str::to_owned)
}

fn slugify_heading(title: &str) -> String {
    let mut output = String::new();
    let mut last_dash = false;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            output.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            output.push('-');
            last_dash = true;
        }
    }
    output.trim_matches('-').to_owned()
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

fn split_docs_uri(value: &str) -> Result<(&str, &str)> {
    let (file, heading) = value
        .split_once('#')
        .ok_or_else(|| anyhow!("docs resource uri must include '#heading'"))?;
    if file.trim().is_empty() || heading.trim().is_empty() {
        return Err(anyhow!(
            "docs resource uri must include non-empty file and heading"
        ));
    }
    Ok((file, heading))
}

fn extract_structured_or_whole(response: &Value) -> Result<Value> {
    structured_content(response)
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

fn render_docs_index_markdown() -> Result<String> {
    let tools = tool_list()["tools"]
        .as_array()
        .cloned()
        .ok_or_else(|| anyhow!("tool registry missing tools array"))?;

    let mut lines = vec![
        "# Atlas Docs Index".to_owned(),
        "".to_owned(),
        "Atlas exposes docs through MCP resources so clients can discover usage without relying on injected prompt text.".to_owned(),
        "".to_owned(),
        "## Resource Usage".to_owned(),
        "".to_owned(),
        "- `resources/list`: enumerate static Atlas resources like `atlas://docs/index` and health/provenance resources".to_owned(),
        "- `resources/templates/list`: enumerate dynamic resource templates like `atlas://docs/{file}#{heading}` and `atlas://tool-docs/{name}`".to_owned(),
        "- `resources/read { uri: \"atlas://docs/index\" }`: read this index".to_owned(),
        "- `resources/read { uri: \"atlas://tool-docs/query_graph\" }`: read per-tool docs with examples and usage".to_owned(),
        "- `resources/read { uri: \"atlas://prompt-docs/review_change\" }`: read per-prompt docs and default prompt body".to_owned(),
        "- `resources/read { uri: \"atlas://docs/README.md#document.mcp-tools\" }`: read bounded Markdown sections from repo docs".to_owned(),
        "".to_owned(),
        "## Tool Discovery Flow".to_owned(),
        "".to_owned(),
        "1. Call `tool_list` for compact live inventory.".to_owned(),
        "2. Call `tool_search` when exact tool name is unclear.".to_owned(),
        "3. Call `tool_help` or read `atlas://tool-docs/{name}` for examples and usage.".to_owned(),
        "4. Read `atlas://prompt-docs/{name}` for exported MCP prompt docs and default prompt body.".to_owned(),
        "".to_owned(),
        "## Per-Tool Docs Resources".to_owned(),
        "".to_owned(),
    ];

    for tool in tools {
        let name = tool["name"].as_str().unwrap_or_default();
        let description = tool["description"].as_str().unwrap_or_default();
        lines.push(format!("- `atlas://tool-docs/{name}` — {description}"));
    }

    lines.extend([
        "".to_owned(),
        "## Per-Prompt Docs Resources".to_owned(),
        "".to_owned(),
    ]);
    for prompt in prompt_descriptors() {
        lines.push(format!(
            "- `atlas://prompt-docs/{}` — {}",
            prompt.name, prompt.description
        ));
    }

    Ok(lines.join("\n"))
}

fn render_tool_manual_markdown(document: &crate::tools::ToolManualDocument) -> String {
    let mut lines = vec![
        format!("# Tool Docs: `{}`", document.resolved_tool_name),
        "".to_owned(),
        document.description.clone(),
        "".to_owned(),
        "## Structure".to_owned(),
        "".to_owned(),
        format!("- purpose: {}", document.tool_structure.purpose),
        format!(
            "- operation_name: `{}`",
            document.tool_structure.operation_name
        ),
        format!("- request_shape: {}", document.tool_structure.request_shape),
        format!(
            "- response_shape: {}",
            document.tool_structure.response_shape
        ),
        format!(
            "- result_contract: `{}`",
            document.tool_structure.result_contract
        ),
        format!(
            "- annotations: read_only=`{}`, state_changing=`{}`, destructive=`{}`",
            document.tool_structure.annotations.read_only,
            document.tool_structure.annotations.state_changing,
            document.tool_structure.annotations.destructive
        ),
        "".to_owned(),
        "## Input Args".to_owned(),
        "".to_owned(),
    ];

    if document.input_args.is_empty() {
        lines.push("- none".to_owned());
    } else {
        for field in &document.input_args {
            let mut line = format!(
                "- `{}`: {} ({}) — {}",
                field.name,
                field.field_type,
                if field.required {
                    "required"
                } else {
                    "optional"
                },
                field.description
            );
            if let Some(default_value) = &field.default_value {
                line.push_str(&format!(" default=`{default_value}`"));
            }
            if !field.enum_values.is_empty() {
                line.push_str(&format!(" enum=`{}`", field.enum_values.join(", ")));
            }
            lines.push(line);
        }
    }

    lines.extend([
        "".to_owned(),
        "## Usage".to_owned(),
        "".to_owned(),
        format!("- CLI: `{}`", document.usage.cli),
        format!(
            "- MCP manual call: `{}`",
            document.usage.mcp_manual_tool_call
        ),
        "".to_owned(),
        "### Target Tool Call Examples".to_owned(),
        "".to_owned(),
    ]);
    for example in &document.usage.target_tool_call_examples {
        lines.push(format!("- `{example}`"));
    }

    lines.extend([
        "".to_owned(),
        "## Output".to_owned(),
        "".to_owned(),
        format!(
            "- structured_content_available: `{}`",
            document.output_response.structured_content_available
        ),
        format!(
            "- response_shape: {}",
            document.output_response.response_shape
        ),
        "".to_owned(),
        "## Error Cases".to_owned(),
        "".to_owned(),
    ]);
    for error in &document.error_cases {
        lines.push(format!(
            "- `{}`: {} {}",
            error.code, error.when, error.behavior
        ));
    }

    lines.join("\n")
}

fn render_prompt_markdown(name: &str, rendered: &Value) -> String {
    let descriptor = prompt_descriptors()
        .into_iter()
        .find(|prompt| prompt.name == name)
        .expect("prompt descriptor must exist for rendered prompt docs");
    let description = rendered["description"]
        .as_str()
        .unwrap_or(&descriptor.description);
    let body = rendered["messages"][0]["content"]["text"]
        .as_str()
        .unwrap_or_default();

    let mut lines = vec![
        format!("# Prompt Docs: `{name}`"),
        "".to_owned(),
        description.to_owned(),
        "".to_owned(),
        "## Arguments".to_owned(),
        "".to_owned(),
    ];

    if descriptor.arguments.is_empty() {
        lines.push("- none".to_owned());
    } else {
        for argument in descriptor.arguments {
            lines.push(format!(
                "- `{}` ({}) — {}",
                argument.name,
                if argument.required {
                    "required"
                } else {
                    "optional"
                },
                argument.description
            ));
        }
    }

    lines.extend([
        "".to_owned(),
        "## Usage".to_owned(),
        "".to_owned(),
        format!("- `prompts/get {{ name: \"{name}\" }}`"),
        format!("- `resources/read {{ uri: \"atlas://prompt-docs/{name}\" }}`"),
        "".to_owned(),
        "## Default Prompt Body".to_owned(),
        "".to_owned(),
        "```text".to_owned(),
        body.to_owned(),
        "```".to_owned(),
    ]);

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::{
        docs_completion_items, resource_descriptors, resource_template_descriptors, resources_list,
        resources_read, resources_templates_list,
    };
    use crate::output::OutputFormat;
    use crate::session_tools::tool_save_context_artifact;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn resources_list_is_deterministic_and_paginates() {
        let first = resources_list(Some(&json!({"limit": 1}))).expect("list resources");
        assert_eq!(first["resources"].as_array().expect("resources").len(), 1);
        assert_eq!(first["nextCursor"], json!("offset:1"));

        let second =
            resources_list(Some(&json!({"limit": 1, "cursor": "offset:1"}))).expect("next page");
        assert_eq!(second["resources"].as_array().expect("resources").len(), 1);
        assert_eq!(second["nextCursor"], json!("offset:2"));

        let third =
            resources_list(Some(&json!({"limit": 1, "cursor": "offset:2"}))).expect("third page");
        assert_eq!(third["resources"].as_array().expect("resources").len(), 1);
        assert!(third.get("nextCursor").is_none());
    }

    #[test]
    fn resource_templates_list_is_deterministic_and_paginates() {
        let first = resources_templates_list(Some(&json!({"limit": 1}))).expect("list templates");
        assert_eq!(
            first["resourceTemplates"]
                .as_array()
                .expect("templates")
                .len(),
            1
        );
        assert_eq!(first["nextCursor"], json!("offset:1"));

        let second = resources_templates_list(Some(&json!({"limit": 1, "cursor": "offset:1"})))
            .expect("next template page");
        assert_eq!(
            second["resourceTemplates"]
                .as_array()
                .expect("templates")
                .len(),
            1
        );
        assert_eq!(second["nextCursor"], json!("offset:2"));
    }

    #[test]
    fn resource_descriptors_include_metadata_fields() {
        for resource in resource_descriptors() {
            assert!(!resource.uri.is_empty());
            assert!(!resource.mime_type.is_empty());
            assert!(resource.icons.is_empty());
            assert!(resource.meta.get("atlas:descriptorKind").is_some());
        }
        for template in resource_template_descriptors() {
            assert!(!template.uri_template.is_empty());
            assert!(!template.mime_type.is_empty());
            assert!(template.icons.is_empty());
        }
    }

    #[test]
    fn docs_completion_uses_context_file_for_heading_suggestions() {
        let dir = TempDir::new().expect("tempdir");
        let docs_dir = dir.path().join("wiki");
        fs::create_dir_all(&docs_dir).expect("docs dir");
        fs::write(
            docs_dir.join("guide.md"),
            "# Guide\n## Install\nbody\n## Usage\nmore\n",
        )
        .expect("write guide");

        let headings = docs_completion_items(
            dir.path().to_str().expect("repo root"),
            "heading",
            "document.guide.i",
            Some(&json!({"arguments": {"file": "wiki/guide.md"}})),
        )
        .expect("heading completion");
        assert_eq!(headings, vec!["document.guide.install"]);
    }

    #[test]
    fn resources_read_docs_index_returns_markdown() {
        let dir = TempDir::new().expect("tempdir");
        let repo_root = dir.path().to_str().expect("repo root");
        let db_path = dir
            .path()
            .join("worldtree.db")
            .to_string_lossy()
            .into_owned();

        let result = resources_read(
            Some(&json!({"uri": "atlas://docs/index"})),
            repo_root,
            &db_path,
        )
        .expect("resource read");
        assert_eq!(result["contents"][0]["mimeType"], json!("text/markdown"));
        let text = result["contents"][0]["text"]
            .as_str()
            .expect("markdown text");
        assert!(text.contains("Atlas Docs Index"));
        assert!(text.contains("atlas://tool-docs/query_graph"));
    }

    #[test]
    fn resources_read_health_status_returns_content() {
        let dir = TempDir::new().expect("tempdir");
        let repo_root = dir.path().to_str().expect("repo root");
        let db_path = dir
            .path()
            .join("worldtree.db")
            .to_string_lossy()
            .into_owned();

        let result = resources_read(
            Some(&json!({"uri": "atlas://health/status"})),
            repo_root,
            &db_path,
        )
        .expect("resource read");
        assert_eq!(result["contents"][0]["mimeType"], json!("application/json"));
        assert!(result["contents"][0]["text"].as_str().is_some());
    }

    #[test]
    fn resources_read_saved_context_round_trips_saved_artifact() {
        let dir = TempDir::new().expect("tempdir");
        let repo_root = dir.path().to_str().expect("repo root");
        let db_path = dir
            .path()
            .join("worldtree.db")
            .to_string_lossy()
            .into_owned();

        let payload = std::iter::repeat_n("saved context payload with safe spacing", 30)
            .collect::<Vec<_>>()
            .join(" ");
        let saved = tool_save_context_artifact(
            Some(&json!({
                "label": "artifact",
                "content": payload,
                "output_format": "json"
            })),
            repo_root,
            &db_path,
            OutputFormat::Json,
        )
        .expect("save artifact");
        let source_id = saved["structuredContent"]["source_id"]
            .as_str()
            .expect("source id");

        let result = resources_read(
            Some(&json!({"uri": format!("atlas://saved-context/{source_id}")})),
            repo_root,
            &db_path,
        )
        .expect("resource read");
        assert_eq!(result["contents"][0]["text"], json!(payload));
    }

    #[test]
    fn resources_read_tool_docs_returns_examples_and_usage() {
        let dir = TempDir::new().expect("tempdir");
        let repo_root = dir.path().to_str().expect("repo root");
        let db_path = dir
            .path()
            .join("worldtree.db")
            .to_string_lossy()
            .into_owned();

        let result = resources_read(
            Some(&json!({"uri": "atlas://tool-docs/query_graph"})),
            repo_root,
            &db_path,
        )
        .expect("resource read");
        assert_eq!(result["contents"][0]["mimeType"], json!("text/markdown"));
        let text = result["contents"][0]["text"]
            .as_str()
            .expect("markdown text");
        assert!(text.contains("# Tool Docs: `query_graph`"));
        assert!(text.contains("## Usage"));
        assert!(text.contains("### Target Tool Call Examples"));
    }

    #[test]
    fn resources_read_prompt_docs_returns_prompt_body_and_args() {
        let dir = TempDir::new().expect("tempdir");
        let repo_root = dir.path().to_str().expect("repo root");
        let db_path = dir
            .path()
            .join("worldtree.db")
            .to_string_lossy()
            .into_owned();

        let result = resources_read(
            Some(&json!({"uri": "atlas://prompt-docs/review_change"})),
            repo_root,
            &db_path,
        )
        .expect("resource read");
        assert_eq!(result["contents"][0]["mimeType"], json!("text/markdown"));
        let text = result["contents"][0]["text"]
            .as_str()
            .expect("markdown text");
        assert!(text.contains("# Prompt Docs: `review_change`"));
        assert!(text.contains("## Arguments"));
        assert!(text.contains("## Default Prompt Body"));
        assert!(text.contains("detect_changes"));
    }

    #[test]
    fn resources_read_unknown_uri_returns_error() {
        let error = resources_read(
            Some(&json!({"uri": "atlas://unknown"})),
            "/repo",
            "/repo/.atlas/worldtree.db",
        )
        .expect_err("unknown resource must fail");
        assert!(error.to_string().contains("unknown resource uri"));
    }

    #[test]
    fn docs_completion_lists_markdown_files() {
        let dir = TempDir::new().expect("tempdir");
        let docs_dir = dir.path().join("wiki");
        fs::create_dir_all(&docs_dir).expect("docs dir");
        fs::write(docs_dir.join("guide.md"), "# Guide\n").expect("guide");
        fs::write(docs_dir.join("notes.txt"), "ignore").expect("notes");

        let files = docs_completion_items(
            dir.path().to_str().expect("repo root"),
            "file",
            "wiki/",
            None,
        )
        .expect("file completion");
        assert_eq!(files, vec!["wiki/guide.md"]);
    }
}
