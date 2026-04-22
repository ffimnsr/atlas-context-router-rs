//! MCP prompt template definitions.

use anyhow::Result;
use serde::Serialize;

#[derive(Clone, Copy)]
struct PromptDef {
    name: &'static str,
    description: &'static str,
    arguments: &'static [PromptArgDef],
}

#[derive(Clone, Copy)]
struct PromptArgDef {
    name: &'static str,
    description: &'static str,
    required: bool,
}

#[derive(Serialize)]
struct PromptDescriptor<'a> {
    name: &'a str,
    description: &'a str,
    arguments: Vec<PromptArgument<'a>>,
}

#[derive(Serialize)]
struct PromptArgument<'a> {
    name: &'a str,
    description: &'a str,
    required: bool,
}

#[derive(Serialize)]
struct PromptGetResponse {
    description: String,
    messages: Vec<PromptMessage>,
}

#[derive(Serialize)]
struct PromptMessage {
    role: &'static str,
    content: PromptContent,
}

#[derive(Serialize)]
struct PromptContent {
    #[serde(rename = "type")]
    kind: &'static str,
    text: String,
}

const REVIEW_CHANGE_ARGS: &[PromptArgDef] = &[
    PromptArgDef {
        name: "files",
        description: "Optional comma-separated repo-relative file list. If omitted, detect changes first.",
        required: false,
    },
    PromptArgDef {
        name: "base",
        description: "Optional git base ref for detect_changes flows, e.g. origin/main.",
        required: false,
    },
    PromptArgDef {
        name: "focus",
        description: "Optional review focus such as api risk, tests, or cross-package impact.",
        required: false,
    },
];

const INSPECT_SYMBOL_ARGS: &[PromptArgDef] = &[
    PromptArgDef {
        name: "symbol",
        description: "Symbol name or qualified name to inspect.",
        required: true,
    },
    PromptArgDef {
        name: "question",
        description: "Optional question to answer after graph exploration.",
        required: false,
    },
];

const PLAN_REFACTOR_ARGS: &[PromptArgDef] = &[
    PromptArgDef {
        name: "target",
        description: "Symbol or file to refactor.",
        required: true,
    },
    PromptArgDef {
        name: "goal",
        description: "Refactor goal, e.g. rename, remove dependency, or extract logic.",
        required: false,
    },
];

const RESUME_SESSION_ARGS: &[PromptArgDef] = &[PromptArgDef {
    name: "task",
    description: "Optional task or topic to recover from prior session context.",
    required: false,
}];

const PROMPTS: &[PromptDef] = &[
    PromptDef {
        name: "review_change",
        description: "Guide external LLM through Atlas MCP review flow for changed files.",
        arguments: REVIEW_CHANGE_ARGS,
    },
    PromptDef {
        name: "inspect_symbol",
        description: "Guide external LLM through Atlas MCP symbol lookup and usage exploration.",
        arguments: INSPECT_SYMBOL_ARGS,
    },
    PromptDef {
        name: "plan_refactor",
        description: "Guide external LLM through Atlas MCP refactor-safety and blast-radius checks.",
        arguments: PLAN_REFACTOR_ARGS,
    },
    PromptDef {
        name: "resume_prior_session",
        description: "Guide external LLM through Atlas MCP continuity and saved-context retrieval.",
        arguments: RESUME_SESSION_ARGS,
    },
];

pub fn prompt_list() -> serde_json::Value {
    let prompts = PROMPTS
        .iter()
        .map(|prompt| PromptDescriptor {
            name: prompt.name,
            description: prompt.description,
            arguments: prompt
                .arguments
                .iter()
                .map(|arg| PromptArgument {
                    name: arg.name,
                    description: arg.description,
                    required: arg.required,
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    serde_json::json!({ "prompts": prompts })
}

pub fn prompt_get(name: &str, args: Option<&serde_json::Value>) -> Result<serde_json::Value> {
    let response = match name {
        "review_change" => render_review_change(args),
        "inspect_symbol" => render_inspect_symbol(args),
        "plan_refactor" => render_plan_refactor(args),
        "resume_prior_session" => render_resume_prior_session(args),
        other => return Err(anyhow::anyhow!("unknown prompt: {other}")),
    }?;
    serde_json::to_value(response).map_err(Into::into)
}

fn render_review_change(args: Option<&serde_json::Value>) -> Result<PromptGetResponse> {
    let files =
        opt_string_arg(args, "files")?.unwrap_or_else(|| "<detect from git diff>".to_owned());
    let base = opt_string_arg(args, "base")?.unwrap_or_else(|| "<working tree>".to_owned());
    let focus = opt_string_arg(args, "focus")?
        .unwrap_or_else(|| "bugs, regressions, missing tests, and cross-boundary risk".to_owned());

    let text = format!(
        "Use Atlas MCP to review code changes. Stay grounded in tool output only. Prefer graph tools before file search.\n\nTarget inputs:\n- files: {files}\n- base: {base}\n- focus: {focus}\n\nRecommended workflow:\n1. If files are unknown, call detect_changes with base={base}. If files are already known, skip directly to context.\n2. Check `atlas_provenance` on first result. If repo_root or db_path looks wrong, call status or doctor before continuing.\n3. Call get_minimal_context for cheap triage.\n4. Call get_review_context for fuller changed-symbol, neighbor, and risk detail.\n5. If any result emits `atlas_freshness`, treat graph facts as potentially stale until build_or_update_graph runs.\n6. Call explain_change when API/signature risk, boundary violations, or test gaps need confirmation.\n7. Call get_impact_radius when blast radius needs explicit changed/impacted nodes and files.\n8. Use query_graph, symbol_neighbors, traverse_graph, or get_context only for targeted follow-up on symbols surfaced by review flow.\n\nResponse requirements:\n- Findings first, ordered by severity.\n- Mention changed symbols, impacted tests, ambiguity, truncation, confidence limits, and trust warnings from atlas_provenance/atlas_freshness.\n- Do not invent callers, tests, or dependencies not returned by Atlas."
    );

    Ok(single_message_response(
        "Review repository changes with Atlas MCP review and impact tools.",
        text,
    ))
}

fn render_inspect_symbol(args: Option<&serde_json::Value>) -> Result<PromptGetResponse> {
    let symbol = required_string_arg(args, "symbol")?;
    let question = opt_string_arg(args, "question")?.unwrap_or_else(|| {
        "Explain what it does, who depends on it, and what to read next.".to_owned()
    });

    let text = format!(
        "Use Atlas MCP to inspect symbol '{symbol}'. Stay grounded in graph results.\n\nQuestion:\n{question}\n\nRecommended workflow:\n1. Call query_graph with text='{symbol}'. Use semantic=true if name is short or ambiguous.\n2. Check `atlas_provenance`. If repo_root or db_path looks wrong for current workspace, stop and call status or doctor.\n3. If multiple candidates appear, compare qname, kind, and file path before choosing. Report ambiguity if unresolved.\n4. Call symbol_neighbors on chosen qname for immediate callers, callees, tests, and local neighborhood.\n5. Call get_context with query='{symbol}' for bounded ranked context. Use intent='usage_lookup' when appropriate.\n6. If any graph result emits `atlas_freshness`, note that pending edits may make edges or locations stale.\n7. Call traverse_graph only if one-hop neighbors are insufficient and you need wider caller/callee reach.\n8. Fall back to file reads only after graph tools stop answering structural questions.\n\nResponse requirements:\n- Name exact qname chosen.\n- Separate direct facts from weaker inferences.\n- Mention truncation, trust warnings, or unresolved edges when present."
    );

    Ok(single_message_response(
        "Inspect a symbol with Atlas MCP graph and context tools.",
        text,
    ))
}

fn render_plan_refactor(args: Option<&serde_json::Value>) -> Result<PromptGetResponse> {
    let target = required_string_arg(args, "target")?;
    let goal = opt_string_arg(args, "goal")?.unwrap_or_else(|| "improve code safely".to_owned());

    let text = format!(
        "Use Atlas MCP to plan refactor for target '{target}'. Goal: {goal}. Keep plan deterministic and evidence-backed.\n\nRecommended workflow:\n1. Resolve target with query_graph. If ambiguous, stop and surface ranked candidates.\n2. Check `atlas_provenance`. If repo_root or db_path does not match expected session, stop and repair session wiring first.\n3. Call get_context for target-centered context. Prefer intent='refactor_safety', 'rename_preview', or 'dependency_removal' when they match goal.\n4. Call symbol_neighbors for direct callers, callees, tests, and nearby nodes.\n5. If any response emits `atlas_freshness`, treat current graph as lagging local edits and include rebuild in validation plan.\n6. Call explain_change or get_impact_radius if likely blast radius crosses files or packages.\n7. Use cross_file_links or concept_clusters when refactor may affect tightly coupled files beyond direct call edges.\n\nResponse requirements:\n- State exact target resolved.\n- List primary risks, affected files/symbols, test coverage gaps, and trust warnings.\n- Recommend validation steps before apply.\n- Do not claim rename/removal safety unless Atlas evidence supports it."
    );

    Ok(single_message_response(
        "Plan a safe refactor with Atlas MCP context and impact tools.",
        text,
    ))
}

fn render_resume_prior_session(args: Option<&serde_json::Value>) -> Result<PromptGetResponse> {
    let task = opt_string_arg(args, "task")?
        .unwrap_or_else(|| "recover relevant prior work and continue efficiently".to_owned());

    let text = format!(
        "Use Atlas MCP to resume prior session context. Goal: {task}. Prefer retrieval-backed restoration over guesswork.\n\nRecommended workflow:\n1. Call get_session_status to confirm session identity and whether resume snapshot exists.\n2. Call resume_session to load compact prior state.\n3. If resume snapshot is insufficient, call search_saved_context with task-focused terms.\n4. Call get_context with include_saved_context=true when structural graph context must be merged with saved artifacts.\n5. Keep raw saved artifacts summarized; do not dump large blobs back into context unless needed.\n\nResponse requirements:\n- State what prior context was recovered.\n- Mention source_ids or retrieval hints used.\n- Call out missing context explicitly instead of filling gaps with assumptions."
    );

    Ok(single_message_response(
        "Resume prior Atlas session state and saved context.",
        text,
    ))
}

fn single_message_response(description: impl Into<String>, text: String) -> PromptGetResponse {
    PromptGetResponse {
        description: description.into(),
        messages: vec![PromptMessage {
            role: "user",
            content: PromptContent { kind: "text", text },
        }],
    }
}

fn opt_string_arg(args: Option<&serde_json::Value>, key: &str) -> Result<Option<String>> {
    match args.and_then(|value| value.get(key)) {
        None => Ok(None),
        Some(serde_json::Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(anyhow::anyhow!("argument '{key}' must be a string")),
    }
}

fn required_string_arg(args: Option<&serde_json::Value>, key: &str) -> Result<String> {
    opt_string_arg(args, key)?.ok_or_else(|| anyhow::anyhow!("missing required argument: {key}"))
}

#[cfg(test)]
mod tests {
    use super::{prompt_get, prompt_list};

    #[test]
    fn prompt_list_exposes_expected_templates() {
        let listed = prompt_list();
        let prompts = listed["prompts"].as_array().expect("prompts array");
        let names = prompts
            .iter()
            .filter_map(|prompt| prompt["name"].as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "review_change",
                "inspect_symbol",
                "plan_refactor",
                "resume_prior_session"
            ]
        );
    }

    #[test]
    fn inspect_symbol_prompt_requires_symbol_argument() {
        let error = prompt_get("inspect_symbol", Some(&serde_json::json!({}))).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("missing required argument: symbol")
        );
    }

    #[test]
    fn inspect_symbol_prompt_renders_symbol_specific_guidance() {
        let rendered = prompt_get(
            "inspect_symbol",
            Some(&serde_json::json!({
                "symbol": "src/lib.rs::fn::compute",
                "question": "What depends on this?"
            })),
        )
        .expect("prompt get");

        let text = rendered["messages"][0]["content"]["text"]
            .as_str()
            .expect("prompt text");
        assert!(text.contains("src/lib.rs::fn::compute"));
        assert!(text.contains("What depends on this?"));
        assert!(text.contains("symbol_neighbors"));
        assert!(text.contains("get_context"));
        assert!(text.contains("atlas_provenance"));
        assert!(text.contains("atlas_freshness"));
        assert!(text.contains("status or doctor"));
    }
}
