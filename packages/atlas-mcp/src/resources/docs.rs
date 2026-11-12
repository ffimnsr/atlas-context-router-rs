//! Docs-backed resource rendering and completion helpers.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use serde_json::Value;

use crate::prompts::prompt_descriptors;
use crate::tool_list;

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

pub(crate) fn split_docs_uri(value: &str) -> Result<(&str, &str)> {
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

pub(crate) fn render_docs_index_markdown() -> Result<String> {
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
            prompt.name,
            prompt.description.as_deref().unwrap_or_default()
        ));
    }

    Ok(lines.join("\n"))
}

pub(crate) fn render_tool_manual_markdown(document: &crate::tools::ToolManualDocument) -> String {
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
            "- annotations: read_only=`{}`, state_changing=`{}`, destructive=`{}`, idempotent=`{}`, open_world=`{}`",
            document.tool_structure.annotations.read_only,
            document.tool_structure.annotations.state_changing,
            document.tool_structure.annotations.destructive,
            document.tool_structure.annotations.idempotent,
            document.tool_structure.annotations.open_world
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
        "## Input Contract".to_owned(),
        "".to_owned(),
        format!(
            "- canonical_form: {}",
            document.input_contract.canonical_form
        ),
        "".to_owned(),
        "### Families".to_owned(),
        "".to_owned(),
    ]);
    if document.input_contract.families.is_empty() {
        lines.push("- none; single object input".to_owned());
    } else {
        for family in &document.input_contract.families {
            lines.push(format!(
                "- `{}` ({})",
                family.family_name, family.family_kind
            ));
            lines.push(format!(
                "  - discriminant_field: `{}`",
                family.discriminant_field.as_deref().unwrap_or("none")
            ));
            lines.push(format!(
                "  - accepted_values: `{}`",
                family.accepted_values.join(", ")
            ));
            lines.push(format!(
                "  - mutually_exclusive_legacy_fields: `{}`",
                family.mutually_exclusive_legacy_fields.join(", ")
            ));
            for variant in &family.variants {
                lines.push(format!("  - value `{}`", variant.value));
                lines.push(format!(
                    "    - required_companion_fields: `{}`",
                    variant.required_companion_fields.join(", ")
                ));
                lines.push(format!(
                    "    - minimal_example: `{}`",
                    variant.minimal_example
                ));
            }
        }
    }
    if !document.input_contract.notes.is_empty() {
        lines.extend(["".to_owned(), "### Notes".to_owned(), "".to_owned()]);
        for note in &document.input_contract.notes {
            lines.push(format!("- {note}"));
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
        "### Tool Usage Guidance".to_owned(),
        "".to_owned(),
    ]);
    for guidance in &document.usage.tool_usage {
        lines.push(format!("- {guidance}"));
    }

    lines.extend([
        "".to_owned(),
        "### Target Tool Call Examples".to_owned(),
        "".to_owned(),
    ]);
    for example in &document.usage.target_tool_call_examples {
        lines.push(format!("- `{example}`"));
    }

    lines.extend([
        "".to_owned(),
        "### Few-Shot Prompts".to_owned(),
        "".to_owned(),
    ]);
    for prompt in &document.usage.few_shot_prompts {
        lines.push(format!("- {prompt}"));
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

pub(crate) fn render_prompt_markdown(name: &str, rendered: &Value) -> String {
    let descriptor = prompt_descriptors()
        .into_iter()
        .find(|prompt| prompt.name == name)
        .expect("prompt descriptor must exist for rendered prompt docs");
    let description = rendered["description"]
        .as_str()
        .unwrap_or(descriptor.description.as_deref().unwrap_or_default());
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

    if let Some(arguments) = descriptor.arguments.as_ref() {
        if arguments.is_empty() {
            lines.push("- none".to_owned());
        } else {
            for argument in arguments {
                lines.push(format!(
                    "- `{}` ({}) — {}",
                    argument.name,
                    if argument.required.unwrap_or(false) {
                        "required"
                    } else {
                        "optional"
                    },
                    argument.description.as_deref().unwrap_or_default()
                ));
            }
        }
    } else {
        lines.push("- none".to_owned());
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
