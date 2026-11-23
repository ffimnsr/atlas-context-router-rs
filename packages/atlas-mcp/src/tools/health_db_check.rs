//! `db_check` MCP tool: SQLite integrity, orphan/dangling rows, and
//! session-side schema probes.
//!
//! Split out of `health.rs` so that module stays below the 1,000 LOC ceiling.

use anyhow::{Context, Result};
use atlas_core::BudgetManager;
use camino::Utf8Path;
use serde_json::json;

use crate::tool_result::{ToolSuccessEnvelope, normalized_tool_result_value};

use super::health::{
    integrity_issue_code, mcp_graph_recovery_suggestions, structural_dangling_edges,
    structural_orphans,
};
use super::shared::{
    error_code_docs, error_message, error_suggestions, inject_budget_metadata, load_budget_policy,
    open_store, u64_arg,
};
pub(super) fn tool_db_check(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    const DEFAULT_LIMIT: usize = 100;
    let policy = load_budget_policy(repo_root)?;
    let mut budgets = BudgetManager::new();
    let requested_limit = u64_arg(args, "limit").unwrap_or(DEFAULT_LIMIT as u64) as usize;
    let limit = budgets.resolve_limit(
        policy.mcp_cli_payload_serialization.nodes,
        "mcp_cli_payload_serialization.max_nodes",
        Some(requested_limit),
    );

    let store = open_store(db_path)?;
    let issues = store.integrity_check().context("integrity check failed")?;
    let orphans = structural_orphans(&store, limit);
    let dangling = structural_dangling_edges(&store, limit);
    let source_repo_id = atlas_repo::stable_repo_id(Utf8Path::new(repo_root));
    let invalid_reachability = store.invalid_reachability_edges(&source_repo_id, limit)?;
    let reachability_provenance_ok = invalid_reachability.is_empty();

    // ICM-A1 + ICM-C1: validate the continuity-side memory and feedback
    // schemas through db check. A missing session DB is not a schema violation
    // (created on demand), but a present one must carry the full schemas.
    let session_db_path = atlas_engine::paths::session_db_path(db_path);
    let session_db_exists = std::path::Path::new(&session_db_path).exists();
    let (
        session_db_ok,
        memory_schema_ok,
        memory_schema_issues,
        feedback_schema_ok,
        feedback_schema_issues,
    ) = if session_db_exists {
        match atlas_session::SessionStore::open(&session_db_path) {
            Ok(session_store) => {
                let memory_issues = session_store.memory_schema_issues();
                let feedback_issues = session_store.feedback_schema_issues();
                (
                    true,
                    memory_issues.is_empty(),
                    memory_issues,
                    feedback_issues.is_empty(),
                    feedback_issues,
                )
            }
            Err(e) => (
                false,
                false,
                vec![format!("cannot open session store: {e}")],
                false,
                vec![format!("cannot open session store: {e}")],
            ),
        }
    } else {
        (true, true, Vec::new(), true, Vec::new())
    };

    let noncanonical_path_rows = issues
        .iter()
        .filter(|issue| issue.starts_with("noncanonical_path:"))
        .cloned()
        .collect::<Vec<_>>();
    let integrity_issues = issues
        .iter()
        .filter(|issue| !issue.starts_with("noncanonical_path:"))
        .cloned()
        .collect::<Vec<_>>();

    let orphan_nodes = orphans
        .iter()
        .map(|n| {
            json!({
                "kind": n.kind.as_str(),
                "qualified_name": &n.qualified_name,
                "file_path": &n.file_path,
                "line_start": n.line_start,
            })
        })
        .collect::<Vec<_>>();

    let dangling_edges = dangling
        .iter()
        .map(|(id, src, tgt, kind, side)| {
            json!({
                "id": id,
                "kind": kind,
                "source_qn": src,
                "target_qn": tgt,
                "missing_side": side,
            })
        })
        .collect::<Vec<_>>();

    let ok = integrity_issues.is_empty()
        && noncanonical_path_rows.is_empty()
        && orphan_nodes.is_empty()
        && dangling_edges.is_empty()
        && reachability_provenance_ok
        && session_db_ok
        && memory_schema_ok
        && feedback_schema_ok;
    let failure_category = if !issues.is_empty() || !orphans.is_empty() || !dangling.is_empty() {
        integrity_issue_code(&issues, !orphans.is_empty() || !dangling.is_empty())
    } else if !reachability_provenance_ok {
        "logical_inconsistency"
    } else if !memory_schema_ok || !feedback_schema_ok {
        "schema_mismatch"
    } else {
        "none"
    };
    let health_class = match failure_category {
        "none" => Some("healthy"),
        "schema_mismatch" => Some("schema_mismatch"),
        "sqlite_corrupt" => Some("sqlite_corrupt"),
        "logical_inconsistency" => Some("logical_inconsistency"),
        _ => None,
    };
    let mut warnings = if noncanonical_path_rows.is_empty() {
        Vec::new()
    } else {
        vec![
            "noncanonical path rows detected; rebuild graph/content state from canonical inputs"
                .to_owned(),
        ]
    };
    if !memory_schema_ok {
        warnings.push(
            "memory schema mismatch; run `atlas migrate` to refresh the session database schema"
                .to_owned(),
        );
    }
    if !feedback_schema_ok {
        warnings.push(
            "feedback schema mismatch; run `atlas migrate` to refresh the session database schema"
                .to_owned(),
        );
    }

    let schema_probes = {
        let (graph, content, session) = atlas_engine::doctor::probe_all_schemas(db_path);
        json!({
            "graph": graph,
            "content": content,
            "session": session,
        })
    };

    let payload = json!({
        "ok": ok,
        "health_class": health_class,
        "schema_version": schema_probes,
        "recovery_mode": "block_only",
        "quarantine_path": serde_json::Value::Null,
        "rebuild_result": if ok { "not_needed" } else { "blocked" },
        "failure_reason": serde_json::Value::Null,
        "integrity": {
            "ok": integrity_issues.is_empty() && noncanonical_path_rows.is_empty(),
            "issues": integrity_issues,
            "issue_count": issues.len(),
        },
        "orphan_nodes": orphan_nodes,
        "dangling_edges": dangling_edges,
        "noncanonical_path_rows": noncanonical_path_rows,
        "reachability_provenance": {
            "ok": reachability_provenance_ok,
            "invalid_count": invalid_reachability.len(),
            "invalid_edges": invalid_reachability
                .iter()
                .map(|(id, value)| json!({
                    "edge_id": id,
                    "reachability": value,
                }))
                .collect::<Vec<_>>(),
        },
        "session_db": {
            "path": session_db_path,
            "exists": session_db_exists,
            "ok": session_db_ok,
            "memory_schema": {
                "ok": memory_schema_ok,
                "issues": memory_schema_issues,
            },
            "feedback_schema": {
                "ok": feedback_schema_ok,
                "issues": feedback_schema_issues,
            },
        },
        "summary": {
            "ok": ok,
            "failure_category": failure_category,
            "health_class": health_class,
            "message": error_message(failure_category),
            "suggestions": error_suggestions(failure_category),
            "mcp_suggestions": mcp_graph_recovery_suggestions(failure_category),
            "error_code_docs": error_code_docs(failure_category),
            "orphan_node_count": orphans.len(),
            "dangling_edge_count": dangling.len(),
            "noncanonical_path_row_count": issues.iter().filter(|issue| issue.starts_with("noncanonical_path:")).count(),
        },
        "warnings": warnings,
        "db_path": db_path,
    });

    let observed = issues.len().max(orphans.len()).max(dangling.len());
    if orphans.len() >= limit || dangling.len() >= limit {
        budgets.record_usage(
            policy.mcp_cli_payload_serialization.nodes,
            "mcp_cli_payload_serialization.max_nodes",
            limit,
            observed,
            true,
        );
    }

    let envelope = ToolSuccessEnvelope::new("db_check", payload);
    let mut response = normalized_tool_result_value(&envelope, output_format)?;
    inject_budget_metadata(
        &mut response,
        &budgets.summary(
            "mcp_cli_payload_serialization.max_nodes",
            limit,
            requested_limit.max(observed),
        ),
    );
    Ok(response)
}
