//! ICM-G8 — MCP overview memory tools.
//!
//! Thin wrappers over `atlas_review::overview`; CLI and MCP share the same
//! service layer so record shapes, validation failures, and defaults match.

use anyhow::{Context, Result};
use atlas_adapters::{RedactionRules, derive_content_db_path, load_redaction_rules_file};
use atlas_contentstore::ContentStore;
use atlas_session::{
    OverviewAnalysisInput, OverviewExportRequest, OverviewExportScope, OverviewFreshness,
    OverviewLevel, OverviewSearchFilter, OverviewSourceKind, OverviewSubjectKind, SessionStore,
    render_level,
};
use serde_json::Value;

use crate::output::OutputFormat;
use crate::tool_result::{
    ToolErrorCode, ToolErrorPayload, tool_execution_error_value,
    tool_result_value as build_tool_result_value,
};

fn string_arg<'a>(args: Option<&'a Value>, key: &str) -> Option<&'a str> {
    args.and_then(|a| a.get(key)).and_then(|v| v.as_str())
}

fn required_string<'a>(args: Option<&'a Value>, key: &str) -> Result<&'a str> {
    string_arg(args, key).ok_or_else(|| anyhow::anyhow!("missing required argument: {key}"))
}

fn bool_arg(args: Option<&Value>, key: &str) -> bool {
    args.and_then(|a| a.get(key))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn u64_arg(args: Option<&Value>, key: &str) -> Option<u64> {
    args.and_then(|a| a.get(key)).and_then(|v| v.as_u64())
}

fn level_arg(args: Option<&Value>) -> Result<Option<OverviewLevel>> {
    Ok(string_arg(args, "level")
        .map(str::parse::<OverviewLevel>)
        .transpose()?)
}

fn open_session(db_path: &str) -> Result<SessionStore> {
    let session_db = atlas_adapters::derive_session_db_path(db_path);
    if let Some(parent) = std::path::Path::new(&session_db).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    Ok(SessionStore::open(&session_db)?)
}

fn open_content(db_path: &str) -> Result<ContentStore> {
    let content_db = derive_content_db_path(db_path);
    Ok(ContentStore::open(&content_db)?)
}

fn load_rules(repo_root: &str) -> Result<RedactionRules> {
    let config =
        atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root)).unwrap_or_default();
    let Some(file) = config.sanitization.redaction_rules_file else {
        return Ok(RedactionRules::default());
    };
    let path = std::path::Path::new(&file);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        atlas_engine::paths::atlas_dir(repo_root).join(file)
    };
    load_redaction_rules_file(&path)
        .with_context(|| format!("cannot load redaction rules {}", path.display()))
}

pub fn tool_overview_export(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let scope: OverviewExportScope = string_arg(args, "scope")
        .unwrap_or("project")
        .parse()
        .map_err(anyhow::Error::from)?;
    let subjects = args
        .and_then(|a| a.get("subjects"))
        .and_then(|v| v.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .or_else(|| string_arg(args, "subject").map(|value| vec![value.to_owned()]))
        .unwrap_or_default();
    let request = OverviewExportRequest {
        scope,
        subjects,
        changed_only: bool_arg(args, "changed") || bool_arg(args, "changed_only"),
        since: string_arg(args, "since").map(str::to_owned),
        limit: u64_arg(args, "limit").map(|value| value as usize),
        include_code_spans: bool_arg(args, "include_code_spans"),
        include_callers: bool_arg(args, "include_callers"),
        include_callees: bool_arg(args, "include_callees"),
        include_tests: bool_arg(args, "include_tests"),
        max_tokens: u64_arg(args, "max_tokens").map(|value| value as usize),
        json: true,
    };
    let store = crate::tools::shared::open_store(db_path)?;
    let mut session = open_session(db_path)?;
    let result = atlas_review::overview::export_overview_packets(
        repo_root,
        db_path,
        &store,
        Some(&mut session),
        &request,
    )?;
    build_tool_result_value(
        &serde_json::json!({
            "tool": "overview_export",
            "repo_root": repo_root,
            "result": result,
            "summary": {
                "packet_count": result.packets.len(),
                "subject_count": result.subject_count,
                "omitted_subject_count": result.omitted_subject_count,
                "graph_freshness": result.freshness.state,
                "safe_to_answer": result.freshness.safe_to_answer,
                "budget_hit": result.budget.budget_hit,
            },
            "warnings": [],
        }),
        output_format,
    )
}

pub fn tool_overview_ingest(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let analysis = if let Some(value) = args.and_then(|a| a.get("analysis")) {
        match serde_json::from_value::<OverviewAnalysisInput>(value.clone()) {
            Ok(analysis) => analysis,
            Err(error) => return overview_ingest_invalid_input(output_format, error.to_string()),
        }
    } else if let Some(path) = string_arg(args, "path").or_else(|| string_arg(args, "input")) {
        let raw = match std::fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(error) => {
                return overview_ingest_invalid_input(
                    output_format,
                    format!("cannot read overview analysis file {path}: {error}"),
                );
            }
        };
        match serde_json::from_str::<OverviewAnalysisInput>(&raw) {
            Ok(analysis) => analysis,
            Err(error) => return overview_ingest_invalid_input(output_format, error.to_string()),
        }
    } else {
        return overview_ingest_invalid_input(
            output_format,
            "provide an `analysis` object or a `path` to analysis JSON".to_owned(),
        );
    };

    let store = crate::tools::shared::open_store(db_path)?;
    let mut session = open_session(db_path)?;
    let mut content = open_content(db_path)?;
    let rules = load_rules(repo_root)?;
    let result = atlas_review::overview::ingest_overview_analysis(
        repo_root,
        &store,
        &mut session,
        &mut content,
        &rules,
        &analysis,
    )?;
    build_tool_result_value(
        &serde_json::json!({
            "tool": "overview_ingest",
            "repo_root": repo_root,
            "record": result.record,
            "summary": {
                "overview_id": result.record.id,
                "subject_id": result.record.subject_id,
                "status": result.status,
                "subject_resolved": result.subject_resolved,
                "packet_validated": result.packet_validated,
                "redacted": result.redacted,
                "body_routed": result.body_routed,
            },
            "warnings": [],
        }),
        output_format,
    )
}

pub fn tool_overview_search(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let query = required_string(args, "query")?;
    let filter = OverviewSearchFilter {
        subject_kind: parse_kind(string_arg(args, "subject_kind"))?,
        freshness: parse_freshness(string_arg(args, "freshness"))?,
        package_name: string_arg(args, "package").map(str::to_owned),
        module_path: string_arg(args, "module").map(str::to_owned),
        canonical_file_path: string_arg(args, "file").map(str::to_owned),
        source_kind: parse_source_kind(string_arg(args, "source_kind"))?,
        limit: u64_arg(args, "limit").map(|value| value as usize),
    };
    let session = open_session(db_path)?;
    let hits = session.search_overview_records(repo_root, query, &filter)?;
    let level = level_arg(args)?;
    let hits_payload = match level {
        Some(level) => serde_json::json!(
            hits.iter()
                .map(|hit| serde_json::json!({
                    "score": hit.score,
                    "ranking": hit.ranking,
                    "record": hit.record,
                    "view": render_level(&hit.record, level),
                }))
                .collect::<Vec<_>>()
        ),
        None => serde_json::to_value(&hits)?,
    };
    build_tool_result_value(
        &{
            let mut payload = serde_json::json!({
                "tool": "overview_search",
                "repo_root": repo_root,
                "query": query,
                "hits": hits_payload,
                "summary": {
                    "hit_count": hits.len(),
                    "limit": filter.limit.unwrap_or(20),
                },
                "warnings": [],
            });
            if let Some(level) = level {
                payload["level"] = serde_json::json!(level.as_str());
            }
            payload
        },
        output_format,
    )
}

pub fn tool_overview_show(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let subject = string_arg(args, "subject")
        .or_else(|| string_arg(args, "target"))
        .ok_or_else(|| anyhow::anyhow!("missing required argument: subject"))?;
    let session = open_session(db_path)?;
    let level = level_arg(args)?;
    let record = match session
        .resolve_overview_subject(repo_root, subject)
        .map_err(anyhow::Error::from)
    {
        Ok(record) => Some(record),
        // A missing subject is a normal lookup miss, not a tool failure. Return a
        // stable object so callers can branch on `found` instead of parsing text.
        Err(error) if overview_subject_miss(&error) => None,
        Err(error) => return Err(error),
    };
    build_tool_result_value(
        &match record {
            Some(record) => {
                let mut payload = serde_json::json!({
                    "tool": "overview_show",
                    "repo_root": repo_root,
                    "found": true,
                    "subject": subject,
                    "record": record.clone(),
                    "matches": [record.clone()],
                    "summary": {
                        "overview_id": record.id,
                        "subject_id": record.subject_id,
                        "subject_kind": record.subject_kind.as_str(),
                        "freshness": record.freshness.as_str(),
                        "confidence": record.confidence,
                    },
                    "warnings": [],
                });
                if let Some(level) = level {
                    payload["level"] = serde_json::json!(level.as_str());
                    payload["view"] = serde_json::to_value(render_level(&record, level))?;
                }
                payload
            }
            None => serde_json::json!({
                "tool": "overview_show",
                "repo_root": repo_root,
                "found": false,
                "code": "subject_not_found",
                "subject": subject,
                "record": Value::Null,
                "matches": [],
                "warnings": [format!(
                    "No overview record matches '{subject}'. Run overview_search or overview_export to list subjects."
                )],
            }),
        },
        output_format,
    )
}

fn overview_ingest_invalid_input(output_format: OutputFormat, message: String) -> Result<Value> {
    let payload = ToolErrorPayload::new(ToolErrorCode::InvalidInput, message)
        .with_tool("overview_ingest")
        .with_retry_guidance("Send an `analysis` object or a `path` to analysis JSON, then retry.");
    tool_execution_error_value(output_format, &payload)
}

fn overview_subject_miss(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("overview subject") && message.contains("not found")
}

pub fn tool_overview_refresh_plan(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let store = crate::tools::shared::open_store(db_path)?;
    let mut session = open_session(db_path)?;
    let plan = atlas_review::overview::refresh_overview_plan(
        repo_root,
        &store,
        &mut session,
        string_arg(args, "since"),
        bool_arg(args, "apply"),
    )?;
    build_tool_result_value(
        &serde_json::json!({
            "tool": "overview_refresh_plan",
            "repo_root": repo_root,
            "plan": plan,
            "summary": {
                "changed_file_count": plan.changed_files.len(),
                "direct_stale_count": plan.direct_stale.len(),
                "possibly_stale_count": plan.possibly_stale.len(),
                "orphaned_count": plan.orphaned.len(),
                "unchanged_count": plan.unchanged_count,
                "apply": plan.apply,
                "applied_count": plan.applied_count,
            },
            "warnings": [],
        }),
        output_format,
    )
}

fn parse_kind(value: Option<&str>) -> Result<Option<OverviewSubjectKind>> {
    value
        .map(|value| value.parse().map_err(anyhow::Error::from))
        .transpose()
}

fn parse_freshness(value: Option<&str>) -> Result<Option<OverviewFreshness>> {
    value
        .map(|value| value.parse().map_err(anyhow::Error::from))
        .transpose()
}

fn parse_source_kind(value: Option<&str>) -> Result<Option<OverviewSourceKind>> {
    value
        .map(|value| value.parse().map_err(anyhow::Error::from))
        .transpose()
}
