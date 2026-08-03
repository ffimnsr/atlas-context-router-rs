#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use rmcp::model::{
    CacheScope, CallToolResponse, CallToolResult, CompleteResult, CompletionInfo, CreateTaskResult,
    DetailedTask, ElicitRequest, ElicitRequestParams, ElicitationSchema, GetPromptResult,
    GetTaskResult, Icon, InputRequest, InputRequiredResult, JsonObject, ListPromptsResult,
    ListResourceTemplatesResult, ListResourcesResult, MetaObject, NotificationMetaObject, Prompt,
    PromptMessage, ReadResourceResult, Resource, ResourceTemplate, Role, Task, TaskPayload,
    TaskStatus, TaskStatusNotificationParams, Tool,
};
use serde_json::Value;

use crate::descriptors::{
    IconDescriptor, PromptDescriptor, ResourceDescriptor, ResourceTemplateDescriptor,
    ToolDescriptor,
};
use atlas_session::{DurableTaskRecord, DurableTaskStatus};

pub(crate) const ATLAS_PUBLIC_LIST_TTL_MS: u64 = 300_000;
pub(crate) const ATLAS_TASK_META_PROGRESS: &str = "atlas/progress";
pub(crate) const ATLAS_TASK_META_CANCEL_REQUESTED: &str = "atlas/cancelRequested";
pub(crate) const ATLAS_TASK_META_REQUEST_STATE: &str = "atlas/requestState";

pub(crate) fn public_cache_scope() -> CacheScope {
    CacheScope::Public
}

pub(crate) fn tool_from_descriptor(descriptor: ToolDescriptor) -> Result<Tool> {
    Ok(descriptor)
}

pub(crate) fn prompt_from_descriptor(descriptor: PromptDescriptor) -> Result<Prompt> {
    Ok(descriptor)
}

pub(crate) fn resource_from_descriptor(descriptor: ResourceDescriptor) -> Result<Resource> {
    Ok(descriptor)
}

pub(crate) fn resource_template_from_descriptor(
    descriptor: ResourceTemplateDescriptor,
) -> Result<ResourceTemplate> {
    Ok(descriptor)
}

pub(crate) fn call_tool_response_from_atlas_value(value: Value) -> Result<CallToolResponse> {
    let Some(object) = value.as_object() else {
        return Err(anyhow!("expected Atlas tool result object"));
    };

    if object.contains_key("task") {
        return create_task_response_from_atlas_value(&value);
    }

    if object.get("resultType").and_then(Value::as_str) == Some("input_required") {
        return input_required_response_from_atlas_value(&value);
    }

    let value = if object.contains_key("content") {
        value
    } else {
        crate::tool_result::tool_result_value(&value, crate::output::OutputFormat::Json)?
    };

    Ok(CallToolResponse::Complete(serde_json::from_value::<
        CallToolResult,
    >(value)?))
}

pub(crate) fn meta_object_from_value(value: Value) -> Result<Option<MetaObject>> {
    match value {
        Value::Null => Ok(None),
        Value::Object(object) => Ok(Some(MetaObject(object))),
        other => Err(anyhow!("expected meta object, got {other}")),
    }
}

pub(crate) fn list_prompts_result_from_atlas_value(value: Value) -> Result<ListPromptsResult> {
    serde_json::from_value(value).map_err(Into::into)
}

pub(crate) fn list_resources_result_from_atlas_value(value: Value) -> Result<ListResourcesResult> {
    serde_json::from_value(value).map_err(Into::into)
}

pub(crate) fn list_resource_templates_result_from_atlas_value(
    value: Value,
) -> Result<ListResourceTemplatesResult> {
    serde_json::from_value(value).map_err(Into::into)
}

pub(crate) fn get_prompt_result_from_atlas_value(value: Value) -> Result<GetPromptResult> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("expected Atlas prompt result object"))?;

    let description = object
        .get("description")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let messages = object
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("expected Atlas prompt result messages array"))?
        .iter()
        .map(prompt_message_from_atlas_value)
        .collect::<Result<Vec<_>>>()?;

    let mut result = GetPromptResult::new(messages);
    if let Some(description) = description {
        result = result.with_description(description);
    }
    if let Some(meta) = optional_meta_from_parent(&value)? {
        result.meta = Some(meta);
    }
    Ok(result)
}

pub(crate) fn read_resource_result_from_atlas_value(value: Value) -> Result<ReadResourceResult> {
    serde_json::from_value(value).map_err(Into::into)
}

pub(crate) fn complete_result_from_atlas_value(value: Value) -> Result<CompleteResult> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("expected Atlas completion result object"))?;
    let completion = object
        .get("completion")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("expected completion object"))?;
    let raw_values = completion
        .get("values")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("expected completion values array"))?;
    let values = raw_values
        .iter()
        .map(|item| {
            item.get("value")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| anyhow!("completion values entries must include string value"))
        })
        .collect::<Result<Vec<_>>>()?;
    let total = completion
        .get("total")
        .and_then(Value::as_u64)
        .map(u32::try_from)
        .transpose()
        .map_err(|_| anyhow!("completion total exceeds u32"))?;
    let has_more = completion
        .get("hasMore")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let completion =
        CompletionInfo::with_pagination(values, total, has_more).map_err(|error| anyhow!(error))?;
    let mut result = CompleteResult::new(completion);
    if let Some(meta) = optional_meta_from_parent(&value)? {
        result.meta = Some(meta);
    }
    Ok(result)
}

pub(crate) fn create_task_result_from_record(task: &DurableTaskRecord) -> CreateTaskResult {
    let mut result = CreateTaskResult::new(task_handle_from_record(task));
    if let Some(meta) = task_meta_from_record(task) {
        result = result.with_meta(meta);
    }
    result
}

pub(crate) fn detailed_task_from_record(task: &DurableTaskRecord) -> Result<DetailedTask> {
    let payload = match task.status {
        DurableTaskStatus::Working => TaskPayload::Working,
        DurableTaskStatus::Cancelled => TaskPayload::Cancelled,
        DurableTaskStatus::Completed => TaskPayload::Completed {
            result: json_object_from_value(task.result.clone().unwrap_or(Value::Null))?,
        },
        DurableTaskStatus::Failed => TaskPayload::Failed {
            error: json_object_from_value(task.error.clone().unwrap_or_else(
                || serde_json::json!({ "message": format!("task '{}' failed", task.task_id) }),
            ))?,
        },
        DurableTaskStatus::InputRequired => TaskPayload::InputRequired {
            input_requests: serde_json::from_value(task.input_requests.clone().ok_or_else(
                || {
                    anyhow!(
                        "durable task '{}' is input_required but input_requests are missing",
                        task.task_id
                    )
                },
            )?)?,
        },
    };
    Ok(DetailedTask::new(task_handle_from_record(task), payload))
}

pub(crate) fn get_task_result_from_record(task: &DurableTaskRecord) -> Result<GetTaskResult> {
    let mut result = GetTaskResult::new(detailed_task_from_record(task)?);
    if let Some(meta) = task_meta_from_record(task) {
        result.meta = Some(meta);
    }
    Ok(result)
}

pub(crate) fn task_status_notification_from_record(
    task: &DurableTaskRecord,
) -> Result<TaskStatusNotificationParams> {
    let mut notification = TaskStatusNotificationParams::new(detailed_task_from_record(task)?);
    if let Some(meta) = task_notification_meta_from_record(task) {
        notification = notification.with_meta(meta);
    }
    Ok(notification)
}

fn create_task_response_from_atlas_value(value: &Value) -> Result<CallToolResponse> {
    let task = value
        .get("task")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("expected task object"))?;

    let mut rmcp_task = Task::new(
        required_string_field(task, "taskId")?,
        parse_task_status(required_string_field(task, "status")?)?,
        required_string_field(task, "createdAt")?,
        required_string_field(task, "lastUpdatedAt")?,
    );

    if let Some(status_message) = optional_string_field(task, "statusMessage") {
        rmcp_task = rmcp_task.with_status_message(status_message);
    }
    if let Some(ttl_ms) = optional_u64_field(task, "ttl") {
        rmcp_task = rmcp_task.with_ttl_ms(ttl_ms);
    }
    if let Some(poll_interval_ms) = optional_u64_field(task, "pollInterval") {
        rmcp_task = rmcp_task.with_poll_interval_ms(poll_interval_ms);
    }

    let mut result = CreateTaskResult::new(rmcp_task);
    if let Some(meta) = optional_meta_from_parent(value)? {
        result = result.with_meta(meta);
    }
    Ok(CallToolResponse::Task(result))
}

pub(crate) fn persisted_input_requests_from_atlas_tool_result(
    value: &Value,
) -> Result<Option<Value>> {
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    if object.get("resultType").and_then(Value::as_str) != Some("input_required") {
        return Ok(None);
    }

    let Some(raw_requests) = object.get("inputRequests") else {
        return Ok(None);
    };

    match raw_requests {
        Value::Object(_) => Ok(Some(raw_requests.clone())),
        Value::Array(items) => {
            let requests = atlas_input_requests_to_rmcp(items)?;
            Ok(Some(serde_json::to_value(requests)?))
        }
        other => Err(anyhow!(
            "inputRequests must be object or array, got {other}"
        )),
    }
}

fn input_required_response_from_atlas_value(value: &Value) -> Result<CallToolResponse> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("expected input_required result object"))?;

    let input_requests = match object.get("inputRequests") {
        None => None,
        Some(Value::Object(map)) => Some(serde_json::from_value(Value::Object(map.clone()))?),
        Some(Value::Array(items)) => Some(atlas_input_requests_to_rmcp(items)?),
        Some(other) => {
            return Err(anyhow!(
                "inputRequests must be object or array, got {other}"
            ));
        }
    };
    let request_state = optional_string_field(object, "requestState").map(str::to_owned);

    let mut result = InputRequiredResult::new(input_requests, request_state);
    if let Some(meta) = optional_meta_from_parent(value)? {
        result = result.with_meta(meta);
    }
    Ok(CallToolResponse::InputRequired(result))
}

fn atlas_input_requests_to_rmcp(items: &[Value]) -> Result<BTreeMap<String, InputRequest>> {
    let mut requests = BTreeMap::new();
    for item in items {
        let object = item
            .as_object()
            .ok_or_else(|| anyhow!("inputRequests entries must be objects"))?;
        let id = required_string_field(object, "id")?.to_owned();
        let request_type = required_string_field(object, "type")?;
        let request = match request_type {
            "form" => InputRequest::Elicitation(ElicitRequest::new(
                ElicitRequestParams::FormElicitationParams {
                    meta: None,
                    message: required_string_field(object, "message")?.to_owned(),
                    requested_schema: serde_json::from_value::<ElicitationSchema>(
                        object
                            .get("requestedSchema")
                            .cloned()
                            .ok_or_else(|| anyhow!("input request missing requestedSchema"))?,
                    )?,
                },
            )),
            other => {
                return Err(anyhow!(
                    "unsupported Atlas input request type '{other}' for rmcp conversion"
                ));
            }
        };
        requests.insert(id, request);
    }
    Ok(requests)
}

fn prompt_message_from_atlas_value(value: &Value) -> Result<PromptMessage> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("prompt message entries must be objects"))?;
    let role = match required_string_field(object, "role")? {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        other => return Err(anyhow!("unsupported prompt role '{other}'")),
    };
    let content = object
        .get("content")
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("prompt message missing content object"))?;
    let kind = required_string_field(content, "type")?;
    if kind != "text" {
        return Err(anyhow!("unsupported prompt content type '{kind}'"));
    }
    let text = required_string_field(content, "text")?;
    Ok(PromptMessage::new_text(role, text))
}

pub(crate) fn schema_object_from_value(value: Value) -> Result<Arc<JsonObject>> {
    match value {
        Value::Object(object) => Ok(Arc::new(object)),
        other => Err(anyhow!("expected schema object, got {other}")),
    }
}

fn optional_meta_from_parent(value: &Value) -> Result<Option<MetaObject>> {
    match value.get("_meta") {
        None => Ok(None),
        Some(meta) => meta_object_from_value(meta.clone()),
    }
}

fn task_handle_from_record(task: &DurableTaskRecord) -> Task {
    let mut rmcp_task = Task::new(
        task.task_id.clone(),
        task_status_from_durable(task.status),
        task.created_at.clone(),
        task.updated_at.clone(),
    );
    if let Some(status_message) = task.status_message.as_ref() {
        rmcp_task = rmcp_task.with_status_message(status_message.clone());
    }
    if let Some(ttl_ms) = task.ttl_ms {
        rmcp_task = rmcp_task.with_ttl_ms(ttl_ms);
    }
    rmcp_task.with_poll_interval_ms(1_000)
}

fn task_meta_from_record(task: &DurableTaskRecord) -> Option<MetaObject> {
    let mut object = serde_json::Map::new();
    if let Some(progress) = task.progress.clone() {
        object.insert(ATLAS_TASK_META_PROGRESS.to_owned(), progress);
    }
    if task.cancel_requested {
        object.insert(
            ATLAS_TASK_META_CANCEL_REQUESTED.to_owned(),
            Value::Bool(task.cancel_requested),
        );
    }
    if let Some(request_state) = task.request_state.clone() {
        object.insert(
            ATLAS_TASK_META_REQUEST_STATE.to_owned(),
            Value::String(request_state),
        );
    }
    (!object.is_empty()).then_some(MetaObject(object))
}

fn task_notification_meta_from_record(task: &DurableTaskRecord) -> Option<NotificationMetaObject> {
    task_meta_from_record(task).map(Into::into)
}

fn task_status_from_durable(status: DurableTaskStatus) -> TaskStatus {
    match status {
        DurableTaskStatus::Working => TaskStatus::Working,
        DurableTaskStatus::InputRequired => TaskStatus::InputRequired,
        DurableTaskStatus::Completed => TaskStatus::Completed,
        DurableTaskStatus::Failed => TaskStatus::Failed,
        DurableTaskStatus::Cancelled => TaskStatus::Cancelled,
    }
}

fn json_object_from_value(value: Value) -> Result<JsonObject> {
    match value {
        Value::Object(object) => Ok(object),
        Value::Null => Ok(Default::default()),
        other => Err(anyhow!("expected JSON object, got {other}")),
    }
}

fn parse_task_status(value: &str) -> Result<TaskStatus> {
    match value {
        "working" => Ok(TaskStatus::Working),
        "input_required" => Ok(TaskStatus::InputRequired),
        "completed" => Ok(TaskStatus::Completed),
        "failed" => Ok(TaskStatus::Failed),
        "cancelled" => Ok(TaskStatus::Cancelled),
        other => Err(anyhow!("unsupported task status '{other}'")),
    }
}

fn required_string_field<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Result<&'a str> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing string field '{field}'"))
}

fn optional_string_field<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
) -> Option<&'a str> {
    object.get(field).and_then(Value::as_str)
}

fn optional_u64_field(object: &serde_json::Map<String, Value>, field: &str) -> Option<u64> {
    object.get(field).and_then(Value::as_u64)
}

fn icons_from_descriptors(icons: Vec<IconDescriptor>) -> Result<Vec<Icon>> {
    icons
        .into_iter()
        .map(icon_from_descriptor)
        .collect::<Result<Vec<_>>>()
}

fn icon_from_descriptor(descriptor: IconDescriptor) -> Result<Icon> {
    serde_json::from_value(serde_json::to_value(descriptor)?).map_err(Into::into)
}
