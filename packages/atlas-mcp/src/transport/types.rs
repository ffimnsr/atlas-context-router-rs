//! Shared types for the transport layer.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::{Condvar, Mutex};
use std::time::Instant;

use super::broker::ReverseRequestBroker;
use super::repo_selection::RepoSelectionSource;
use crate::logging;
use crate::output::OutputFormat;

// ---------------------------------------------------------------------------
// Reverse-request machinery
// ---------------------------------------------------------------------------

pub(crate) type ReverseResponseWaiter = Arc<(
    Mutex<Option<std::result::Result<serde_json::Value, String>>>,
    Condvar,
)>;

pub(crate) struct PendingReverseRequest {
    pub(crate) scope_id: String,
    pub(crate) waiter: ReverseResponseWaiter,
}

// ---------------------------------------------------------------------------
// ServerOptions
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct ServerOptions {
    pub worker_threads: usize,
    pub tool_timeout_ms: u64,
    pub tool_timeout_ms_by_tool: HashMap<String, u64>,
    #[cfg(feature = "http-transport")]
    pub http_auth: Option<crate::auth::ProtectedResourceAuthConfig>,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            worker_threads: 2,
            tool_timeout_ms: 300_000,
            tool_timeout_ms_by_tool: HashMap::new(),
            #[cfg(feature = "http-transport")]
            http_auth: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Transport event types
// ---------------------------------------------------------------------------

pub(crate) struct PendingRequest {
    pub(crate) id: serde_json::Value,
    pub(crate) request: RequestLogContext,
    pub(crate) queued_at: Instant,
    pub(crate) deadline: Instant,
    pub(crate) timeout_ms: u128,
    pub(crate) output_format: OutputFormat,
    pub(crate) progress_token: Option<serde_json::Value>,
    pub(crate) cancel_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

pub(crate) struct RequestDispatchContext<'a> {
    pub(crate) worker_pool: &'a super::worker::WorkerPool,
    pub(crate) server_options: &'a ServerOptions,
    pub(crate) canceled_tokens: Arc<Mutex<HashSet<u64>>>,
    pub(crate) event_tx: &'a std::sync::mpsc::Sender<TransportEvent>,
}

pub(crate) struct StdioReverseEmitter {
    pub(crate) event_tx: std::sync::mpsc::Sender<TransportEvent>,
}

impl super::broker::ReverseRequestEmitter for StdioReverseEmitter {
    fn emit_request(&self, request: serde_json::Value) -> anyhow::Result<()> {
        self.event_tx
            .send(TransportEvent::OutboundJson(request.to_string()))
            .map_err(|_| anyhow::anyhow!("stdio reverse-request channel disconnected"))?;
        Ok(())
    }

    fn emit_task_status(&self, params: serde_json::Value) -> anyhow::Result<()> {
        self.event_tx
            .send(TransportEvent::OutboundJson(
                crate::transport::jsonrpc::jsonrpc_notification(
                    "notifications/tasks/status",
                    params,
                ),
            ))
            .map_err(|_| anyhow::anyhow!("stdio task notification channel disconnected"))?;
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct TransportStats {
    pub(crate) received: u64,
    pub(crate) notifications: u64,
    pub(crate) parse_errors: u64,
    pub(crate) async_dispatched: u64,
    pub(crate) completed: u64,
    pub(crate) completed_ok: u64,
    pub(crate) completed_err: u64,
    pub(crate) protocol_errors: u64,
    pub(crate) tool_execution_errors: u64,
    pub(crate) timed_out: u64,
    pub(crate) canceled: u64,
    pub(crate) dropped_late: u64,
}

pub(crate) enum TransportEvent {
    InputLine(String),
    InputClosed,
    InputError(String),
    WorkerStarted {
        token: u64,
        queue_wait_ms: u128,
    },
    Response {
        token: u64,
        response: String,
        completion: RequestCompletion,
    },
    RepoContextResolved {
        repo_context: ActiveRepoContext,
        selection_source: RepoSelectionSource,
        candidate_roots: Option<Vec<String>>,
    },
    OutboundJson(String),
    ProgressReport {
        token: u64,
        message: String,
        percentage: Option<u32>,
    },
}

#[derive(Clone)]
pub(crate) struct RequestLogContext {
    pub(crate) request_id: String,
    pub(crate) method: String,
    pub(crate) tool_name: Option<String>,
}

pub(crate) struct RequestCompletion {
    pub(crate) request: RequestLogContext,
    pub(crate) queue_wait_ms: u128,
    pub(crate) execution_ms: u128,
    pub(crate) success: bool,
}

// ---------------------------------------------------------------------------
// Connection state
// ---------------------------------------------------------------------------

pub(crate) struct ConnectionState {
    pub(crate) trace: TraceLevel,
    pub(crate) initialized: bool,
    pub(crate) log_level: Option<logging::LogLevel>,
    pub(crate) client_capabilities: serde_json::Value,
    pub(crate) canceled_tokens: Arc<Mutex<HashSet<u64>>>,
    pub(crate) reverse_broker: ReverseRequestBroker,
    pub(crate) repo_resolution: RepoResolutionState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ActiveRepoContext {
    pub(crate) repo_root: String,
    pub(crate) db_path: String,
}

#[derive(Clone, Debug)]
pub(crate) struct RepoResolutionState {
    pub(crate) startup: Option<ActiveRepoContext>,
    pub(crate) active: Option<ActiveRepoContext>,
    pub(crate) active_selection_source: Option<RepoSelectionSource>,
    pub(crate) candidate_roots: Option<Vec<String>>,
    pub(crate) preferred_root_hint_uri: Option<String>,
    pub(crate) launch_cwd_fallback: Option<ActiveRepoContext>,
    pub(crate) dynamic_roots: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TraceLevel {
    Off,
    Messages,
    Verbose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TraceThreshold {
    Messages,
    Verbose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProgressEventKind {
    Begin,
    Report,
    End,
}

// ---------------------------------------------------------------------------
// Factory helpers
// ---------------------------------------------------------------------------

pub(crate) fn connection_state(
    repo_root: Option<&str>,
    db_path: Option<&str>,
    dynamic_roots: bool,
    launch_cwd_repo_root: Option<&str>,
) -> ConnectionState {
    let startup = match (repo_root, db_path) {
        (Some(repo_root), Some(db_path)) if !repo_root.is_empty() && !db_path.is_empty() => {
            Some(ActiveRepoContext {
                repo_root: repo_root.to_owned(),
                db_path: db_path.to_owned(),
            })
        }
        _ => None,
    };
    let launch_cwd_fallback = launch_cwd_repo_root.map(|repo_root| ActiveRepoContext {
        repo_root: repo_root.to_owned(),
        db_path: atlas_engine::paths::default_db_path(repo_root),
    });
    ConnectionState {
        trace: TraceLevel::Off,
        initialized: false,
        log_level: None,
        client_capabilities: serde_json::Value::Null,
        canceled_tokens: Arc::new(Mutex::new(HashSet::new())),
        reverse_broker: ReverseRequestBroker::new(),
        repo_resolution: RepoResolutionState {
            startup: startup.clone(),
            active: startup,
            active_selection_source: None,
            candidate_roots: None,
            preferred_root_hint_uri: None,
            launch_cwd_fallback,
            dynamic_roots,
        },
    }
}

pub(crate) fn request_id_string(id: &serde_json::Value) -> String {
    match id {
        serde_json::Value::String(value) => value.clone(),
        _ => id.to_string(),
    }
}
