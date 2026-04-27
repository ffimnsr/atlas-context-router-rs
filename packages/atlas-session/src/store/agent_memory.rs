use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use atlas_core::{AtlasError, Result};

use crate::SessionId;

use super::{
    AgentMemorySummary, AgentPartitionSummary, AgentResponsibilitySummary, DelegatedTaskSummary,
    SessionEventRow, SessionStore,
};

#[derive(Default)]
struct PartitionAccumulator {
    event_count: usize,
    last_event_at: Option<String>,
    active_task_count: usize,
    completed_task_count: usize,
}

#[derive(Clone)]
struct TaskAccumulator {
    task_id: String,
    title: String,
    status: String,
    agent_id: Option<String>,
    delegated_by: Option<String>,
    responsibility: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Default)]
struct ResponsibilityAccumulator {
    responsibilities: BTreeSet<String>,
    active_task_count: usize,
    completed_task_count: usize,
    last_event_at: Option<String>,
}

pub(super) fn summarize_agent_memory(
    store: &SessionStore,
    session_id: &SessionId,
    requested_agent_id: Option<&str>,
    merge_agent_partitions: bool,
) -> Result<AgentMemorySummary> {
    let events = store.list_events(session_id)?;
    summarize_agent_memory_from_events(events, requested_agent_id, merge_agent_partitions)
}

pub(super) fn summarize_agent_memory_from_events(
    events: Vec<SessionEventRow>,
    requested_agent_id: Option<&str>,
    merge_agent_partitions: bool,
) -> Result<AgentMemorySummary> {
    let merged_view = merge_agent_partitions || requested_agent_id.is_none();
    let mut partitions: BTreeMap<Option<String>, PartitionAccumulator> = BTreeMap::new();
    let mut tasks: BTreeMap<String, TaskAccumulator> = BTreeMap::new();
    let mut responsibilities: BTreeMap<String, ResponsibilityAccumulator> = BTreeMap::new();

    for event in events {
        let payload: Value = serde_json::from_str(&event.payload_json).map_err(|error| {
            AtlasError::Other(format!("cannot parse session event payload: {error}"))
        })?;
        let event_agent_id = extract_agent_id(&payload);
        if !scope_matches(
            event_agent_id.as_deref(),
            requested_agent_id,
            merge_agent_partitions,
        ) {
            continue;
        }

        let entry = partitions.entry(event_agent_id.clone()).or_default();
        entry.event_count += 1;
        entry.last_event_at =
            max_timestamp(entry.last_event_at.take(), Some(event.created_at.clone()));

        if let Some(task_update) =
            extract_task_update(&payload, &event.created_at, event_agent_id.clone())
        {
            let task = tasks
                .entry(task_update.task_id.clone())
                .or_insert_with(|| task_update.clone());
            if task.created_at > task_update.created_at {
                task.created_at = task_update.created_at.clone();
            }
            task.updated_at = task_update.updated_at.clone();
            task.status = task_update.status.clone();
            if task.agent_id.is_none() {
                task.agent_id = task_update.agent_id.clone();
            }
            if task.delegated_by.is_none() {
                task.delegated_by = task_update.delegated_by.clone();
            }
            if task.responsibility.is_none() {
                task.responsibility = task_update.responsibility.clone();
            }
            if task.title == task.task_id && task_update.title != task_update.task_id {
                task.title = task_update.title.clone();
            }
        }

        if let Some(agent_id) = event_agent_id.as_deref() {
            let responsibility = responsibilities.entry(agent_id.to_owned()).or_default();
            responsibility.last_event_at = max_timestamp(
                responsibility.last_event_at.take(),
                Some(event.created_at.clone()),
            );
            for value in extract_responsibilities(&payload) {
                responsibility.responsibilities.insert(value);
            }
        }
    }

    for task in tasks.values() {
        if let Some(agent_id) = task.agent_id.as_deref() {
            let partition = partitions.entry(Some(agent_id.to_owned())).or_default();
            if task.status == "completed" {
                partition.completed_task_count += 1;
            } else {
                partition.active_task_count += 1;
            }
            let responsibility = responsibilities.entry(agent_id.to_owned()).or_default();
            if task.status == "completed" {
                responsibility.completed_task_count += 1;
            } else {
                responsibility.active_task_count += 1;
            }
            if let Some(value) = task.responsibility.as_deref() {
                responsibility.responsibilities.insert(value.to_owned());
            }
            responsibility.last_event_at = max_timestamp(
                responsibility.last_event_at.take(),
                Some(task.updated_at.clone()),
            );
        }
    }

    let mut partition_summaries: Vec<AgentPartitionSummary> = partitions
        .into_iter()
        .map(|(agent_id, entry)| AgentPartitionSummary {
            agent_id,
            event_count: entry.event_count,
            last_event_at: entry.last_event_at,
            active_task_count: entry.active_task_count,
            completed_task_count: entry.completed_task_count,
        })
        .collect();
    partition_summaries.sort_by(|left, right| {
        right
            .event_count
            .cmp(&left.event_count)
            .then_with(|| left.agent_id.cmp(&right.agent_id))
    });

    let mut delegated_tasks: Vec<DelegatedTaskSummary> = tasks
        .into_values()
        .map(|task| DelegatedTaskSummary {
            task_id: task.task_id,
            title: task.title,
            status: task.status,
            agent_id: task.agent_id,
            delegated_by: task.delegated_by,
            responsibility: task.responsibility,
            created_at: task.created_at,
            updated_at: task.updated_at,
        })
        .collect();
    delegated_tasks.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.task_id.cmp(&right.task_id))
    });

    let mut responsibility_summaries: Vec<AgentResponsibilitySummary> = responsibilities
        .into_iter()
        .map(|(agent_id, entry)| AgentResponsibilitySummary {
            agent_id,
            responsibilities: entry.responsibilities.into_iter().collect(),
            active_task_count: entry.active_task_count,
            completed_task_count: entry.completed_task_count,
            last_event_at: entry.last_event_at,
        })
        .collect();
    responsibility_summaries.sort_by(|left, right| left.agent_id.cmp(&right.agent_id));

    Ok(AgentMemorySummary {
        merged_view,
        requested_agent_id: requested_agent_id.map(str::to_owned),
        partitions: partition_summaries,
        delegated_tasks,
        responsibilities: responsibility_summaries,
    })
}

fn scope_matches(
    event_agent_id: Option<&str>,
    requested_agent_id: Option<&str>,
    merge_agent_partitions: bool,
) -> bool {
    if merge_agent_partitions || requested_agent_id.is_none() {
        return true;
    }
    event_agent_id == requested_agent_id
}

fn max_timestamp(left: Option<String>, right: Option<String>) -> Option<String> {
    match (left, right) {
        (Some(left), Some(right)) => Some(std::cmp::max(left, right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn extract_task_update(
    payload: &Value,
    created_at: &str,
    fallback_agent_id: Option<String>,
) -> Option<TaskAccumulator> {
    let hook_event = payload.get("hook_event").and_then(Value::as_str)?;
    if !matches!(
        hook_event,
        "task-created" | "task-completed" | "subagent-start" | "subagent-stop"
    ) {
        return None;
    }

    let status = match hook_event {
        "task-completed" | "subagent-stop" => "completed",
        _ => "active",
    }
    .to_owned();
    let task_id = extract_first_string(
        payload,
        &[
            "task_id",
            "taskId",
            "subagent_id",
            "subagentId",
            "id",
            "name",
        ],
    )
    .or_else(|| fallback_agent_id.clone())
    .unwrap_or_else(|| hook_event.to_owned());
    let title = extract_first_string(
        payload,
        &["title", "summary", "description", "task", "prompt", "name"],
    )
    .unwrap_or_else(|| task_id.clone());
    let agent_id = fallback_agent_id.or_else(|| {
        extract_first_string(
            payload,
            &[
                "agent_id",
                "agentId",
                "agent_name",
                "agent",
                "subagent",
                "name",
            ],
        )
    });
    let delegated_by = extract_first_string(
        payload,
        &[
            "delegated_by",
            "delegatedBy",
            "parent_agent_id",
            "parentAgentId",
            "caller_agent_id",
            "delegator",
        ],
    );
    let responsibility = extract_responsibilities(payload).into_iter().next();

    Some(TaskAccumulator {
        task_id,
        title,
        status,
        agent_id,
        delegated_by,
        responsibility,
        created_at: created_at.to_owned(),
        updated_at: created_at.to_owned(),
    })
}

fn extract_agent_id(payload: &Value) -> Option<String> {
    extract_first_string(
        payload,
        &[
            "agent_id",
            "agentId",
            "agent_name",
            "agentName",
            "subagent_id",
            "subagentId",
            "subagent_name",
            "subagentName",
        ],
    )
}

fn extract_responsibilities(payload: &Value) -> Vec<String> {
    let mut out = BTreeSet::new();
    for key in [
        "responsibility",
        "responsibilities",
        "role",
        "scope",
        "description",
    ] {
        for value in extract_all_values(payload, key) {
            match value {
                Value::String(text) => {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        out.insert(trimmed.to_owned());
                    }
                }
                Value::Array(values) => {
                    for value in values {
                        if let Some(text) = value.as_str() {
                            let trimmed = text.trim();
                            if !trimmed.is_empty() {
                                out.insert(trimmed.to_owned());
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out.into_iter().collect()
}

fn extract_first_string(payload: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        for value in extract_all_values(payload, key) {
            if let Some(text) = value.as_str() {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_owned());
                }
            }
        }
    }
    None
}

fn extract_all_values<'a>(value: &'a Value, key: &str) -> Vec<&'a Value> {
    let mut out = Vec::new();
    collect_values(value, key, &mut out);
    out
}

fn collect_values<'a>(value: &'a Value, key: &str, out: &mut Vec<&'a Value>) {
    match value {
        Value::Object(map) => {
            if let Some(candidate) = map.get(key) {
                out.push(candidate);
            }
            for nested in map.values() {
                collect_values(nested, key, out);
            }
        }
        Value::Array(values) => {
            for nested in values {
                collect_values(nested, key, out);
            }
        }
        _ => {}
    }
}
