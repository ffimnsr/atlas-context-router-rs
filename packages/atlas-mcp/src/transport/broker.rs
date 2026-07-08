//! Reverse-request broker for sending client-directed requests
//! (e.g. roots/list, elicitation) and resolving the response.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;

use super::types::PendingReverseRequest;

/// Trait for emitting outbound JSON-RPC requests to the client.
pub(crate) trait ReverseRequestEmitter: Send + Sync {
    fn emit_request(&self, request: serde_json::Value) -> Result<()>;
    fn emit_task_status(&self, params: serde_json::Value) -> Result<()>;
}

/// Broker that tracks pending reverse requests and matches responses.
#[derive(Clone)]
pub(crate) struct ReverseRequestBroker {
    next_request_id: Arc<AtomicU64>,
    pending: Arc<Mutex<HashMap<String, PendingReverseRequest>>>,
}

impl ReverseRequestBroker {
    pub(crate) fn new() -> Self {
        Self {
            next_request_id: Arc::new(AtomicU64::new(0)),
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(crate) fn issue_request(
        &self,
        scope_id: &str,
        emitter: &Arc<dyn ReverseRequestEmitter>,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value> {
        let request_id = format!(
            "atlas-reverse-{scope_id}-{}",
            self.next_request_id.fetch_add(1, Ordering::Relaxed) + 1
        );
        let waiter = Arc::new((Mutex::new(None), Condvar::new()));
        self.pending
            .lock()
            .expect("reverse request broker lock poisoned")
            .insert(
                request_id.clone(),
                PendingReverseRequest {
                    scope_id: scope_id.to_owned(),
                    waiter: Arc::clone(&waiter),
                },
            );
        emitter.emit_request(serde_json::json!({
            "jsonrpc": "2.0",
            "id": request_id.clone(),
            "method": method,
            "params": params,
        }))?;
        let deadline = Instant::now() + timeout;
        let (lock, cv) = &*waiter;
        let mut response = lock.lock().expect("reverse request waiter lock poisoned");
        while response.is_none() {
            let now = Instant::now();
            if now >= deadline {
                self.pending
                    .lock()
                    .expect("reverse request broker lock poisoned")
                    .remove(&request_id);
                return Err(anyhow::anyhow!(
                    "reverse request '{method}' timed out after {} ms",
                    timeout.as_millis()
                ));
            }
            let wait = deadline.saturating_duration_since(now);
            let (guard, timeout_result) = cv
                .wait_timeout(response, wait)
                .expect("reverse request wait_timeout failed unexpectedly");
            response = guard;
            if timeout_result.timed_out() && response.is_none() {
                self.pending
                    .lock()
                    .expect("reverse request broker lock poisoned")
                    .remove(&request_id);
                return Err(anyhow::anyhow!(
                    "reverse request '{method}' timed out after {} ms",
                    timeout.as_millis()
                ));
            }
        }
        match response
            .take()
            .expect("reverse request response set before wake")
        {
            Ok(value) => Ok(value),
            Err(message) => Err(anyhow::anyhow!(message)),
        }
    }

    pub(crate) fn try_resolve_response(&self, response: &serde_json::Value) -> bool {
        self.try_resolve_response_for_scope(None, response)
    }

    pub(crate) fn try_resolve_response_for_scope(
        &self,
        required_scope_prefix: Option<&str>,
        response: &serde_json::Value,
    ) -> bool {
        let Some(id) = response.get("id") else {
            return false;
        };
        let key = request_id_string(id);
        let pending = {
            let mut pending = self
                .pending
                .lock()
                .expect("reverse request broker lock poisoned");
            let Some(found) = pending.get(&key) else {
                return false;
            };
            if let Some(prefix) = required_scope_prefix
                && !found.scope_id.starts_with(prefix)
            {
                return false;
            }
            pending.remove(&key)
        };
        let Some(pending) = pending else {
            return false;
        };
        let result = if let Some(error) = response.get("error") {
            Err(error
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("reverse request failed")
                .to_owned())
        } else {
            Ok(response
                .get("result")
                .cloned()
                .unwrap_or(serde_json::Value::Null))
        };
        let (lock, cv) = &*pending.waiter;
        *lock.lock().expect("reverse request waiter lock poisoned") = Some(result);
        cv.notify_all();
        true
    }

    /// Exposed for testing: returns true when no pending requests remain.
    #[cfg(test)]
    pub(crate) fn is_pending_empty(&self) -> bool {
        self.pending
            .lock()
            .expect("reverse request broker lock poisoned")
            .is_empty()
    }

    pub(crate) fn cancel_scope(&self, scope_id: &str, reason: &str) {
        let keys = self
            .pending
            .lock()
            .expect("reverse request broker lock poisoned")
            .iter()
            .filter_map(|(key, pending)| (pending.scope_id == scope_id).then_some(key.clone()))
            .collect::<Vec<_>>();
        for key in keys {
            if let Some(pending) = self
                .pending
                .lock()
                .expect("reverse request broker lock poisoned")
                .remove(&key)
            {
                let (lock, cv) = &*pending.waiter;
                *lock.lock().expect("reverse request waiter lock poisoned") =
                    Some(Err(reason.to_owned()));
                cv.notify_all();
            }
        }
    }
}

fn request_id_string(id: &serde_json::Value) -> String {
    match id {
        serde_json::Value::String(value) => value.clone(),
        _ => id.to_string(),
    }
}
