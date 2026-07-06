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
use crate::runtime_context::{self, ReverseRequestClient};

pub(crate) type TaskApiResult<T> = std::result::Result<T, TaskApiError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TaskApiErrorKind {
    Cancelled,
    Failed,
    Internal,
    InvalidParams,
    NotFound,
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

pub(crate) fn execute_tool_call(
    name: &str,
    args: Option<Value>,
    repo_root: &str,
    db_path: &str,
) -> Result<Value> {
    let defer_threshold = Duration::from_millis(resolve_defer_threshold_ms());
    let request_context = runtime_context::current().ok();
    let explicit_task = task_ttl_from_context_args();
    let auto_defer = explicit_task.is_none() && is_auto_defer_candidate(name);
    if explicit_task.is_none() && !auto_defer {
        return crate::tools::call(name, args.as_ref(), repo_root, db_path);
    }

    let request_id = request_context
        .as_ref()
        .map(|ctx| ctx.originating_request_id.clone())
        .unwrap_or_else(|| "unknown".to_owned());
    let task_id = new_task_id(&request_id, name);
    let transport_kind = request_context
        .as_ref()
        .map(|ctx| ctx.transport_kind.clone());
    let session_id = request_context
        .as_ref()
        .and_then(|ctx| ctx.session_id.clone());
    let task = if explicit_task.is_some() {
        Some(create_task_record(
            repo_root,
            &task_id,
            request_id.clone(),
            name,
            transport_kind.clone(),
            session_id.clone(),
            explicit_task,
        )?)
    } else {
        None
    };
    let cancel_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
    live_tasks()
        .lock()
        .expect("live tasks lock poisoned")
        .insert(
            task_id.clone(),
            LiveTaskHandle {
                cancel_flag: Arc::clone(&cancel_flag),
            },
        );

    let (completion_tx, completion_rx) = mpsc::channel();
    let tool_name = name.to_owned();
    let spawn_name = tool_name.clone();
    let repo_root_owned = repo_root.to_owned();
    let db_path_owned = db_path.to_owned();
    let request_context_for_thread = request_context.clone();
    let task_id_for_thread = task_id.clone();
    let worker_tool_name = tool_name.clone();
    thread::Builder::new()
        .name(format!("atlas-mcp:task:{task_id}"))
        .spawn(move || {
            let result = run_task_worker(
                &task_id_for_thread,
                &worker_tool_name,
                args,
                &repo_root_owned,
                &db_path_owned,
                cancel_flag,
                request_context_for_thread,
            );
            let _ = completion_tx.send(result);
            live_tasks()
                .lock()
                .expect("live tasks lock poisoned")
                .remove(&task_id_for_thread);
        })
        .with_context(|| format!("cannot spawn durable task thread for {spawn_name}"))?;

    if let Some(task) = task.as_ref() {
        return Ok(create_task_result(task));
    }

    match completion_rx.recv_timeout(defer_threshold) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let task = create_task_record(
                repo_root,
                &task_id,
                request_id,
                &tool_name,
                transport_kind,
                session_id,
                None,
            )?;
            Ok(create_task_result(&task))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err(anyhow!("durable task worker disconnected unexpectedly"))
        }
    }
}

fn run_task_worker(
    task_id: &str,
    tool_name: &str,
    args: Option<Value>,
    repo_root: &str,
    db_path: &str,
    cancel_flag: Arc<std::sync::atomic::AtomicBool>,
    request_context: Option<ReverseRequestClient>,
) -> Result<Value> {
    if let Some(client) = request_context.clone() {
        runtime_context::install(client);
    }
    let repo_root_for_progress = repo_root.to_owned();
    let task_id_for_progress = task_id.to_owned();
    let request_context_for_progress = request_context.clone();
    progress::install(
        move |message, percentage| {
            let snapshot = json!({
                "message": message,
                "percentage": percentage,
            });
            let _ = update_task_store(
                &repo_root_for_progress,
                &task_id_for_progress,
                DurableTaskUpdate {
                    status: Some(DurableTaskStatus::Working),
                    status_message: Some(message.to_owned()),
                    progress: Some(snapshot),
                    ..Default::default()
                },
            );
            if let Some(client) = request_context_for_progress.as_ref() {
                let task = open_task_record(&repo_root_for_progress, &task_id_for_progress)
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| fallback_task_status(&task_id_for_progress, message));
                let _ = client.notify_task_status(task_wire_json(&task));
            }
        },
        cancel_flag,
    );
    let result = crate::tools::call(tool_name, args.as_ref(), repo_root, db_path);
    progress::uninstall();
    runtime_context::uninstall();

    let current = open_task_record(repo_root, task_id)?;
    let already_cancelled = current
        .as_ref()
        .is_some_and(|task| task.status == DurableTaskStatus::Cancelled);
    match result {
        Ok(value) if current.is_some() && !already_cancelled => {
            let update = DurableTaskUpdate {
                status: Some(DurableTaskStatus::Completed),
                status_message: Some("completed".to_owned()),
                result: Some(value.clone()),
                ..Default::default()
            };
            update_task_store(repo_root, task_id, update)?;
            if let Some(client) = request_context.as_ref()
                && let Some(task) = open_task_record(repo_root, task_id)?
            {
                client.notify_task_status(task_wire_json(&task))?;
            }
            Ok(value)
        }
        Ok(value) => Ok(value),
        Err(error) if already_cancelled => Err(error),
        Err(error) if current.is_some() => {
            let error_json = json!({ "message": error.to_string() });
            let update = DurableTaskUpdate {
                status: Some(DurableTaskStatus::Failed),
                status_message: Some(error.to_string()),
                error: Some(error_json),
                ..Default::default()
            };
            update_task_store(repo_root, task_id, update)?;
            if let Some(client) = request_context.as_ref()
                && let Some(task) = open_task_record(repo_root, task_id)?
            {
                client.notify_task_status(task_wire_json(&task))?;
            }
            Err(error)
        }
        Err(error) => Err(error),
    }
}

fn create_task_record(
    repo_root: &str,
    task_id: &str,
    request_id: String,
    tool_name: &str,
    transport_kind: Option<String>,
    session_id: Option<String>,
    ttl_ms: Option<u64>,
) -> Result<DurableTaskRecord> {
    let mut store = SessionStore::open_in_repo(repo_root)?;
    Ok(store.create_durable_task(&NewDurableTask {
        task_id: task_id.to_owned(),
        originating_method: "tools/call".to_owned(),
        request_id: Some(request_id),
        tool_name: Some(tool_name.to_owned()),
        transport_kind,
        session_id,
        status: DurableTaskStatus::Working,
        status_message: Some("working".to_owned()),
        ttl_ms,
    })?)
}

fn update_task_store(repo_root: &str, task_id: &str, update: DurableTaskUpdate) -> Result<()> {
    let mut store = SessionStore::open_in_repo(repo_root)?;
    Ok(store.update_durable_task(task_id, &update)?)
}

fn open_task_record(repo_root: &str, task_id: &str) -> Result<Option<DurableTaskRecord>> {
    let store = SessionStore::open_in_repo(repo_root)?;
    Ok(store.get_durable_task(task_id)?)
}

fn task_ttl_from_context_args() -> Option<u64> {
    current_tool_call_request_params()
        .and_then(|params| params.get("task").cloned())
        .and_then(|task| task.get("ttl").and_then(Value::as_u64))
}

fn current_tool_call_request_params() -> Option<Value> {
    TOOL_CALL_PARAMS.with(|slot| slot.borrow().clone())
}

pub(crate) fn install_tool_call_request_params(params: Option<&Value>) {
    TOOL_CALL_PARAMS.with(|slot| *slot.borrow_mut() = params.cloned());
}

pub(crate) fn uninstall_tool_call_request_params() {
    TOOL_CALL_PARAMS.with(|slot| *slot.borrow_mut() = None);
}

pub(crate) fn tasks_list(
    params: Option<&Value>,
    repo_root: &str,
    _output_format: OutputFormat,
) -> TaskApiResult<Value> {
    let cursor = params
        .and_then(|value| value.get("cursor"))
        .and_then(Value::as_str);
    let store = SessionStore::open_in_repo(repo_root)
        .map_err(|error| TaskApiError::new(TaskApiErrorKind::Internal, error.into()))?;
    let page = store
        .list_durable_tasks(cursor, 20)
        .map_err(classify_task_store_error)?;
    Ok(json!({
        "tasks": page.tasks.iter().map(task_wire_json).collect::<Vec<_>>(),
        "nextCursor": page.next_cursor,
    }))
}

pub(crate) fn tasks_get(
    params: Option<&Value>,
    repo_root: &str,
    _output_format: OutputFormat,
) -> TaskApiResult<Value> {
    let task_id = required_task_id(params)?;
    let task = open_task_record(repo_root, task_id)
        .map_err(|error| TaskApiError::new(TaskApiErrorKind::Internal, error))?
        .ok_or_else(|| {
            task_api_error(
                TaskApiErrorKind::NotFound,
                format!("unknown task_id '{task_id}'"),
            )
        })?;
    Ok(task_wire_json(&task))
}

pub(crate) fn tasks_result(
    params: Option<&Value>,
    repo_root: &str,
    _output_format: OutputFormat,
) -> TaskApiResult<Value> {
    let task_id = required_task_id(params)?;
    let task = open_task_record(repo_root, task_id)
        .map_err(|error| TaskApiError::new(TaskApiErrorKind::Internal, error))?
        .ok_or_else(|| {
            task_api_error(
                TaskApiErrorKind::NotFound,
                format!("unknown task_id '{task_id}'"),
            )
        })?;
    match task.status {
        DurableTaskStatus::Completed => task.result.ok_or_else(|| {
            task_api_error(
                TaskApiErrorKind::Internal,
                format!("task '{task_id}' completed without stored result"),
            )
        }),
        DurableTaskStatus::Failed => Err(task_api_error(
            TaskApiErrorKind::Failed,
            task.error
                .and_then(|value| {
                    value
                        .get("message")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| format!("task '{task_id}' failed")),
        )),
        DurableTaskStatus::Cancelled => Err(task_api_error(
            TaskApiErrorKind::Cancelled,
            format!("task '{task_id}' was cancelled"),
        )),
        DurableTaskStatus::InputRequired => Err(task_api_error(
            TaskApiErrorKind::NotReady,
            format!("task '{task_id}' requires additional input"),
        )),
        DurableTaskStatus::Working => Err(task_api_error(
            TaskApiErrorKind::NotReady,
            format!("task '{task_id}' is still working"),
        )),
    }
}

pub(crate) fn tasks_cancel(
    params: Option<&Value>,
    repo_root: &str,
    _output_format: OutputFormat,
) -> TaskApiResult<Value> {
    let task_id = required_task_id(params)?;
    if let Some(handle) = live_tasks()
        .lock()
        .expect("live tasks lock poisoned")
        .get(task_id)
        .cloned()
    {
        handle
            .cancel_flag
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    update_task_store(
        repo_root,
        task_id,
        DurableTaskUpdate {
            status: Some(DurableTaskStatus::Cancelled),
            status_message: Some("cancelled".to_owned()),
            cancel_requested: Some(true),
            ..Default::default()
        },
    )
    .map_err(|error| TaskApiError::new(TaskApiErrorKind::Internal, error))?;
    let task = open_task_record(repo_root, task_id)
        .map_err(|error| TaskApiError::new(TaskApiErrorKind::Internal, error))?
        .ok_or_else(|| {
            task_api_error(
                TaskApiErrorKind::NotFound,
                format!("unknown task_id '{task_id}'"),
            )
        })?;
    if let Ok(client) = runtime_context::current() {
        let _ = client.notify_task_status(task_wire_json(&task));
    }
    Ok(task_wire_json(&task))
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

fn classify_task_store_error(error: atlas_core::AtlasError) -> TaskApiError {
    let detail = error.to_string();
    let kind = if detail.starts_with("invalid durable task cursor:") {
        TaskApiErrorKind::InvalidParams
    } else {
        TaskApiErrorKind::Internal
    };
    TaskApiError::new(kind, error.into())
}

fn task_api_error(kind: TaskApiErrorKind, message: impl Into<String>) -> TaskApiError {
    TaskApiError::new(kind, anyhow!(message.into()))
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
    json!({ "task": task_wire_json(task) })
}

fn task_wire_json(task: &DurableTaskRecord) -> Value {
    json!({
        "taskId": task.task_id,
        "status": task.status.as_str(),
        "statusMessage": task.status_message,
        "createdAt": task.created_at,
        "lastUpdatedAt": task.updated_at,
        "ttl": task.ttl_ms,
        "pollInterval": DEFAULT_TASK_POLL_INTERVAL_MS,
    })
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
        ttl_ms: None,
        cancel_requested: false,
    }
}

fn is_auto_defer_candidate(name: &str) -> bool {
    matches!(
        name,
        "build_or_update_graph"
            | "postprocess_graph"
            | "doctor"
            | "analyze_architecture"
            | "analyze_metrics"
            | "assess_risk"
            | "analyze_patterns"
            | "find_large_functions"
            | "find_complex_functions"
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
            ttl_ms: Some(5000),
            cancel_requested: false,
        };
        assert_eq!(task_wire_json(&task)["taskId"], json!("task-1"));
        assert_eq!(task_wire_json(&task)["status"], json!("working"));
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
        let result = tasks_result(
            Some(&json!({"taskId": &task_id})),
            dir.path().to_str().unwrap(),
            OutputFormat::Json,
        )
        .unwrap();
        assert_eq!(result["slept_ms"], json!(25));

        let reopened = SessionStore::open_in_repo(dir.path())
            .unwrap()
            .get_durable_task(&task_id)
            .unwrap()
            .unwrap();
        assert_eq!(reopened.status, DurableTaskStatus::Completed);
    }

    #[test]
    fn explicit_task_cancel_marks_task_cancelled() {
        let dir = TempDir::new().unwrap();
        install_tool_call_request_params(Some(&json!({"task": {"ttl": 1000}})));
        let created = execute_tool_call(
            "__test_sleep",
            Some(json!({"sleep_ms": 200})),
            dir.path().to_str().unwrap(),
            "db",
        )
        .unwrap();
        uninstall_tool_call_request_params();

        let task_id = created["task"]["taskId"].as_str().unwrap().to_owned();
        let cancelled = tasks_cancel(
            Some(&json!({"taskId": &task_id})),
            dir.path().to_str().unwrap(),
            OutputFormat::Json,
        )
        .unwrap();
        assert_eq!(cancelled["status"], json!("cancelled"));
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
    fn tasks_result_working_task_uses_not_ready_error_kind() {
        let dir = TempDir::new().unwrap();
        create_task_with_status(&dir, "task-working", DurableTaskStatus::Working, None);
        let error = tasks_result(
            Some(&json!({"taskId": "task-working"})),
            dir.path().to_str().unwrap(),
            OutputFormat::Json,
        )
        .unwrap_err();
        assert_eq!(error.kind(), TaskApiErrorKind::NotReady);
        assert!(error.message().contains("still working"));
    }

    #[test]
    fn tasks_result_cancelled_task_uses_cancelled_error_kind() {
        let dir = TempDir::new().unwrap();
        create_task_with_status(&dir, "task-cancelled", DurableTaskStatus::Cancelled, None);
        let error = tasks_result(
            Some(&json!({"taskId": "task-cancelled"})),
            dir.path().to_str().unwrap(),
            OutputFormat::Json,
        )
        .unwrap_err();
        assert_eq!(error.kind(), TaskApiErrorKind::Cancelled);
        assert!(error.message().contains("was cancelled"));
    }

    #[test]
    fn tasks_result_failed_task_uses_failed_error_kind() {
        let dir = TempDir::new().unwrap();
        create_task_with_status(&dir, "task-failed", DurableTaskStatus::Failed, Some("boom"));
        let error = tasks_result(
            Some(&json!({"taskId": "task-failed"})),
            dir.path().to_str().unwrap(),
            OutputFormat::Json,
        )
        .unwrap_err();
        assert_eq!(error.kind(), TaskApiErrorKind::Failed);
        assert_eq!(error.message(), "boom");
    }

    #[test]
    fn implicit_auto_defer_short_run_skips_task_persistence() {
        let dir = TempDir::new().unwrap();
        install_test_auto_defer_tool("__test_sleep");
        install_test_defer_threshold_ms(100);
        let result = execute_tool_call(
            "__test_sleep",
            Some(json!({"sleep_ms": 5})),
            dir.path().to_str().unwrap(),
            "db",
        )
        .unwrap();
        uninstall_test_defer_threshold_ms();
        uninstall_test_auto_defer_tool("__test_sleep");

        assert_eq!(result["slept_ms"], json!(5));
        let page = SessionStore::open_in_repo(dir.path())
            .unwrap()
            .list_durable_tasks(None, 10)
            .unwrap();
        assert!(page.tasks.is_empty());
    }

    #[test]
    fn implicit_auto_defer_returns_task_handle_after_threshold() {
        let dir = TempDir::new().unwrap();
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
        let result = tasks_result(
            Some(&json!({"taskId": &task_id})),
            dir.path().to_str().unwrap(),
            OutputFormat::Json,
        )
        .unwrap();
        assert_eq!(result["slept_ms"], json!(100));
    }
}
