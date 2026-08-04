//! `rmcp::ServerHandler` implementation for `AtlasRmcpServer`.

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use anyhow::Result;
use rmcp::ErrorData as McpError;
use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, CancelTaskParams, CompleteRequestParams,
    CompleteResult, CustomRequest, CustomResult, DiscoverResult, GetPromptRequestParams,
    GetPromptResponse, GetTaskParams, GetTaskResult, ListPromptsResult,
    ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, PaginatedRequestParams,
    ProtocolVersion, ReadResourceRequestParams, ReadResourceResponse, ServerInfo,
    SetLevelRequestParams, SubscriptionFilter, UpdateTaskParams,
};
use rmcp::service::{NotificationContext, RequestContext, RoleServer, SubscriptionContext};
use serde_json::Value;

use crate::transport::TraceThreshold;

use super::protocol::{
    authenticated_principal, request_id_string, start_progress_forwarder,
    timeout_call_tool_response, tool_call_timeout, tool_response_is_error,
};
use super::{AtlasRmcpCallContext, AtlasRmcpServer};

impl ServerHandler for AtlasRmcpServer {
    async fn ping(&self, _context: RequestContext<RoleServer>) -> Result<(), McpError> {
        self.ping_result()
    }

    fn get_info(&self) -> ServerInfo {
        self.info()
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Owned(vec![
            ProtocolVersion::V_2026_07_28,
            ProtocolVersion::V_2025_11_25,
        ])
    }

    async fn discover(
        &self,
        _context: RequestContext<RoleServer>,
    ) -> Result<DiscoverResult, McpError> {
        Ok(self.discover_result())
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        self.list_tools_result()
            .map_err(crate::rmcp_error::internal_error)
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        self.list_prompts_result()
            .map_err(crate::rmcp_error::internal_error)
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        self.list_resources_result(_request)
            .map_err(crate::rmcp_error::internal_error)
    }

    async fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourceTemplatesResult, McpError> {
        self.list_resource_templates_result(_request)
            .map_err(crate::rmcp_error::internal_error)
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, McpError> {
        self.get_prompt_result(&request.name, request.arguments.as_ref())
            .map(Into::into)
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, McpError> {
        self.read_resource_result(&request.uri).map(Into::into)
    }

    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        Some(self.accepted_subscription_filter_result(requested))
    }

    async fn listen(&self, context: SubscriptionContext) -> Result<(), McpError> {
        let plan = self.bootstrap_notification_plan(context.accepted());
        if plan.send_tool_list_changed {
            context
                .sink()
                .notify_tool_list_changed()
                .await
                .map_err(|error| crate::rmcp_error::internal_error_message(error.to_string()))?;
        }
        if plan.send_prompt_list_changed {
            context
                .sink()
                .notify_prompt_list_changed()
                .await
                .map_err(|error| crate::rmcp_error::internal_error_message(error.to_string()))?;
        }
        if plan.send_resource_list_changed {
            context
                .sink()
                .notify_resource_list_changed()
                .await
                .map_err(|error| crate::rmcp_error::internal_error_message(error.to_string()))?;
        }
        for uri in plan.resource_updates {
            context
                .sink()
                .notify_resource_updated(uri)
                .await
                .map_err(|error| crate::rmcp_error::internal_error_message(error.to_string()))?;
        }
        context.cancelled().await;
        Ok(())
    }

    async fn complete(
        &self,
        request: CompleteRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CompleteResult, McpError> {
        self.complete_result(request)
    }

    async fn get_task(
        &self,
        request: GetTaskParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetTaskResult, McpError> {
        self.get_task_result(request)
    }

    async fn update_task(
        &self,
        request: UpdateTaskParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        self.update_task_result(request)
    }

    async fn cancel_task(
        &self,
        request: CancelTaskParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        self.cancel_task_result(request)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let _ = self.try_refresh_dynamic_roots(&context).await;
        let started = Instant::now();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let progress_task = start_progress_forwarder(&context, Arc::clone(&cancel_flag));
        let cancel_watch = {
            let ct = context.ct.clone();
            let cancel_flag = Arc::clone(&cancel_flag);
            tokio::spawn(async move {
                ct.cancelled().await;
                cancel_flag.store(true, Ordering::Relaxed);
            })
        };
        let call_context = AtlasRmcpCallContext {
            request_id: request_id_string(&context),
            client_capabilities: context
                .client_capabilities()
                .and_then(|caps| serde_json::to_value(caps).ok()),
            authenticated_principal: authenticated_principal(&context),
            progress: progress_task.as_ref().map(|(ctx, _)| ctx.clone()),
        };
        let timeout = tool_call_timeout(&self.options, request.name.as_ref());
        let request_id = request_id_string(&context);
        self.emit_trace_lifecycle_log(
            &context,
            TraceThreshold::Verbose,
            crate::logging::LogLevel::Debug,
            format!(
                "started request_id={request_id} method=tools/call tool={} timeout_ms={}",
                request.name,
                timeout.as_millis()
            ),
        )
        .await;
        let server = self.clone();
        let blocking_request = request.clone();
        let response = match tokio::time::timeout(
            timeout,
            tokio::task::spawn_blocking(move || {
                server.call_tool_impl(blocking_request, call_context)
            }),
        )
        .await
        {
            Ok(Ok(result)) => result?,
            Ok(Err(error)) => {
                return Err(crate::rmcp_error::internal_error_message(format!(
                    "rmcp tool worker join error: {error}"
                )));
            }
            Err(_) => {
                cancel_flag.store(true, Ordering::Relaxed);
                self.emit_trace_lifecycle_log(
                    &context,
                    TraceThreshold::Messages,
                    crate::logging::LogLevel::Error,
                    format!(
                        "timed out request_id={request_id} method=tools/call tool={} timeout_ms={}",
                        request.name,
                        timeout.as_millis()
                    ),
                )
                .await;
                timeout_call_tool_response(request.name.as_ref(), timeout)?
            }
        };
        if let Some((progress_context, handle)) = progress_task {
            drop(progress_context);
            let _ = handle.await;
        }
        cancel_watch.abort();
        self.emit_trace_lifecycle_log(
            &context,
            TraceThreshold::Messages,
            if tool_response_is_error(&response) {
                crate::logging::LogLevel::Error
            } else {
                crate::logging::LogLevel::Info
            },
            format!(
                "completed request_id={request_id} method=tools/call tool={} success={} total_ms={}",
                request.name,
                !tool_response_is_error(&response),
                started.elapsed().as_millis()
            ),
        )
        .await;
        self.emit_tool_completion_log(&context, &request, &response, started.elapsed())
            .await;
        Ok(response)
    }

    async fn set_level(
        &self,
        request: SetLevelRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        self.set_level_result(request);
        Ok(())
    }

    async fn on_custom_request(
        &self,
        request: CustomRequest,
        _context: RequestContext<RoleServer>,
    ) -> Result<CustomResult, McpError> {
        match request.method.as_str() {
            "$/setTrace" => {
                self.set_trace_level_result(request.params.as_ref())?;
                Ok(CustomResult::new(Value::Null))
            }
            _ => Err(crate::rmcp_error::method_not_found(request.method)),
        }
    }

    async fn on_initialized(&self, _context: NotificationContext<RoleServer>) {
        self.mark_initialized();
    }

    async fn on_roots_list_changed(&self, _context: NotificationContext<RoleServer>) {
        self.invalidate_dynamic_roots();
    }

    async fn on_custom_notification(
        &self,
        notification: rmcp::model::CustomNotification,
        _context: NotificationContext<RoleServer>,
    ) {
        if notification.method == "$/setTrace" {
            let _ = self.set_trace_level_result(notification.params.as_ref());
        }
    }
}
