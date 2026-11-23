use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use atlas_session::{
    DurableTaskRecord, DurableTaskStatus, DurableTaskUpdate, NewDurableTask, SessionStore,
};
use serde_json::{Value, json};

use crate::output::OutputFormat;
use crate::progress;
use crate::runtime_context::{self, RequestContext};
use crate::tool_result::normalize_tool_execution_error;

pub(crate) type TaskApiResult<T> = std::result::Result<T, TaskApiError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TaskApiErrorKind {
    #[allow(dead_code)]
    Cancelled,
    #[allow(dead_code)]
    Failed,
    Internal,
    InvalidParams,
    NotFound,
    #[allow(dead_code)]
    NotReady,
}

#[derive(Debug)]
pub(crate) struct TaskApiError {
    kind: TaskApiErrorKind,
    source: anyhow::Error,
}

impl TaskApiError {
    fn new(kind: TaskApiErrorKind, source: anyhow::Error) -> Self {
        Self { kind, source }
    }

    pub(crate) fn kind(&self) -> TaskApiErrorKind {
        self.kind
    }

    #[cfg_attr(not(feature = "http-transport"), allow(dead_code))]
    pub(crate) fn message(&self) -> String {
        self.source.to_string()
    }

    pub(crate) fn into_anyhow(self) -> anyhow::Error {
        self.source
    }
}

const TASK_DEFER_THRESHOLD_MS_ENV: &str = "ATLAS_MCP_TASK_DEFER_THRESHOLD_MS";
const DEFAULT_TASK_DEFER_THRESHOLD_MS: u64 = 1_000;
const DEFAULT_TASK_POLL_INTERVAL_MS: u64 = 1_000;

static TASK_ID_COUNTER: AtomicU64 = AtomicU64::new(0);
static LIVE_TASKS: OnceLock<Mutex<HashMap<String, LiveTaskHandle>>> = OnceLock::new();

thread_local! {
    static TOOL_CALL_PARAMS: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
    #[cfg(test)]
    static TEST_AUTO_DEFER_TOOLS: std::cell::RefCell<std::collections::HashSet<String>> = std::cell::RefCell::new(std::collections::HashSet::new());
    #[cfg(test)]
    static TEST_DEFER_THRESHOLD_MS: std::cell::RefCell<Option<u64>> = const { std::cell::RefCell::new(None) };
}

#[derive(Clone)]
struct LiveTaskHandle {
    cancel_flag: Arc<std::sync::atomic::AtomicBool>,
}

struct TaskExecutionTarget {
    repo_root: String,
    db_path: String,
    worker_threads: usize,
}

#[cfg(test)]
pub(crate) fn execute_tool_call(
    name: &str,
    args: Option<Value>,
    repo_root: &str,
    db_path: &str,
) -> Result<Value> {
    execute_tool_call_with_worker_threads(name, args, repo_root, db_path, 2)
}

fn required_task_id(params: Option<&Value>) -> TaskApiResult<&str> {
    params
        .and_then(|value| value.get("taskId"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            task_api_error(
                TaskApiErrorKind::InvalidParams,
                "missing required argument: taskId",
            )
        })
}

#[cfg(test)]
pub(crate) fn tasks_get(
    params: Option<&Value>,
    repo_root: &str,
    _output_format: OutputFormat,
) -> TaskApiResult<Value> {
    let task_id = required_task_id(params)?;
    let task = task_record(repo_root, task_id)?;
    Ok(task_state_json(&task))
}

fn live_tasks() -> &'static Mutex<HashMap<String, LiveTaskHandle>> {
    LIVE_TASKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn new_task_id(request_id: &str, tool_name: &str) -> String {
    let counter = TASK_ID_COUNTER.fetch_add(1, Ordering::Relaxed) + 1;
    format!("atlas-task-{request_id}-{tool_name}-{counter}")
}

fn resolve_defer_threshold_ms() -> u64 {
    #[cfg(test)]
    if let Some(value) = TEST_DEFER_THRESHOLD_MS.with(|threshold| *threshold.borrow()) {
        return value;
    }

    std::env::var(TASK_DEFER_THRESHOLD_MS_ENV)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_TASK_DEFER_THRESHOLD_MS)
}

fn create_task_result(task: &DurableTaskRecord) -> Value {
    json!({ "task": task_handle_json(task) })
}

fn is_input_required_tool_result(value: &Value) -> bool {
    value.get("resultType").and_then(Value::as_str) == Some("input_required")
}

fn normalize_tool_call_result(
    tool_name: &str,
    args: Option<&Value>,
    result: Result<Value>,
) -> Result<Value> {
    match result {
        Ok(value) => {
            if runtime_context::current().is_ok()
                || value.get("isError") == Some(&Value::Bool(true))
            {
                Ok(value)
            } else {
                Ok(crate::tool_result::structured_content(&value)
                    .cloned()
                    .unwrap_or(value))
            }
        }
        Err(error) => {
            let _ = args;
            normalize_tool_execution_error(tool_name, OutputFormat::Json, error)
        }
    }
}

fn task_handle_json(task: &DurableTaskRecord) -> Value {
    json!({
        "taskId": task.task_id,
        "originatingMethod": task.originating_method,
        "toolName": task.tool_name,
        "status": task.status.as_str(),
        "statusMessage": task.status_message,
        "createdAt": task.created_at,
        "lastUpdatedAt": task.updated_at,
        "ttl": task.ttl_ms,
        "pollInterval": DEFAULT_TASK_POLL_INTERVAL_MS,
    })
}

fn task_state_json(task: &DurableTaskRecord) -> Value {
    let mut value = json!({
        "task": task_handle_json(task),
        "status": task.status.as_str(),
        "statusMessage": task.status_message,
        "cancelRequested": task.cancel_requested,
        "progress": task.progress,
    });
    if let Some(object) = value.as_object_mut() {
        match task.status {
            DurableTaskStatus::Completed => {
                object.insert(
                    "result".to_owned(),
                    task.result.clone().unwrap_or(Value::Null),
                );
            }
            DurableTaskStatus::Failed => {
                object.insert(
                    "error".to_owned(),
                    task.error.clone().unwrap_or_else(
                        || json!({"message": format!("task '{}' failed", task.task_id)}),
                    ),
                );
            }
            DurableTaskStatus::InputRequired => {
                object.insert("inputRequired".to_owned(), Value::Bool(true));
                if let Some(input_requests) = task.input_requests.clone() {
                    object.insert("inputRequests".to_owned(), input_requests);
                }
                if let Some(request_state) = task.request_state.clone() {
                    object.insert("requestState".to_owned(), Value::String(request_state));
                }
            }
            DurableTaskStatus::Cancelled | DurableTaskStatus::Working => {}
        }
    }
    value
}

fn task_status_notification_json(task: &DurableTaskRecord) -> Value {
    if let Ok(notification) = crate::rmcp_types::task_status_notification_from_record(task)
        && let Ok(value) = serde_json::to_value(notification)
    {
        return value;
    }

    let mut value = task_state_json(task);
    if let Some(object) = value.as_object_mut() {
        object.insert("source".to_owned(), json!("tasks"));
    }
    value
}

fn fallback_task_status(task_id: &str, status_message: &str) -> DurableTaskRecord {
    DurableTaskRecord {
        task_id: task_id.to_owned(),
        originating_method: "tools/call".to_owned(),
        request_id: None,
        tool_name: None,
        transport_kind: None,
        session_id: None,
        created_at: String::new(),
        updated_at: String::new(),
        status: DurableTaskStatus::Working,
        status_message: Some(status_message.to_owned()),
        progress: None,
        result: None,
        error: None,
        input_requests: None,
        request_state: None,
        ttl_ms: None,
        cancel_requested: false,
    }
}

fn is_auto_defer_candidate(name: &str) -> bool {
    matches!(
        name,
        "build_graph"
            | "update_graph"
            | "postprocess_graph"
            | "doctor"
            | "analyze_architecture"
            | "analyze_metrics"
            | "assess_risk"
            | "analyze_patterns"
            | "find_large_functions"
            | "find_complex_functions"
            | "find_similar_functions"
            | "find_duplicates"
            | "infer_modules"
            | "label_components"
            | "analyze_dead_code"
            | "analyze_remove"
            | "analyze_safety"
            | "analyze_dependency"
    ) || test_auto_defer_tools_contains(name)
}

#[cfg(test)]
fn install_test_auto_defer_tool(name: &str) {
    TEST_AUTO_DEFER_TOOLS.with(|tools| {
        tools.borrow_mut().insert(name.to_owned());
    });
}

#[cfg(test)]
fn uninstall_test_auto_defer_tool(name: &str) {
    TEST_AUTO_DEFER_TOOLS.with(|tools| {
        tools.borrow_mut().remove(name);
    });
}

#[cfg(test)]
fn install_test_defer_threshold_ms(value: u64) {
    TEST_DEFER_THRESHOLD_MS.with(|threshold| {
        *threshold.borrow_mut() = Some(value);
    });
}

#[cfg(test)]
fn uninstall_test_defer_threshold_ms() {
    TEST_DEFER_THRESHOLD_MS.with(|threshold| {
        *threshold.borrow_mut() = None;
    });
}

#[cfg(test)]
fn test_auto_defer_tools_contains(name: &str) -> bool {
    TEST_AUTO_DEFER_TOOLS.with(|tools| tools.borrow().contains(name))
}

#[cfg(not(test))]
fn test_auto_defer_tools_contains(_name: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn wire_json_matches_task_shape() {
        let task = DurableTaskRecord {
            task_id: "task-1".to_owned(),
            originating_method: "tools/call".to_owned(),
            request_id: Some("1".to_owned()),
            tool_name: Some("doctor".to_owned()),
            transport_kind: Some("stdio".to_owned()),
            session_id: None,
            created_at: "2026-01-01T00:00:00Z".to_owned(),
            updated_at: "2026-01-01T00:00:01Z".to_owned(),
            status: DurableTaskStatus::Working,
            status_message: Some("working".to_owned()),
            progress: None,
            result: None,
            error: None,
            input_requests: None,
            request_state: None,
            ttl_ms: Some(5000),
            cancel_requested: false,
        };
        assert_eq!(task_handle_json(&task)["taskId"], json!("task-1"));
        assert_eq!(task_handle_json(&task)["status"], json!("working"));
        assert_eq!(task_state_json(&task)["status"], json!("working"));
    }

    #[test]
    fn task_status_notification_uses_rmcp_meta_for_progress_boundary() {
        let task = DurableTaskRecord {
            task_id: "task-progress".to_owned(),
            originating_method: "tools/call".to_owned(),
            request_id: Some("1".to_owned()),
            tool_name: Some("doctor".to_owned()),
            transport_kind: Some("rmcp".to_owned()),
            session_id: None,
            created_at: "2026-01-01T00:00:00Z".to_owned(),
            updated_at: "2026-01-01T00:00:05Z".to_owned(),
            status: DurableTaskStatus::Working,
            status_message: Some("indexing".to_owned()),
            progress: Some(json!({"message": "indexed files", "percentage": 55})),
            result: None,
            error: None,
            input_requests: None,
            request_state: None,
            ttl_ms: Some(5000),
            cancel_requested: false,
        };

        let notification = task_status_notification_json(&task);
        assert_eq!(notification["taskId"], json!("task-progress"));
        assert_eq!(notification["status"], json!("working"));
        assert_eq!(
            notification["_meta"][crate::rmcp_types::ATLAS_TASK_META_PROGRESS],
            json!({"message": "indexed files", "percentage": 55})
        );
        assert!(notification.get("progress").is_none());
    }

    #[test]
    fn new_task_id_is_unique() {
        let a = new_task_id("1", "doctor");
        let b = new_task_id("1", "doctor");
        assert_ne!(a, b);
    }

    #[test]
    fn store_round_trip_list_and_result() {
        let dir = TempDir::new().unwrap();
        let mut store = SessionStore::open_in_repo(dir.path()).unwrap();
        let created = store
            .create_durable_task(&NewDurableTask {
                task_id: "task-1".to_owned(),
                originating_method: "tools/call".to_owned(),
                request_id: Some("1".to_owned()),
                tool_name: Some("doctor".to_owned()),
                transport_kind: Some("stdio".to_owned()),
                session_id: None,
                status: DurableTaskStatus::Working,
                status_message: Some("working".to_owned()),
                ttl_ms: Some(1000),
            })
            .unwrap();
        assert_eq!(created.task_id, "task-1");
        store
            .update_durable_task(
                "task-1",
                &DurableTaskUpdate {
                    status: Some(DurableTaskStatus::Completed),
                    result: Some(json!({"ok": true})),
                    ..Default::default()
                },
            )
            .unwrap();
        let page = store.list_durable_tasks(None, 10).unwrap();
        assert_eq!(page.tasks.len(), 1);
        assert_eq!(page.tasks[0].status, DurableTaskStatus::Completed);
    }

    fn install_task_context(supports_tasks: bool) {
        runtime_context::install(runtime_context::RequestContext::new(
            Arc::new(|_| Ok(())),
            runtime_context::ClientInteractionCapabilities {
                supports_elicitation_form: false,
                supports_elicitation_url: false,
                supports_tasks,
            },
            "test",
            None,
            None,
            "test-request",
            "tools/call",
            None,
        ));
    }

    #[test]
    fn explicit_task_create_poll_result_and_reopen_work() {
        let dir = TempDir::new().unwrap();
        install_tool_call_request_params(Some(&json!({"task": {"ttl": 1000}})));
        let created = execute_tool_call(
            "__test_sleep",
            Some(json!({"sleep_ms": 25})),
            dir.path().to_str().unwrap(),
            "db",
        )
        .unwrap();
        uninstall_tool_call_request_params();

        let task_id = created["task"]["taskId"].as_str().unwrap().to_owned();
        for _ in 0..50 {
            let task = tasks_get(
                Some(&json!({"taskId": &task_id})),
                dir.path().to_str().unwrap(),
                OutputFormat::Json,
            )
            .unwrap();
            if task["status"] == json!("completed") {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let result = tasks_get(
            Some(&json!({"taskId": &task_id})),
            dir.path().to_str().unwrap(),
            OutputFormat::Json,
        )
        .unwrap();
        assert_eq!(result["result"]["slept_ms"], json!(25));

        let reopened = SessionStore::open_in_repo(dir.path())
            .unwrap()
            .get_durable_task(&task_id)
            .unwrap()
            .unwrap();
        assert_eq!(reopened.status, DurableTaskStatus::Completed);
    }

    #[test]
    fn tasks_update_accepts_client_input_for_input_required_task() {
        let dir = TempDir::new().unwrap();
        create_task_with_status(&dir, "task-input", DurableTaskStatus::InputRequired, None);
        let updated = tasks_update(
            Some(&json!({"taskId": "task-input", "input": {"answer": "yes"}})),
            dir.path().to_str().unwrap(),
            OutputFormat::Json,
        )
        .unwrap();
        assert_eq!(updated["accepted"], json!(true));
        let task = tasks_get(
            Some(&json!({"taskId": "task-input"})),
            dir.path().to_str().unwrap(),
            OutputFormat::Json,
        )
        .unwrap();
        assert_eq!(task["status"], json!("working"));
        assert_eq!(task["progress"]["clientInput"]["answer"], json!("yes"));
    }

    fn create_task_with_status(
        dir: &TempDir,
        task_id: &str,
        status: DurableTaskStatus,
        error_message: Option<&str>,
    ) {
        let mut store = SessionStore::open_in_repo(dir.path()).unwrap();
        store
            .create_durable_task(&NewDurableTask {
                task_id: task_id.to_owned(),
                originating_method: "tools/call".to_owned(),
                request_id: Some("1".to_owned()),
                tool_name: Some("doctor".to_owned()),
                transport_kind: Some("stdio".to_owned()),
                session_id: None,
                status: DurableTaskStatus::Working,
                status_message: Some("working".to_owned()),
                ttl_ms: Some(1000),
            })
            .unwrap();
        store
            .update_durable_task(
                task_id,
                &DurableTaskUpdate {
                    status: Some(status),
                    status_message: Some(status.as_str().to_owned()),
                    error: error_message.map(|message| json!({ "message": message })),
                    ..Default::default()
                },
            )
            .unwrap();
    }

    #[test]
    fn tasks_get_unknown_task_uses_not_found_error_kind() {
        let dir = TempDir::new().unwrap();
        let error = tasks_get(
            Some(&json!({"taskId": "missing"})),
            dir.path().to_str().unwrap(),
            OutputFormat::Json,
        )
        .unwrap_err();
        assert_eq!(error.kind(), TaskApiErrorKind::NotFound);
        assert!(error.message().contains("unknown task_id 'missing'"));
    }

    #[test]
    fn tasks_get_includes_terminal_error_state() {
        let dir = TempDir::new().unwrap();
        create_task_with_status(&dir, "task-failed", DurableTaskStatus::Failed, Some("boom"));
        let task = tasks_get(
            Some(&json!({"taskId": "task-failed"})),
            dir.path().to_str().unwrap(),
            OutputFormat::Json,
        )
        .unwrap();
        assert_eq!(task["status"], json!("failed"));
        assert_eq!(task["error"]["message"], json!("boom"));
    }

    #[test]
    fn tasks_update_rejects_terminal_task() {
        let dir = TempDir::new().unwrap();
        create_task_with_status(&dir, "task-cancelled", DurableTaskStatus::Cancelled, None);
        let error = tasks_update(
            Some(&json!({"taskId": "task-cancelled", "input": {"answer": "no"}})),
            dir.path().to_str().unwrap(),
            OutputFormat::Json,
        )
        .unwrap_err();
        assert_eq!(error.kind(), TaskApiErrorKind::InvalidParams);
        assert!(error.message().contains("no longer accepts updates"));
    }

    #[test]
    fn implicit_auto_defer_short_run_skips_task_persistence() {
        let dir = TempDir::new().unwrap();
        install_task_context(true);
        install_test_auto_defer_tool("__test_sleep");
        install_test_defer_threshold_ms(DEFAULT_TASK_DEFER_THRESHOLD_MS);
        let result = execute_tool_call(
            "__test_sleep",
            Some(json!({"sleep_ms": 5})),
            dir.path().to_str().unwrap(),
            "db",
        )
        .unwrap();
        uninstall_test_defer_threshold_ms();
        uninstall_test_auto_defer_tool("__test_sleep");
        runtime_context::uninstall();

        assert_eq!(result["structuredContent"]["slept_ms"], json!(5));
        let page = SessionStore::open_in_repo(dir.path())
            .unwrap()
            .list_durable_tasks(None, 10)
            .unwrap();
        assert!(page.tasks.is_empty());
    }

    #[test]
    fn implicit_auto_defer_without_task_capability_runs_synchronously() {
        let dir = TempDir::new().unwrap();
        install_task_context(false);
        install_test_auto_defer_tool("__test_sleep");
        install_test_defer_threshold_ms(10);
        let result = execute_tool_call(
            "__test_sleep",
            Some(json!({"sleep_ms": 25})),
            dir.path().to_str().unwrap(),
            "db",
        )
        .unwrap();
        uninstall_test_defer_threshold_ms();
        uninstall_test_auto_defer_tool("__test_sleep");
        runtime_context::uninstall();

        assert_eq!(result["structuredContent"]["slept_ms"], json!(25));
        let page = SessionStore::open_in_repo(dir.path())
            .unwrap()
            .list_durable_tasks(None, 10)
            .unwrap();
        assert!(page.tasks.is_empty());
    }

    #[test]
    fn implicit_auto_defer_returns_task_handle_after_threshold() {
        let dir = TempDir::new().unwrap();
        install_task_context(true);
        install_test_auto_defer_tool("__test_sleep");
        install_test_defer_threshold_ms(10);
        let created = execute_tool_call(
            "__test_sleep",
            Some(json!({"sleep_ms": 100})),
            dir.path().to_str().unwrap(),
            "db",
        )
        .unwrap();
        uninstall_test_defer_threshold_ms();
        uninstall_test_auto_defer_tool("__test_sleep");
        runtime_context::uninstall();

        let task_id = created["task"]["taskId"].as_str().unwrap().to_owned();
        assert_eq!(created["task"]["status"], json!("working"));
        for _ in 0..50 {
            let task = tasks_get(
                Some(&json!({"taskId": &task_id})),
                dir.path().to_str().unwrap(),
                OutputFormat::Json,
            )
            .unwrap();
            if task["status"] == json!("completed") {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let result = tasks_get(
            Some(&json!({"taskId": &task_id})),
            dir.path().to_str().unwrap(),
            OutputFormat::Json,
        )
        .unwrap();
        assert_eq!(
            result["result"]["structuredContent"]["slept_ms"],
            json!(100)
        );
    }

    #[test]
    fn explicit_task_tool_failure_is_persisted_as_is_error_result() {
        let dir = TempDir::new().unwrap();
        install_tool_call_request_params(Some(&json!({"task": {"ttl": 1000}})));
        let created = execute_tool_call(
            "search_content",
            Some(json!({"query": "(", "is_regex": true})),
            dir.path().to_str().unwrap(),
            "db",
        )
        .unwrap();
        uninstall_tool_call_request_params();

        let task_id = created["task"]["taskId"].as_str().unwrap().to_owned();
        for _ in 0..50 {
            let task = tasks_get(
                Some(&json!({"taskId": &task_id})),
                dir.path().to_str().unwrap(),
                OutputFormat::Json,
            )
            .unwrap();
            if task["status"] == json!("completed") {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let result = tasks_get(
            Some(&json!({"taskId": &task_id})),
            dir.path().to_str().unwrap(),
            OutputFormat::Json,
        )
        .unwrap();
        assert_eq!(result["result"]["isError"], json!(true));
        assert_eq!(
            result["result"]["structuredContent"]["code"],
            json!("invalid_input")
        );
        assert!(
            result["result"]["structuredContent"]["message"]
                .as_str()
                .expect("message")
                .contains("invalid regex pattern")
        );
    }

    #[test]
    fn normalize_tool_call_result_maps_missing_file_to_file_not_found_code() {
        let dir = TempDir::new().unwrap();
        let result = execute_tool_call(
            "read_file_around_match",
            Some(json!({"file": "src/missing.rs", "query": "needle"})),
            dir.path().to_str().unwrap(),
            "db",
        )
        .unwrap();

        assert_eq!(result["isError"], json!(true));
        assert_eq!(result["structuredContent"]["code"], json!("file_not_found"));
        assert_eq!(
            result["structuredContent"]["details"]["path"],
            json!("src/missing.rs")
        );
    }
}

mod exec;

use self::exec::task_api_error;
pub(crate) use self::exec::task_record;

pub(crate) use self::exec::execute_tool_call_with_worker_threads;
pub(crate) use self::exec::install_tool_call_request_params;
pub(crate) use self::exec::task_ttl_from_request_params;
pub(crate) use self::exec::tasks_cancel;
pub(crate) use self::exec::tasks_update;
pub(crate) use self::exec::uninstall_tool_call_request_params;
