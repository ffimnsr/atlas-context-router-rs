//! Shared transport types still used by rmcp wrappers.

use std::collections::HashMap;
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use std::sync::Condvar;
#[cfg(test)]
use std::sync::Mutex;

use super::repo_selection::RepoSelectionSource;

// ---------------------------------------------------------------------------
// Reverse-request machinery
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) type ReverseResponseWaiter = Arc<(
    Mutex<Option<std::result::Result<serde_json::Value, String>>>,
    Condvar,
)>;

#[cfg(test)]
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
// Repo resolution state
// ---------------------------------------------------------------------------

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
