//! ICM-F2 — MCP memoir tools.
//!
//! Thin wrappers over the shared memoir service layer in `atlas-session`.
//! Vocabulary validation, alias normalization, duplicate detection, and the
//! explicit `create_missing` opt-in all live in that layer so MCP and CLI
//! cannot drift.

use anyhow::Result;
use atlas_adapters::derive_session_db_path;
use atlas_session::{
    MemoirInspectQuery, MemoryScope, NewMemoirConcept, NewMemoirGraph, NewMemoirRelation,
    SessionStore,
};
use serde_json::Value;

use crate::output::OutputFormat;
use crate::tool_result::tool_result_value as build_tool_result_value;

fn string_arg<'a>(args: Option<&'a Value>, key: &str) -> Option<&'a str> {
    args.and_then(|a| a.get(key)).and_then(|v| v.as_str())
}

fn required_string<'a>(args: Option<&'a Value>, key: &str) -> Result<&'a str> {
    string_arg(args, key).ok_or_else(|| anyhow::anyhow!("missing required argument: {key}"))
}

fn scope_arg(args: Option<&Value>) -> Result<MemoryScope> {
    string_arg(args, "scope")
        .map(|raw| raw.parse().map_err(anyhow::Error::from))
        .transpose()
        .map(|scope| scope.unwrap_or_default())
}

fn open_store(db_path: &str) -> Result<SessionStore> {
    let session_db = derive_session_db_path(db_path);
    Ok(SessionStore::open(&session_db)?)
}

/// Create a memoir concept graph. Duplicate `(repo, scope, name)` identities
/// fail deterministically.
pub fn tool_memoir_create(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let name = required_string(args, "name")?;
    let description = string_arg(args, "description").unwrap_or_default();
    let scope = scope_arg(args)?;

    let mut store = open_store(db_path)?;
    let graph = store.create_memoir_graph(&NewMemoirGraph {
        repo_root: repo_root.to_owned(),
        scope,
        name: name.to_owned(),
        description: description.to_owned(),
        metadata: serde_json::json!({}),
    })?;

    build_tool_result_value(
        &serde_json::json!({
            "tool": "memoir_create",
            "repo_root": repo_root,
            "graph": graph,
            "summary": {
                "graph_id": graph.id,
                "name": graph.name,
                "scope": graph.scope.as_str(),
                "is_new": true,
            },
            "warnings": [],
        }),
        output_format,
    )
}

/// Add a concept to an existing memoir graph.
pub fn tool_memoir_add_concept(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let graph = required_string(args, "graph")?;
    let name = required_string(args, "name")?;
    let description = string_arg(args, "description").unwrap_or_default();
    let kind = string_arg(args, "kind").unwrap_or("concept");
    let source_id = string_arg(args, "source_id");
    let scope = scope_arg(args)?;

    let mut store = open_store(db_path)?;
    let concept = store.add_memoir_concept(&NewMemoirConcept {
        repo_root: repo_root.to_owned(),
        scope,
        graph: graph.to_owned(),
        name: name.to_owned(),
        description: description.to_owned(),
        kind: kind.to_owned(),
        source_id: source_id.map(str::to_owned),
        metadata: serde_json::json!({}),
    })?;

    build_tool_result_value(
        &serde_json::json!({
            "tool": "memoir_add_concept",
            "repo_root": repo_root,
            "concept": concept,
            "summary": {
                "concept_id": concept.id,
                "graph_id": concept.graph_id,
                "name": concept.name,
                "kind": concept.kind,
            },
            "warnings": [],
        }),
        output_format,
    )
}

/// Link two memoir concepts with a controlled relation type. Missing endpoints
/// require the explicit `create_missing` opt-in.
pub fn tool_memoir_link(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let graph = required_string(args, "graph")?;
    let source = required_string(args, "source")?;
    let target = required_string(args, "target")?;
    let relation = string_arg(args, "relation")
        .or_else(|| string_arg(args, "relation_type"))
        .ok_or_else(|| anyhow::anyhow!("missing required argument: relation"))?;
    let confidence = args
        .and_then(|a| a.get("confidence"))
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0);
    let source_id = string_arg(args, "source_id");
    let scope = scope_arg(args)?;
    let create_missing = args
        .and_then(|a| {
            a.get("create_missing")
                .or_else(|| a.get("allow_missing_concepts"))
        })
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let mut store = open_store(db_path)?;
    let relation = store.link_memoir_concepts(&NewMemoirRelation {
        repo_root: repo_root.to_owned(),
        scope,
        graph: graph.to_owned(),
        source: source.to_owned(),
        target: target.to_owned(),
        relation: relation.to_owned(),
        confidence,
        source_id: source_id.map(str::to_owned),
        metadata: serde_json::json!({}),
        create_missing,
    })?;

    build_tool_result_value(
        &serde_json::json!({
            "tool": "memoir_link",
            "repo_root": repo_root,
            "relation": relation,
            "summary": {
                "relation_id": relation.id,
                "relation_type": relation.relation_type,
                "source_concept_id": relation.source_concept_id,
                "target_concept_id": relation.target_concept_id,
                "confidence": relation.confidence,
                "created_missing_concepts": create_missing,
            },
            "warnings": [],
        }),
        output_format,
    )
}

/// Delete a memoir graph and all of its concepts and relations. Missing graphs
/// report `found: false`; `dry_run` reports counts without writing.
pub fn tool_memoir_delete(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let graph = required_string(args, "graph")?;
    let scope = scope_arg(args)?;
    let dry_run = args
        .and_then(|a| a.get("dry_run"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let store = open_store(db_path)?;
    let result = store.delete_memoir_graph(repo_root, scope, graph, dry_run)?;

    build_tool_result_value(
        &serde_json::json!({
            "tool": "memoir_delete",
            "repo_root": repo_root,
            "result": result,
            "summary": {
                "graph": result.graph,
                "scope": result.scope.as_str(),
                "found": result.found,
                "deleted": result.deleted,
                "dry_run": result.dry_run,
                "concepts_deleted": result.concepts_deleted,
                "relations_deleted": result.relations_deleted,
            },
            "warnings": [],
        }),
        output_format,
    )
}

/// Bounded concept inspection with canonical relation direction and source
/// evidence ids.
pub fn tool_memoir_inspect(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let concept = required_string(args, "concept")?;
    let graph = string_arg(args, "graph").map(str::to_owned);
    let depth = args
        .and_then(|a| a.get("depth"))
        .and_then(|v| v.as_u64())
        .unwrap_or(1) as u32;
    let relation = string_arg(args, "relation").map(str::to_owned);
    let scope = string_arg(args, "scope")
        .map(|raw| raw.parse().map_err(anyhow::Error::from))
        .transpose()?;

    let store = open_store(db_path)?;
    let inspection = store.inspect_memoir_concept(&MemoirInspectQuery {
        repo_root: repo_root.to_owned(),
        graph,
        concept: concept.to_owned(),
        scope,
        depth,
        relation,
    })?;

    build_tool_result_value(
        &serde_json::json!({
            "tool": "memoir_inspect",
            "repo_root": repo_root,
            "inspection": inspection,
            "summary": {
                "graph_id": inspection.graph.id,
                "graph_name": inspection.graph.name,
                "concept_id": inspection.root.id,
                "concept_name": inspection.root.name,
                "depth": inspection.depth,
                "edge_count": inspection.edges.len(),
                "truncated": inspection.truncated,
            },
            "warnings": [],
        }),
        output_format,
    )
}
