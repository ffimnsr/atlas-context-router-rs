//! Read-only MCP surface for the persisted taxonomy tables: Louvain
//! communities and user-curated flows.
//!
//! Communities are derived data written by `postprocess_graph` (stage
//! `communities`) and consumed as a fallback module bucket by
//! `infer_modules`/`label_components`; flows are user-curated ordered symbol
//! sequences managed from the CLI. Both were previously CLI-only, so agents on
//! the MCP loop could not inspect them or detect staleness. These handlers
//! keep payloads bounded: member lists are capped with an explicit truncated
//! flag instead of streaming unbounded rows.

use anyhow::Result;
use serde_json::{Value, json};

use super::shared::{bool_arg, open_store, str_arg, tool_result_value};

use crate::tool_result::{
    InputShapeErrorSpec, input_shape_error_payload, tool_execution_error_value,
};

/// Maximum member qualified names emitted per community or flow.
const TAXONOMY_NODE_LIMIT: usize = 100;

pub(super) fn tool_list_communities(
    args: Option<&Value>,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<Value> {
    let include_nodes = bool_arg(args, "include_nodes").unwrap_or(false);
    let store = open_store(db_path)?;
    let communities = store
        .list_communities()
        .map_err(|error| anyhow::anyhow!("cannot list communities: {error}"))?;

    let mut total_memberships = 0usize;
    let community_values = communities
        .iter()
        .map(|community| {
            let members = store
                .get_community_nodes(community.id)
                .map_err(|error| anyhow::anyhow!("cannot load community nodes: {error}"))?;
            total_memberships += members.len();
            let nodes_truncated = members.len() > TAXONOMY_NODE_LIMIT;
            let mut value = json!({
                "id": community.id,
                "name": &community.name,
                "algorithm": &community.algorithm,
                "level": community.level,
                "parent_community_id": community.parent_community_id,
                "member_count": members.len(),
            });
            if include_nodes {
                value["nodes"] = json!(
                    members
                        .iter()
                        .take(TAXONOMY_NODE_LIMIT)
                        .map(|member| &member.node_qualified_name)
                        .collect::<Vec<_>>()
                );
                value["nodes_truncated"] = json!(nodes_truncated);
            }
            Ok(value)
        })
        .collect::<Result<Vec<_>>>()?;

    let payload = json!({
        "communities": community_values,
        "summary": {
            "community_count": communities.len(),
            "total_memberships": total_memberships,
            "include_nodes": include_nodes,
        },
        "warnings": [],
    });
    tool_result_value(&payload, output_format)
}

pub(super) fn tool_list_flows(
    args: Option<&Value>,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<Value> {
    let name_filter = str_arg(args, "name")?;
    let node_qn_filter = str_arg(args, "node_qn")?;
    // Fail closed on ambiguous filters: name and node_qn select flows by
    // different keys and the combined result would hide a mistake in one of
    // them, mirroring the conflict rejection style of build/update inputs.
    if let (Some(_), Some(_)) = (name_filter, node_qn_filter) {
        return tool_execution_error_value(
            output_format,
            &input_shape_error_payload(
                "list_flows",
                "list_flows accepts either name or node_qn, not both",
                "Filter by one key; name returns one flow with ordered members, node_qn returns every flow containing the symbol.",
                InputShapeErrorSpec {
                    offending_fields: vec!["name/node_qn".to_owned()],
                    normalization_performed: Vec::new(),
                    accepted_argument_families: vec!["name".to_owned(), "node_qn".to_owned()],
                    retry_example: Some(json!({ "name": "checkout" })),
                    fail_closed_reason: None,
                    retry_guidance: Some(
                        "Re-call list_flows with exactly one of name or node_qn.".to_owned(),
                    ),
                    extra_details: None,
                },
            ),
        );
    }

    let store = open_store(db_path)?;
    let (flows, matched, filtered_by, warnings) = if let Some(name) = name_filter {
        let flow = store
            .get_flow_by_name(name)
            .map_err(|error| anyhow::anyhow!("cannot look up flow: {error}"))?;
        match flow {
            Some(flow) => (vec![flow], true, Some("name"), Vec::new()),
            None => {
                let warning = format!("flow not found: {name}");
                (Vec::new(), false, Some("name"), vec![warning])
            }
        }
    } else if let Some(node_qn) = node_qn_filter {
        let flows = store
            .flows_for_node(node_qn)
            .map_err(|error| anyhow::anyhow!("cannot look up flows for node: {error}"))?;
        let matched = !flows.is_empty();
        let mut warnings = Vec::new();
        if !matched {
            warnings.push(format!("no flow contains node: {node_qn}"));
        }
        (flows, matched, Some("node_qn"), warnings)
    } else {
        let flows = store
            .list_flows()
            .map_err(|error| anyhow::anyhow!("cannot list flows: {error}"))?;
        (flows, true, None, Vec::new())
    };

    let mut total_memberships = 0usize;
    let flow_values = flows
        .iter()
        .map(|flow| {
            let members = store
                .get_flow_members(flow.id)
                .map_err(|error| anyhow::anyhow!("cannot load flow members: {error}"))?;
            total_memberships += members.len();
            let members_truncated = members.len() > TAXONOMY_NODE_LIMIT;
            let member_values = members
                .iter()
                .take(TAXONOMY_NODE_LIMIT)
                .map(|member| {
                    json!({
                        "node_qn": &member.node_qualified_name,
                        "position": member.position,
                        "role": &member.role,
                    })
                })
                .collect::<Vec<_>>();
            Ok(json!({
                "id": flow.id,
                "name": &flow.name,
                "kind": &flow.kind,
                "description": &flow.description,
                "member_count": members.len(),
                "members": member_values,
                "members_truncated": members_truncated,
            }))
        })
        .collect::<Result<Vec<_>>>()?;

    let payload = json!({
        "flows": flow_values,
        "summary": {
            "flow_count": flows.len(),
            "total_memberships": total_memberships,
            "matched": matched,
            "filtered_by": filtered_by,
        },
        "warnings": warnings,
    });
    tool_result_value(&payload, output_format)
}
