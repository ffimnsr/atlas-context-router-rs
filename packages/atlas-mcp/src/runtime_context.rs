use std::cell::RefCell;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use serde_json::Value;

pub(crate) type ReverseRequestFn = dyn Fn(&str, Value, Duration) -> Result<Value> + Send + Sync;
pub(crate) type TaskStatusNotifyFn = dyn Fn(Value) -> Result<()> + Send + Sync;

#[allow(dead_code)]
#[derive(Clone, Debug, Default)]
pub(crate) struct ClientInteractionCapabilities {
    pub supports_elicitation_form: bool,
    pub supports_elicitation_url: bool,
}

#[derive(Clone)]
pub(crate) struct ReverseRequestClient {
    request_fn: Arc<ReverseRequestFn>,
    task_notify_fn: Arc<TaskStatusNotifyFn>,
    pub capabilities: ClientInteractionCapabilities,
    pub transport_kind: String,
    pub session_id: Option<String>,
    pub originating_request_id: String,
}

impl ReverseRequestClient {
    pub(crate) fn new(
        request_fn: Arc<ReverseRequestFn>,
        task_notify_fn: Arc<TaskStatusNotifyFn>,
        capabilities: ClientInteractionCapabilities,
        transport_kind: impl Into<String>,
        session_id: Option<String>,
        originating_request_id: impl Into<String>,
    ) -> Self {
        Self {
            request_fn,
            task_notify_fn,
            capabilities,
            transport_kind: transport_kind.into(),
            session_id,
            originating_request_id: originating_request_id.into(),
        }
    }

    pub(crate) fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        (self.request_fn)(method, params, timeout)
    }

    pub(crate) fn notify_task_status(&self, params: Value) -> Result<()> {
        (self.task_notify_fn)(params)
    }
}

thread_local! {
    static REQUEST_CONTEXT: RefCell<Option<ReverseRequestClient>> = const { RefCell::new(None) };
}

pub(crate) fn install(client: ReverseRequestClient) {
    REQUEST_CONTEXT.with(|slot| *slot.borrow_mut() = Some(client));
}

pub(crate) fn uninstall() {
    REQUEST_CONTEXT.with(|slot| *slot.borrow_mut() = None);
}

pub(crate) fn current() -> Result<ReverseRequestClient> {
    REQUEST_CONTEXT.with(|slot| {
        slot.borrow()
            .clone()
            .ok_or_else(|| anyhow!("no active MCP request context for reverse request"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_round_trip() {
        let client = ReverseRequestClient::new(
            Arc::new(|_, _, _| Ok(serde_json::json!({"ok": true}))),
            Arc::new(|_| Ok(())),
            ClientInteractionCapabilities {
                supports_elicitation_form: true,
                supports_elicitation_url: false,
            },
            "stdio",
            None,
            "1",
        );
        install(client.clone());
        let active = current().unwrap();
        assert!(active.capabilities.supports_elicitation_form);
        assert_eq!(active.transport_kind, "stdio");
        uninstall();
    }
}
