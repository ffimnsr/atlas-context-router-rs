use std::path::Path;
use std::process::Command;

use anyhow::Result;
use atlas_adapters::derive_content_db_path;
use atlas_contentstore::{ContentStore, SearchFilters};
use atlas_core::SearchQuery;
use atlas_store_sqlite::Store;
use ignore::WalkBuilder;
use serde::Serialize;
use serde_json::{Value, json};

use crate::prompts::prompt_descriptors;
use crate::resources::{
    docs_completion_items, resource_descriptors, resource_template_descriptors,
};
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
const REVIEW_FOCUS_VALUES: &[&str] = &[
    "api risk",
    "boundary violations",
    "cross-package impact",
    "missing tests",
    "performance regressions",
    "security",
    "test gaps",
];
const INSPECT_QUESTION_VALUES: &[&str] = &[
    "Explain what it does.",
    "How is it used?",
    "What could break if it changes?",
    "What tests cover it?",
    "What should I read next?",
];
const REFACTOR_GOAL_VALUES: &[&str] = &[
    "extract logic",
    "improve code safely",
    "remove dependency",
    "rename",
    "split module",
];
const RESUME_TASK_VALUES: &[&str] = &[
    "continue implementation",
    "recover prior decisions",
    "resume debugging",
    "resume review",
    "summarize last session",
];
const COMMON_GIT_REF_VALUES: &[&str] = &["HEAD", "main", "master", "origin/main", "origin/master"];
const MAX_COMPLETION_VALUES: usize = 25;

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

pub(crate) fn complete(args: Option<&Value>, repo_root: &str, db_path: &str) -> Result<Value> {
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
        "source_id" if is_saved_context_template_ref(request_ref) => {
            saved_context_source_ids(repo_root, db_path, prefix)?
        }
        "uri" if is_resources_read_ref(request_ref) => {
            resource_uri_candidates(repo_root, db_path, prefix, context)?
        }
        "name" if is_tools_call_ref(request_ref) => filter_prefix(tool_names(), prefix),
        "name" if is_prompts_get_ref(request_ref) => filter_prefix(prompt_names(), prefix),
        _ => prompt_argument_candidates(request_ref, name, prefix, repo_root, db_path)?,
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

fn is_saved_context_template_ref(request_ref: Option<&Value>) -> bool {
    request_ref_name(request_ref) == Some("atlas://saved-context/{source_id}")
        || request_ref
            .and_then(|value| value.get("uriTemplate"))
            .and_then(Value::as_str)
            == Some("atlas://saved-context/{source_id}")
}

fn is_resources_read_ref(request_ref: Option<&Value>) -> bool {
    request_ref_name(request_ref) == Some("resources/read")
}

fn request_ref_name(request_ref: Option<&Value>) -> Option<&str> {
    request_ref
        .and_then(|value| value.get("name").or_else(|| value.get("uriTemplate")))
        .and_then(Value::as_str)
}

fn prompt_argument_candidates(
    request_ref: Option<&Value>,
    name: &str,
    prefix: &str,
    repo_root: &str,
    db_path: &str,
) -> Result<Vec<String>> {
    let Some(prompt_name) = request_ref_name(request_ref) else {
        return Ok(Vec::new());
    };

    let values = match (prompt_name, name) {
        ("review_change", "files") => comma_separated_repo_file_candidates(repo_root, prefix)?,
        ("review_change", "base") => git_ref_candidates(repo_root, prefix),
        ("review_change", "focus") => filter_prefix(REVIEW_FOCUS_VALUES.iter().copied(), prefix),
        ("inspect_symbol", "symbol") => target_candidates(repo_root, db_path, prefix, false)?,
        ("inspect_symbol", "question") => {
            filter_prefix(INSPECT_QUESTION_VALUES.iter().copied(), prefix)
        }
        ("plan_refactor", "target") => target_candidates(repo_root, db_path, prefix, true)?,
        ("plan_refactor", "goal") => filter_prefix(REFACTOR_GOAL_VALUES.iter().copied(), prefix),
        ("resume_prior_session", "task") => {
            filter_prefix(RESUME_TASK_VALUES.iter().copied(), prefix)
        }
        _ => Vec::new(),
    };
    Ok(values)
}

fn target_candidates(
    repo_root: &str,
    db_path: &str,
    prefix: &str,
    include_files: bool,
) -> Result<Vec<String>> {
    if prefix.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut values = graph_symbol_candidates(db_path, prefix, include_files)?;
    if include_files || prefix.contains('/') || prefix.contains('.') {
        values.extend(repo_file_candidates(repo_root, prefix)?);
    }
    Ok(normalize_completion_values(values))
}

fn graph_symbol_candidates(
    db_path: &str,
    prefix: &str,
    include_files: bool,
) -> Result<Vec<String>> {
    if prefix.trim().is_empty() {
        return Ok(Vec::new());
    }
    let store = match Store::open(db_path) {
        Ok(store) => store,
        Err(_) => return Ok(Vec::new()),
    };
    let query = SearchQuery {
        text: String::new(),
        include_files,
        limit: MAX_COMPLETION_VALUES,
        regex_pattern: Some(format!("(?i)^{}", regex::escape(prefix))),
        ..Default::default()
    };
    let prefix_lower = prefix.to_ascii_lowercase();
    let mut values = Vec::new();
    for result in atlas_search::execute_query(&store, &query, false)? {
        let node = result.node;
        if !node.qualified_name.is_empty()
            && node
                .qualified_name
                .to_ascii_lowercase()
                .starts_with(&prefix_lower)
        {
            values.push(node.qualified_name.clone());
        }
        if !node.name.is_empty() && node.name.to_ascii_lowercase().starts_with(&prefix_lower) {
            values.push(node.name.clone());
        }
        if include_files
            && !node.file_path.is_empty()
            && node
                .file_path
                .to_ascii_lowercase()
                .starts_with(&prefix_lower)
        {
            values.push(node.file_path.clone());
        }
    }
    Ok(normalize_completion_values(values))
}

fn comma_separated_repo_file_candidates(repo_root: &str, prefix: &str) -> Result<Vec<String>> {
    let (base, tail) = split_comma_completion_prefix(prefix);
    let values = repo_file_candidates(repo_root, tail)?
        .into_iter()
        .map(|value| format!("{base}{value}"))
        .collect::<Vec<_>>();
    Ok(normalize_completion_values(values))
}

fn split_comma_completion_prefix(prefix: &str) -> (String, &str) {
    match prefix.rsplit_once(',') {
        Some((head, tail)) => (format!("{}, ", head.trim_end()), tail.trim_start()),
        None => (String::new(), prefix),
    }
}

fn repo_file_candidates(repo_root: &str, prefix: &str) -> Result<Vec<String>> {
    let prefix_lower = prefix.to_ascii_lowercase();
    let root = Path::new(repo_root);
    let mut values = Vec::new();
    let mut walk = WalkBuilder::new(root);
    walk.hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .parents(true)
        .filter_entry(|entry| {
            let Some(name) = entry.file_name().to_str() else {
                return true;
            };
            !matches!(name, ".git" | ".atlas" | "target")
        });
    for entry in walk.build() {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        if !entry
            .file_type()
            .map(|kind| kind.is_file())
            .unwrap_or(false)
        {
            continue;
        }
        let Ok(relative) = entry.path().strip_prefix(root) else {
            continue;
        };
        let relative = relative.to_string_lossy().replace('\\', "/");
        if relative.to_ascii_lowercase().starts_with(&prefix_lower) {
            values.push(relative);
        }
    }
    Ok(normalize_completion_values(values))
}

fn git_ref_candidates(repo_root: &str, prefix: &str) -> Vec<String> {
    let mut values = filter_prefix(COMMON_GIT_REF_VALUES.iter().copied(), prefix);
    let output = Command::new("git")
        .arg("for-each-ref")
        .arg("--format=%(refname:short)")
        .arg("refs/heads")
        .arg("refs/remotes")
        .arg("refs/tags")
        .current_dir(repo_root)
        .output();
    if let Ok(output) = output
        && output.status.success()
    {
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let dynamic = stdout
            .lines()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>();
        values.extend(filter_prefix(dynamic, prefix));
    }
    normalize_completion_values(values)
}

fn resource_uri_candidates(
    repo_root: &str,
    db_path: &str,
    prefix: &str,
    context: Option<&Value>,
) -> Result<Vec<String>> {
    let mut values = resource_descriptors()
        .into_iter()
        .map(|resource| resource.uri)
        .collect::<Vec<_>>();

    let docs_prefix = prefix.strip_prefix("atlas://docs/");
    if prefix.is_empty() || prefix == "atlas://" || docs_prefix.is_some() {
        values.extend(docs_uri_candidates(
            repo_root,
            docs_prefix.unwrap_or_default(),
            context,
        )?);
    }

    let saved_prefix = prefix.strip_prefix("atlas://saved-context/");
    if prefix.is_empty() || prefix == "atlas://" || saved_prefix.is_some() {
        values.extend(saved_context_uris(
            repo_root,
            db_path,
            saved_prefix.unwrap_or_default(),
        )?);
    }

    let tool_docs_prefix = prefix.strip_prefix("atlas://tool-docs/");
    if prefix.is_empty() || prefix == "atlas://" || tool_docs_prefix.is_some() {
        values.extend(tool_docs_uris(tool_docs_prefix.unwrap_or_default()));
    }

    let prompt_docs_prefix = prefix.strip_prefix("atlas://prompt-docs/");
    if prefix.is_empty() || prefix == "atlas://" || prompt_docs_prefix.is_some() {
        values.extend(prompt_docs_uris(prompt_docs_prefix.unwrap_or_default()));
    }

    values.extend(
        resource_template_descriptors()
            .into_iter()
            .map(|template| template.uri_template),
    );

    Ok(filter_prefix(normalize_completion_values(values), prefix))
}

fn docs_uri_candidates(
    repo_root: &str,
    prefix: &str,
    _context: Option<&Value>,
) -> Result<Vec<String>> {
    if let Some((file, heading_prefix)) = prefix.split_once('#') {
        let headings = docs_completion_items(
            repo_root,
            "heading",
            heading_prefix,
            Some(&json!({"arguments": {"file": file}})),
        )?;
        return Ok(normalize_completion_values(
            headings
                .into_iter()
                .map(|heading| format!("atlas://docs/{file}#{heading}"))
                .collect(),
        ));
    }

    let files = docs_completion_items(repo_root, "file", prefix, None)?;
    Ok(normalize_completion_values(
        files
            .into_iter()
            .map(|file| format!("atlas://docs/{file}#"))
            .collect(),
    ))
}

fn saved_context_uris(repo_root: &str, db_path: &str, prefix: &str) -> Result<Vec<String>> {
    Ok(saved_context_source_ids(repo_root, db_path, prefix)?
        .into_iter()
        .map(|source_id| format!("atlas://saved-context/{source_id}"))
        .collect())
}

fn tool_docs_uris(prefix: &str) -> Vec<String> {
    crate::tool_list()["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tool| tool["name"].as_str())
        .filter(|name| prefix.is_empty() || name.starts_with(prefix))
        .map(|name| format!("atlas://tool-docs/{name}"))
        .collect()
}

fn prompt_docs_uris(prefix: &str) -> Vec<String> {
    prompt_names()
        .into_iter()
        .filter(|name| prefix.is_empty() || name.starts_with(prefix))
        .map(|name| format!("atlas://prompt-docs/{name}"))
        .collect()
}

fn saved_context_source_ids(repo_root: &str, db_path: &str, prefix: &str) -> Result<Vec<String>> {
    let content_db = derive_content_db_path(db_path);
    let store = match ContentStore::open(&content_db) {
        Ok(store) => store,
        Err(_) => return Ok(Vec::new()),
    };
    let filters = SearchFilters {
        repo_root: Some(repo_root.to_owned()),
        ..Default::default()
    };
    store
        .recent_source_ids_by_prefix(prefix, &filters, MAX_COMPLETION_VALUES)
        .map(normalize_completion_values)
        .map_err(Into::into)
}

fn normalize_completion_values(values: Vec<String>) -> Vec<String> {
    let mut values = values
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values.truncate(MAX_COMPLETION_VALUES);
    values
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
    use atlas_adapters::derive_content_db_path;
    use atlas_contentstore::{ContentStore, SourceMeta};
    use serde_json::json;
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use tempfile::TempDir;

    struct CompletionFixture {
        _dir: TempDir,
        repo_root: String,
        db_path: String,
    }

    impl CompletionFixture {
        fn new() -> Self {
            let dir = TempDir::new().expect("tempdir");
            let src_dir = dir.path().join("src");
            let docs_dir = dir.path().join("wiki");
            fs::create_dir_all(&src_dir).expect("src dir");
            fs::create_dir_all(&docs_dir).expect("docs dir");
            fs::write(
                src_dir.join("lib.rs"),
                "pub fn greet() -> &'static str { \"hi\" }\npub fn helper() {}\n",
            )
            .expect("lib");
            fs::write(docs_dir.join("guide.md"), "# Guide\n## Install\ntext\n").expect("guide");
            fs::write(dir.path().join("README.md"), "# Fixture\n").expect("readme");
            git(dir.path(), &["init", "--quiet"]);
            git(dir.path(), &["config", "user.name", "Atlas Tests"]);
            git(
                dir.path(),
                &["config", "user.email", "atlas-tests@example.com"],
            );
            git(dir.path(), &["add", "."]);
            git(dir.path(), &["commit", "--quiet", "-m", "fixture baseline"]);
            git(dir.path(), &["branch", "topic/completions"]);

            let repo_root = dir.path().to_string_lossy().into_owned();
            let db_path = dir
                .path()
                .join(".atlas")
                .join("worldtree.db")
                .to_string_lossy()
                .into_owned();
            crate::tools::call(
                "build_or_update_graph",
                Some(&json!({"mode": "build", "output_format": "json"})),
                &repo_root,
                &db_path,
            )
            .expect("build graph");

            let content_db = derive_content_db_path(&db_path);
            let mut store = ContentStore::open(&content_db).expect("open content store");
            let _ = store.migrate();
            store
                .index_artifact(
                    SourceMeta {
                        id: "src-completion-123".to_owned(),
                        session_id: Some("session-test".to_owned()),
                        agent_id: None,
                        source_type: "mcp_artifact".to_owned(),
                        label: "completion seed".to_owned(),
                        repo_root: Some(repo_root.clone()),
                        identity_kind: "artifact_label".to_owned(),
                        identity_value: "completion-seed".to_owned(),
                    },
                    "completion seed content",
                    "text/plain",
                )
                .expect("seed content store");

            Self {
                _dir: dir,
                repo_root,
                db_path,
            }
        }
    }

    fn git(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn output_format_completion_filters_by_prefix() {
        let result = complete(
            Some(&json!({
                "ref": {"name": "tools/call"},
                "argument": {"name": "output_format", "value": "j"}
            })),
            "/repo",
            "/repo/.atlas/worldtree.db",
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
            "/repo/.atlas/worldtree.db",
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
    fn prompt_argument_completion_covers_missing_prompt_side() {
        let fixture = CompletionFixture::new();

        let files = complete(
            Some(&json!({
                "ref": {"name": "review_change"},
                "argument": {"name": "files", "value": "src/l"}
            })),
            &fixture.repo_root,
            &fixture.db_path,
        )
        .expect("files completion");
        let file_values = files["completion"]["values"]
            .as_array()
            .expect("file values")
            .iter()
            .filter_map(|value| value["value"].as_str())
            .collect::<Vec<_>>();
        assert!(file_values.contains(&"src/lib.rs"));

        let base = complete(
            Some(&json!({
                "ref": {"name": "review_change"},
                "argument": {"name": "base", "value": "to"}
            })),
            &fixture.repo_root,
            &fixture.db_path,
        )
        .expect("base completion");
        let base_values = base["completion"]["values"]
            .as_array()
            .expect("base values")
            .iter()
            .filter_map(|value| value["value"].as_str())
            .collect::<Vec<_>>();
        assert!(base_values.contains(&"topic/completions"));

        let symbol = complete(
            Some(&json!({
                "ref": {"name": "inspect_symbol"},
                "argument": {"name": "symbol", "value": "gre"}
            })),
            &fixture.repo_root,
            &fixture.db_path,
        )
        .expect("symbol completion");
        let symbol_values = symbol["completion"]["values"]
            .as_array()
            .expect("symbol values")
            .iter()
            .filter_map(|value| value["value"].as_str())
            .collect::<Vec<_>>();
        assert!(symbol_values.contains(&"greet"));

        let target = complete(
            Some(&json!({
                "ref": {"name": "plan_refactor"},
                "argument": {"name": "target", "value": "src/l"}
            })),
            &fixture.repo_root,
            &fixture.db_path,
        )
        .expect("target completion");
        let target_values = target["completion"]["values"]
            .as_array()
            .expect("target values")
            .iter()
            .filter_map(|value| value["value"].as_str())
            .collect::<Vec<_>>();
        assert!(target_values.contains(&"src/lib.rs"));

        let goal = complete(
            Some(&json!({
                "ref": {"name": "plan_refactor"},
                "argument": {"name": "goal", "value": "re"}
            })),
            &fixture.repo_root,
            &fixture.db_path,
        )
        .expect("goal completion");
        assert_eq!(
            goal["completion"]["values"][0]["value"],
            json!("remove dependency")
        );

        let task = complete(
            Some(&json!({
                "ref": {"name": "resume_prior_session"},
                "argument": {"name": "task", "value": "res"}
            })),
            &fixture.repo_root,
            &fixture.db_path,
        )
        .expect("task completion");
        let task_values = task["completion"]["values"]
            .as_array()
            .expect("task values")
            .iter()
            .filter_map(|value| value["value"].as_str())
            .collect::<Vec<_>>();
        assert!(task_values.contains(&"resume debugging"));
        assert!(task_values.contains(&"resume review"));
    }

    #[test]
    fn docs_template_heading_completion_uses_context_file() {
        let fixture = CompletionFixture::new();
        let result = complete(
            Some(&json!({
                "ref": {"uriTemplate": "atlas://docs/{file}#{heading}"},
                "argument": {"name": "heading", "value": "document.guide.i"},
                "context": {"arguments": {"file": "wiki/guide.md"}}
            })),
            &fixture.repo_root,
            &fixture.db_path,
        )
        .expect("completion");

        assert_eq!(
            result["completion"]["values"][0]["value"],
            json!("document.guide.install")
        );
    }

    #[test]
    fn resource_completion_covers_missing_resource_side() {
        let fixture = CompletionFixture::new();

        let uri = complete(
            Some(&json!({
                "ref": {"name": "resources/read"},
                "argument": {"name": "uri", "value": "atlas://"}
            })),
            &fixture.repo_root,
            &fixture.db_path,
        )
        .expect("resource uri completion");
        let uri_values = uri["completion"]["values"]
            .as_array()
            .expect("uri values")
            .iter()
            .filter_map(|value| value["value"].as_str())
            .collect::<Vec<_>>();
        assert!(uri_values.contains(&"atlas://docs/index"));
        assert!(uri_values.contains(&"atlas://graph/provenance"));
        assert!(
            uri_values
                .iter()
                .any(|value| value.starts_with("atlas://docs/wiki/guide.md#"))
        );
        assert!(
            uri_values
                .iter()
                .any(|value| value.starts_with("atlas://tool-docs/"))
        );
        assert!(uri_values.contains(&"atlas://saved-context/src-completion-123"));

        let source_id = complete(
            Some(&json!({
                "ref": {"uriTemplate": "atlas://saved-context/{source_id}"},
                "argument": {"name": "source_id", "value": "src-completion"}
            })),
            &fixture.repo_root,
            &fixture.db_path,
        )
        .expect("source id completion");
        let source_values = source_id["completion"]["values"]
            .as_array()
            .expect("source values")
            .iter()
            .filter_map(|value| value["value"].as_str())
            .collect::<Vec<_>>();
        assert!(source_values.contains(&"src-completion-123"));

        let tool_docs = complete(
            Some(&json!({
                "ref": {"name": "resources/read"},
                "argument": {"name": "uri", "value": "atlas://tool-docs/tool_"}
            })),
            &fixture.repo_root,
            &fixture.db_path,
        )
        .expect("tool docs uri completion");
        let tool_doc_values = tool_docs["completion"]["values"]
            .as_array()
            .expect("tool doc values")
            .iter()
            .filter_map(|value| value["value"].as_str())
            .collect::<Vec<_>>();
        assert!(tool_doc_values.contains(&"atlas://tool-docs/tool_help"));
        assert!(tool_doc_values.contains(&"atlas://tool-docs/tool_list"));

        let prompt_docs = complete(
            Some(&json!({
                "ref": {"name": "resources/read"},
                "argument": {"name": "uri", "value": "atlas://prompt-docs/re"}
            })),
            &fixture.repo_root,
            &fixture.db_path,
        )
        .expect("prompt docs uri completion");
        let prompt_doc_values = prompt_docs["completion"]["values"]
            .as_array()
            .expect("prompt doc values")
            .iter()
            .filter_map(|value| value["value"].as_str())
            .collect::<Vec<_>>();
        assert!(prompt_doc_values.contains(&"atlas://prompt-docs/resume_prior_session"));
        assert!(prompt_doc_values.contains(&"atlas://prompt-docs/review_change"));
    }

    #[test]
    fn unsupported_field_returns_empty_deterministic_result() {
        let result = complete(
            Some(&json!({"argument": {"name": "unknown", "value": "x"}})),
            "/repo",
            "/repo/.atlas/worldtree.db",
        )
        .expect("completion");
        assert_eq!(result["completion"]["values"], json!([]));
        assert_eq!(result["completion"]["hasMore"], json!(false));
    }
}
