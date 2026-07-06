use anyhow::Result;
use serde::Serialize;
use serde_json::Value;

use crate::prompts::prompt_descriptors;
use crate::resources::docs_completion_items;
use crate::tools::{parse_mcp_intent, tool_descriptors};

const INTENT_VALUES: &[&str] = &[
    "symbol",
    "file",
    "review",
    "impact",
    "usage_lookup",
    "refactor_safety",
    "dead_code_check",
    "rename_preview",
    "dependency_removal",
];

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompletionResponse {
    completion: CompletionItems,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompletionItems {
    values: Vec<CompletionValue>,
    has_more: bool,
    total: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct CompletionValue {
    value: String,
    label: String,
}

pub(crate) fn complete(args: Option<&Value>, repo_root: &str) -> Result<Value> {
    let argument = args
        .and_then(|value| value.get("argument"))
        .or(args)
        .unwrap_or(&Value::Null);
    let name = argument
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| {
            args.and_then(|value| value.get("name"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default();
    let prefix = argument
        .get("value")
        .and_then(Value::as_str)
        .or_else(|| {
            args.and_then(|value| value.get("value"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default();
    let context = args.and_then(|value| value.get("context"));
    let request_ref = args.and_then(|value| value.get("ref"));

    let mut values = match name {
        "output_format" => filter_prefix(["json", "toon"], prefix),
        "intent" => filter_prefix(INTENT_VALUES.iter().copied(), prefix),
        "file" | "heading" if is_docs_template_ref(request_ref) => {
            docs_completion_items(repo_root, name, prefix, context)?
        }
        "name" if is_tools_call_ref(request_ref) => filter_prefix(tool_names(), prefix),
        "name" if is_prompts_get_ref(request_ref) => filter_prefix(prompt_names(), prefix),
        _ => Vec::new(),
    };

    if name == "intent" && values.is_empty() && !prefix.is_empty() {
        let normalized = parse_mcp_intent(prefix);
        values = vec![intent_name(normalized).to_owned()];
    }

    let total = values.len();
    let values = values
        .into_iter()
        .map(|value| CompletionValue {
            label: value.clone(),
            value,
        })
        .collect::<Vec<_>>();

    serde_json::to_value(CompletionResponse {
        completion: CompletionItems {
            values,
            has_more: false,
            total,
        },
    })
    .map_err(Into::into)
}

fn tool_names() -> Vec<String> {
    let mut names = tool_descriptors()
        .into_iter()
        .map(|tool| tool.name)
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

fn prompt_names() -> Vec<String> {
    let mut names = prompt_descriptors()
        .into_iter()
        .map(|prompt| prompt.name)
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
}

fn filter_prefix<I, S>(values: I, prefix: &str) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let prefix_lower = prefix.to_ascii_lowercase();
    let mut filtered = values
        .into_iter()
        .map(|value| value.as_ref().to_owned())
        .filter(|value| value.to_ascii_lowercase().starts_with(&prefix_lower))
        .collect::<Vec<_>>();
    filtered.sort();
    filtered.dedup();
    filtered
}

fn is_tools_call_ref(request_ref: Option<&Value>) -> bool {
    request_ref_name(request_ref) == Some("tools/call")
}

fn is_prompts_get_ref(request_ref: Option<&Value>) -> bool {
    request_ref_name(request_ref) == Some("prompts/get")
}

fn is_docs_template_ref(request_ref: Option<&Value>) -> bool {
    request_ref_name(request_ref) == Some("atlas://docs/{file}#{heading}")
        || request_ref
            .and_then(|value| value.get("uriTemplate"))
            .and_then(Value::as_str)
            == Some("atlas://docs/{file}#{heading}")
}

fn request_ref_name(request_ref: Option<&Value>) -> Option<&str> {
    request_ref
        .and_then(|value| value.get("name").or_else(|| value.get("uriTemplate")))
        .and_then(Value::as_str)
}

fn intent_name(intent: atlas_core::model::ContextIntent) -> &'static str {
    match intent {
        atlas_core::model::ContextIntent::Symbol => "symbol",
        atlas_core::model::ContextIntent::File => "file",
        atlas_core::model::ContextIntent::Review => "review",
        atlas_core::model::ContextIntent::Impact => "impact",
        atlas_core::model::ContextIntent::ImpactAnalysis => "impact",
        atlas_core::model::ContextIntent::UsageLookup => "usage_lookup",
        atlas_core::model::ContextIntent::RefactorSafety => "refactor_safety",
        atlas_core::model::ContextIntent::DeadCodeCheck => "dead_code_check",
        atlas_core::model::ContextIntent::RenamePreview => "rename_preview",
        atlas_core::model::ContextIntent::DependencyRemoval => "dependency_removal",
    }
}

#[cfg(test)]
mod tests {
    use super::complete;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn output_format_completion_filters_by_prefix() {
        let result = complete(
            Some(&json!({
                "ref": {"name": "tools/call"},
                "argument": {"name": "output_format", "value": "j"}
            })),
            "/repo",
        )
        .expect("completion");

        assert_eq!(result["completion"]["values"][0]["value"], json!("json"));
        assert_eq!(result["completion"]["total"], json!(1));
    }

    #[test]
    fn tool_name_completion_uses_dispatcher_context() {
        let result = complete(
            Some(&json!({
                "ref": {"name": "tools/call"},
                "argument": {"name": "name", "value": "get_"}
            })),
            "/repo",
        )
        .expect("completion");

        let values = result["completion"]["values"]
            .as_array()
            .expect("values")
            .iter()
            .filter_map(|value| value["value"].as_str())
            .collect::<Vec<_>>();
        assert!(values.contains(&"get_context"));
        assert!(values.contains(&"get_review_context"));
    }

    #[test]
    fn docs_template_heading_completion_uses_context_file() {
        let dir = TempDir::new().expect("tempdir");
        let docs_dir = dir.path().join("wiki");
        fs::create_dir_all(&docs_dir).expect("docs dir");
        fs::write(docs_dir.join("guide.md"), "# Guide\n## Install\ntext\n").expect("guide");

        let result = complete(
            Some(&json!({
                "ref": {"uriTemplate": "atlas://docs/{file}#{heading}"},
                "argument": {"name": "heading", "value": "document.guide.i"},
                "context": {"arguments": {"file": "wiki/guide.md"}}
            })),
            dir.path().to_str().expect("repo root"),
        )
        .expect("completion");

        assert_eq!(
            result["completion"]["values"][0]["value"],
            json!("document.guide.install")
        );
    }

    #[test]
    fn unsupported_field_returns_empty_deterministic_result() {
        let result = complete(
            Some(&json!({"argument": {"name": "unknown", "value": "x"}})),
            "/repo",
        )
        .expect("completion");
        assert_eq!(result["completion"]["values"], json!([]));
        assert_eq!(result["completion"]["hasMore"], json!(false));
    }
}
