use std::cell::RefCell;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use serde_json::Value;

pub(crate) type TaskStatusNotifyFn = dyn Fn(Value) -> Result<()> + Send + Sync;

#[derive(Clone, Debug, Default)]
pub(crate) struct ClientInteractionCapabilities {
    pub supports_elicitation_form: bool,
    #[allow(dead_code)]
    pub supports_elicitation_url: bool,
}

#[derive(Clone)]
pub(crate) struct RequestContext {
    task_notify_fn: Arc<TaskStatusNotifyFn>,
    pub capabilities: ClientInteractionCapabilities,
    pub transport_kind: String,
    pub session_id: Option<String>,
    pub authenticated_principal: Option<String>,
    pub originating_request_id: String,
    pub request_method: String,
    pub request_params: Option<Value>,
}

impl RequestContext {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        task_notify_fn: Arc<TaskStatusNotifyFn>,
        capabilities: ClientInteractionCapabilities,
        transport_kind: impl Into<String>,
        session_id: Option<String>,
        authenticated_principal: Option<String>,
        originating_request_id: impl Into<String>,
        request_method: impl Into<String>,
        request_params: Option<Value>,
    ) -> Self {
        Self {
            task_notify_fn,
            capabilities,
            transport_kind: transport_kind.into(),
            session_id,
            authenticated_principal,
            originating_request_id: originating_request_id.into(),
            request_method: request_method.into(),
            request_params,
        }
    }

    pub(crate) fn notify_task_status(&self, params: Value) -> Result<()> {
        (self.task_notify_fn)(params)
    }
}

thread_local! {
    static REQUEST_CONTEXT: RefCell<Option<RequestContext>> = const { RefCell::new(None) };
}

pub(crate) fn install(client: RequestContext) {
    REQUEST_CONTEXT.with(|slot| *slot.borrow_mut() = Some(client));
}

pub(crate) fn uninstall() {
    REQUEST_CONTEXT.with(|slot| *slot.borrow_mut() = None);
}

pub(crate) fn current() -> Result<RequestContext> {
    REQUEST_CONTEXT.with(|slot| {
        slot.borrow()
            .clone()
            .ok_or_else(|| anyhow!("no active MCP request context"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_round_trip() {
        let client = RequestContext::new(
            Arc::new(|_| Ok(())),
            ClientInteractionCapabilities {
                supports_elicitation_form: true,
                supports_elicitation_url: false,
            },
            "stdio",
            None,
            None,
            "1",
            "tools/call",
            Some(serde_json::json!({"name": "purge_saved_context"})),
        );
        install(client.clone());
        let active = current().unwrap();
        assert!(active.capabilities.supports_elicitation_form);
        assert_eq!(active.transport_kind, "stdio");
        assert_eq!(active.request_method, "tools/call");
        uninstall();
    }
}
