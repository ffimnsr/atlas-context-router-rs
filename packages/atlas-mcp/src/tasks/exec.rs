use super::*;

pub(crate) fn execute_tool_call_with_worker_threads(
    name: &str,
    args: Option<Value>,
    repo_root: &str,
    db_path: &str,
    worker_threads: usize,
) -> Result<Value> {
    let worker_threads = worker_threads.max(1);
    let defer_threshold = Duration::from_millis(resolve_defer_threshold_ms());
    let request_context = runtime_context::current().ok();
    let explicit_task = task_ttl_from_context_args();
    let supports_tasks = request_context
        .as_ref()
        .is_some_and(|context| context.capabilities.supports_tasks);
    let auto_defer = supports_tasks && explicit_task.is_none() && is_auto_defer_candidate(name);
    if explicit_task.is_none() && !auto_defer {
        return normalize_tool_call_result(
            name,
            args.as_ref(),
            crate::tools::call_with_worker_threads(
                name,
                args.as_ref(),
                repo_root,
                db_path,
                worker_threads,
            ),
        );
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
    let execution_target = TaskExecutionTarget {
        repo_root: repo_root.to_owned(),
        db_path: db_path.to_owned(),
        worker_threads,
    };
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
                &execution_target,
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
    target: &TaskExecutionTarget,
    cancel_flag: Arc<std::sync::atomic::AtomicBool>,
    request_context: Option<RequestContext>,
) -> Result<Value> {
    if let Some(client) = request_context.clone() {
        runtime_context::install(client);
    }
    let repo_root_for_progress = target.repo_root.clone();
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
                let _ = client.notify_task_status(task_status_notification_json(&task));
            }
        },
        cancel_flag,
    );
    let result = normalize_tool_call_result(
        tool_name,
        args.as_ref(),
        crate::tools::call_with_worker_threads(
            tool_name,
            args.as_ref(),
            &target.repo_root,
            &target.db_path,
            target.worker_threads,
        ),
    );
    progress::uninstall();
    runtime_context::uninstall();

    let current = open_task_record(&target.repo_root, task_id)?;
    let already_cancelled = current
        .as_ref()
        .is_some_and(|task| task.status == DurableTaskStatus::Cancelled);
    match result {
        Ok(value)
            if current.is_some() && !already_cancelled && is_input_required_tool_result(&value) =>
        {
            let update = DurableTaskUpdate {
                status: Some(DurableTaskStatus::InputRequired),
                status_message: Some("input required".to_owned()),
                input_requests: crate::rmcp_types::persisted_input_requests_from_atlas_tool_result(
                    &value,
                )?,
                request_state: value
                    .get("requestState")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                ..Default::default()
            };
            update_task_store(&target.repo_root, task_id, update)?;
            if let Some(client) = request_context.as_ref()
                && let Some(task) = open_task_record(&target.repo_root, task_id)?
            {
                client.notify_task_status(task_status_notification_json(&task))?;
            }
            Ok(value)
        }
        Ok(value) if current.is_some() && !already_cancelled => {
            let update = DurableTaskUpdate {
                status: Some(DurableTaskStatus::Completed),
                status_message: Some("completed".to_owned()),
                result: Some(value.clone()),
                ..Default::default()
            };
            update_task_store(&target.repo_root, task_id, update)?;
            if let Some(client) = request_context.as_ref()
                && let Some(task) = open_task_record(&target.repo_root, task_id)?
            {
                client.notify_task_status(task_status_notification_json(&task))?;
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
            update_task_store(&target.repo_root, task_id, update)?;
            if let Some(client) = request_context.as_ref()
                && let Some(task) = open_task_record(&target.repo_root, task_id)?
            {
                client.notify_task_status(task_status_notification_json(&task))?;
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

pub(crate) fn task_ttl_from_request_params(params: &Value) -> Option<u64> {
    params
        .get("task")
        .or_else(|| {
            params
                .get("arguments")
                .and_then(|arguments| arguments.get("task"))
        })
        .and_then(|task| task.get("ttl").and_then(Value::as_u64))
}

fn task_ttl_from_context_args() -> Option<u64> {
    current_tool_call_request_params().and_then(|params| task_ttl_from_request_params(&params))
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

pub(crate) fn task_record(repo_root: &str, task_id: &str) -> TaskApiResult<DurableTaskRecord> {
    open_task_record(repo_root, task_id)
        .map_err(|error| TaskApiError::new(TaskApiErrorKind::Internal, error))?
        .ok_or_else(|| {
            task_api_error(
                TaskApiErrorKind::NotFound,
                format!("unknown task_id '{task_id}'"),
            )
        })
}

pub(crate) fn tasks_update(
    params: Option<&Value>,
    repo_root: &str,
    _output_format: OutputFormat,
) -> TaskApiResult<Value> {
    let task_id = required_task_id(params)?;
    let input = params
        .and_then(|value| {
            value
                .get("input")
                .or_else(|| value.get("payload"))
                .or_else(|| value.get("inputResponses"))
        })
        .cloned()
        .ok_or_else(|| {
            task_api_error(
                TaskApiErrorKind::InvalidParams,
                "missing required argument: input",
            )
        })?;
    let task = task_record(repo_root, task_id)?;
    match task.status {
        DurableTaskStatus::Completed | DurableTaskStatus::Failed | DurableTaskStatus::Cancelled => {
            return Err(task_api_error(
                TaskApiErrorKind::InvalidParams,
                format!("task '{task_id}' no longer accepts updates"),
            ));
        }
        DurableTaskStatus::InputRequired | DurableTaskStatus::Working => {}
    }
    update_task_store(
        repo_root,
        task_id,
        DurableTaskUpdate {
            status: Some(DurableTaskStatus::Working),
            status_message: Some("client input received".to_owned()),
            progress: Some(json!({"clientInput": input})),
            ..Default::default()
        },
    )
    .map_err(|error| TaskApiError::new(TaskApiErrorKind::Internal, error))?;
    let task = task_record(repo_root, task_id)?;
    if let Ok(client) = runtime_context::current() {
        let _ = client.notify_task_status(task_status_notification_json(&task));
    }
    Ok(json!({
        "task": task_handle_json(&task),
        "accepted": true,
        "updateKind": "client_input",
    }))
}

pub(crate) fn tasks_cancel(task_id: &str, repo_root: &str) -> TaskApiResult<DurableTaskRecord> {
    let task = task_record(repo_root, task_id)?;
    match task.status {
        DurableTaskStatus::Completed | DurableTaskStatus::Failed | DurableTaskStatus::Cancelled => {
            return Err(task_api_error(
                TaskApiErrorKind::InvalidParams,
                format!("task '{task_id}' no longer accepts cancellation"),
            ));
        }
        DurableTaskStatus::InputRequired | DurableTaskStatus::Working => {}
    }

    if let Some(handle) = live_tasks()
        .lock()
        .expect("live tasks lock poisoned")
        .get(task_id)
        .cloned()
    {
        handle.cancel_flag.store(true, Ordering::Relaxed);
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

    let task = task_record(repo_root, task_id)?;
    if let Ok(client) = runtime_context::current() {
        let _ = client.notify_task_status(task_status_notification_json(&task));
    }
    Ok(task)
}

pub(super) fn task_api_error(kind: TaskApiErrorKind, message: impl Into<String>) -> TaskApiError {
    TaskApiError::new(kind, anyhow!(message.into()))
}
