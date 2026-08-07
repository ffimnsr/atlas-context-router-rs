#![allow(dead_code, deprecated)]

//! Official MCP SDK (rmcp) server implementation: protocol-surface plumbing
//! for the Atlas MCP tools.
//!
//! Module root: server struct + core result builders. `handler` implements the
//! `rmcp::ServerHandler` trait surface; `protocol` holds free helper fns.

mod handler;
mod protocol;

#[cfg(test)]
mod tests;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use rmcp::ErrorData as McpError;
use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestMethod, CallToolRequestParams, CallToolResponse, CancelTaskParams,
    ClientCapabilities, CompleteRequestParams, CompleteResult, ConstString, DiscoverResult,
    GetPromptResult, GetTaskParams, GetTaskResult, Implementation, ListPromptsResult,
    ListResourceTemplatesResult, ListResourcesResult, ListToolsResult, PaginatedRequestParams,
    PromptsCapability, ProtocolVersion, ReadResourceResult, ResourcesCapability, Root,
    ServerCapabilities, ServerInfo, SetLevelRequestParams, SubscriptionFilter, ToolsCapability,
    UpdateTaskParams,
};
use rmcp::service::{RequestContext, RoleServer};
use serde_json::{Value, json};

use crate::completion;
use crate::output::OutputFormat;
use crate::prompts;
use crate::resources;
use crate::runtime_context::RequestContext as AtlasRequestContext;
use crate::spec;
use crate::tools;
use crate::transport::ServerOptions;
use crate::transport::helpers::{
    ToolRepoResolutionContext, annotate_tool_result_with_repo_selection,
    resolve_repo_context_for_tool_call,
};
use crate::transport::repo_selection::{RepoSelectionSource, strip_repo_selector_fields};
use crate::transport::{ActiveRepoContext, RepoResolutionState, TraceLevel, TraceThreshold};

use protocol::{
    BootstrapNotificationPlan, cache_scope_from_str, canonical_repo_roots_from_roots,
    client_interaction_capabilities, legacy_completion_request_value, logging_level_from_rmcp,
    logging_message_notification_param, map_invalid_params_error, map_prompt_error,
    map_task_api_error, request_id_string, tool_call_log_level, tool_response_is_error,
    trace_enabled, trace_level_from_value,
};

#[derive(Clone, Debug)]
pub(crate) struct AtlasRmcpServer {
    repo_root: String,
    db_path: String,
    options: ServerOptions,
    state: Arc<AtlasRmcpRuntimeState>,
}

#[derive(Debug)]
struct AtlasRmcpRuntimeState {
    initialized: AtomicBool,
    requested_log_level: Mutex<Option<crate::logging::LogLevel>>,
    trace_level: Mutex<TraceLevel>,
    repo_resolution: Mutex<RepoResolutionState>,
}

#[derive(Clone, Debug)]
struct AtlasRmcpProgressContext {
    tx: tokio::sync::mpsc::UnboundedSender<(String, Option<u32>)>,
    cancel_flag: Arc<AtomicBool>,
}

#[derive(Clone, Debug, Default)]
struct AtlasRmcpCallContext {
    request_id: String,
    client_capabilities: Option<Value>,
    authenticated_principal: Option<String>,
    progress: Option<AtlasRmcpProgressContext>,
}

#[derive(Clone, Debug)]
pub(crate) struct AuthenticatedPrincipal(pub(crate) String);

impl AtlasRmcpRuntimeState {
    fn new(repo_root: &str, db_path: &str, dynamic_roots: bool) -> Self {
        let startup = if repo_root.is_empty() || db_path.is_empty() {
            None
        } else {
            Some(ActiveRepoContext {
                repo_root: repo_root.to_owned(),
                db_path: db_path.to_owned(),
            })
        };
        Self {
            initialized: AtomicBool::new(false),
            requested_log_level: Mutex::new(None),
            trace_level: Mutex::new(TraceLevel::Off),
            repo_resolution: Mutex::new(RepoResolutionState {
                startup: startup.clone(),
                active: startup,
                active_selection_source: None,
                candidate_roots: None,
                dynamic_roots,
            }),
        }
    }
}

impl AtlasRmcpServer {
    pub(crate) fn new(
        repo_root: impl Into<String>,
        db_path: impl Into<String>,
        options: ServerOptions,
    ) -> Self {
        let repo_root = repo_root.into();
        let db_path = db_path.into();
        Self {
            state: Arc::new(AtlasRmcpRuntimeState::new(&repo_root, &db_path, false)),
            repo_root,
            db_path,
            options,
        }
    }

    #[cfg(test)]
    fn new_with_dynamic_roots(
        repo_root: Option<&str>,
        db_path: Option<&str>,
        options: ServerOptions,
    ) -> Self {
        let repo_root = repo_root.unwrap_or_default().to_owned();
        let db_path = db_path.unwrap_or_default().to_owned();
        Self {
            state: Arc::new(AtlasRmcpRuntimeState::new(&repo_root, &db_path, true)),
            repo_root,
            db_path,
            options,
        }
    }

    pub(crate) fn repo_root(&self) -> &str {
        &self.repo_root
    }

    pub(crate) fn db_path(&self) -> &str {
        &self.db_path
    }

    pub(crate) fn options(&self) -> &ServerOptions {
        &self.options
    }

    pub(crate) fn is_initialized(&self) -> bool {
        self.state.initialized.load(Ordering::Relaxed)
    }

    pub(crate) fn requested_log_level(&self) -> Option<crate::logging::LogLevel> {
        *self
            .state
            .requested_log_level
            .lock()
            .expect("rmcp requested_log_level lock poisoned")
    }

    pub(crate) fn trace_level(&self) -> TraceLevel {
        *self
            .state
            .trace_level
            .lock()
            .expect("rmcp trace_level lock poisoned")
    }

    pub(crate) fn repo_resolution(&self) -> RepoResolutionState {
        self.state
            .repo_resolution
            .lock()
            .expect("rmcp repo_resolution lock poisoned")
            .clone()
    }

    #[cfg(test)]
    fn set_candidate_roots_for_tests(&self, roots: Option<Vec<String>>) {
        self.state
            .repo_resolution
            .lock()
            .expect("rmcp repo_resolution lock poisoned")
            .candidate_roots = roots;
    }

    pub(crate) fn implementation(&self) -> Implementation {
        let info = spec::server_info();
        Implementation::new(info.name, info.version).with_description(info.description)
    }

    pub(crate) fn server_capabilities(&self) -> ServerCapabilities {
        let mut tools = ToolsCapability::default();
        tools.list_changed = Some(true);

        let mut prompts = PromptsCapability::default();
        prompts.list_changed = Some(true);

        let mut resources = ResourcesCapability::default();
        resources.subscribe = Some(true);
        resources.list_changed = Some(true);

        ServerCapabilities::builder()
            .enable_logging()
            .enable_tools_with(tools)
            .enable_prompts_with(prompts)
            .enable_resources_with(resources)
            .enable_completions()
            .enable_tasks()
            .build()
    }

    pub(crate) fn info(&self) -> ServerInfo {
        ServerInfo::new(self.server_capabilities())
            .with_protocol_version(ProtocolVersion::V_2026_07_28)
            .with_server_info(self.implementation())
            .with_instructions(spec::DISCOVER_INSTRUCTIONS)
    }

    pub(crate) fn discover_result(&self) -> DiscoverResult {
        DiscoverResult::from_server_info(
            self.supported_protocol_versions().into_owned(),
            self.info(),
        )
        .with_ttl_ms(spec::DISCOVER_CACHE_TTL_MS)
        .with_cache_scope(cache_scope_from_str(spec::DISCOVER_CACHE_SCOPE))
    }

    pub(crate) fn list_tools_result(&self) -> Result<ListToolsResult> {
        let tools = tools::tool_descriptors();
        Ok(ListToolsResult::with_all_items(tools)
            .with_ttl_ms(crate::rmcp_types::ATLAS_PUBLIC_LIST_TTL_MS)
            .with_cache_scope(crate::rmcp_types::public_cache_scope()))
    }

    pub(crate) fn list_prompts_result(&self) -> Result<ListPromptsResult> {
        Ok(
            ListPromptsResult::with_all_items(prompts::prompt_descriptors())
                .with_ttl_ms(crate::rmcp_types::ATLAS_PUBLIC_LIST_TTL_MS)
                .with_cache_scope(crate::rmcp_types::public_cache_scope()),
        )
    }

    pub(crate) fn list_resources_result(
        &self,
        request: Option<PaginatedRequestParams>,
    ) -> Result<ListResourcesResult> {
        let args = request.and_then(|request| serde_json::to_value(request).ok());
        let result = resources::resources_list(args.as_ref())?;
        crate::rmcp_types::list_resources_result_from_atlas_value(result)
    }

    pub(crate) fn list_resource_templates_result(
        &self,
        request: Option<PaginatedRequestParams>,
    ) -> Result<ListResourceTemplatesResult> {
        let args = request.and_then(|request| serde_json::to_value(request).ok());
        let result = resources::resources_templates_list(args.as_ref())?;
        crate::rmcp_types::list_resource_templates_result_from_atlas_value(result)
    }

    pub(crate) fn get_prompt_result(
        &self,
        name: &str,
        arguments: Option<&rmcp::model::JsonObject>,
    ) -> Result<GetPromptResult, McpError> {
        let args = arguments.cloned().map(Value::Object);
        prompts::prompt_get(name, args.as_ref())
            .and_then(crate::rmcp_types::get_prompt_result_from_atlas_value)
            .map_err(map_prompt_error)
    }

    pub(crate) fn read_resource_result(&self, uri: &str) -> Result<ReadResourceResult, McpError> {
        resources::resources_read(
            Some(&json!({ "uri": uri })),
            self.repo_root(),
            self.db_path(),
        )
        .and_then(crate::rmcp_types::read_resource_result_from_atlas_value)
        .map_err(map_invalid_params_error)
    }

    pub(crate) fn complete_result(
        &self,
        request: CompleteRequestParams,
    ) -> Result<CompleteResult, McpError> {
        let legacy_args = legacy_completion_request_value(&request)
            .map_err(|error| crate::rmcp_error::invalid_params(error.to_string(), None))?;
        completion::complete(Some(&legacy_args), self.repo_root(), self.db_path())
            .and_then(crate::rmcp_types::complete_result_from_atlas_value)
            .map_err(map_invalid_params_error)
    }

    pub(crate) fn accepted_subscription_filter_result(
        &self,
        requested: &SubscriptionFilter,
    ) -> SubscriptionFilter {
        requested.supported_by(&self.server_capabilities())
    }

    fn bootstrap_notification_plan(
        &self,
        accepted: &SubscriptionFilter,
    ) -> BootstrapNotificationPlan {
        BootstrapNotificationPlan {
            send_tool_list_changed: accepted.tools_list_changed == Some(true),
            send_prompt_list_changed: accepted.prompts_list_changed == Some(true),
            send_resource_list_changed: accepted.resources_list_changed == Some(true),
            resource_updates: accepted.resource_subscriptions.clone().unwrap_or_default(),
        }
    }

    fn ping_result(&self) -> Result<(), McpError> {
        Ok(())
    }

    fn set_level_result(&self, request: SetLevelRequestParams) {
        let mapped = logging_level_from_rmcp(request.level);
        *self
            .state
            .requested_log_level
            .lock()
            .expect("rmcp requested_log_level lock poisoned") = Some(mapped);
    }

    fn set_trace_level_result(&self, params: Option<&Value>) -> Result<(), McpError> {
        let level = trace_level_from_value(params)
            .map_err(|error| crate::rmcp_error::invalid_params(error.to_string(), None))?;
        *self
            .state
            .trace_level
            .lock()
            .expect("rmcp trace_level lock poisoned") = level;
        Ok(())
    }

    fn mark_initialized(&self) {
        self.state.initialized.store(true, Ordering::Relaxed);
    }

    fn invalidate_dynamic_roots(&self) {
        let mut repo_resolution = self
            .state
            .repo_resolution
            .lock()
            .expect("rmcp repo_resolution lock poisoned");
        if !repo_resolution.dynamic_roots {
            return;
        }
        repo_resolution.active = repo_resolution.startup.clone();
        repo_resolution.active_selection_source = None;
        repo_resolution.candidate_roots = None;
    }

    fn update_repo_resolution(
        &self,
        repo_context: ActiveRepoContext,
        selection_source: RepoSelectionSource,
    ) {
        let mut repo_resolution = self
            .state
            .repo_resolution
            .lock()
            .expect("rmcp repo_resolution lock poisoned");
        repo_resolution.active = Some(repo_context);
        repo_resolution.active_selection_source = Some(selection_source);
    }

    fn should_refresh_dynamic_roots(&self) -> bool {
        let repo_resolution = self.repo_resolution();
        repo_resolution.dynamic_roots
            && repo_resolution.active.is_none()
            && repo_resolution.startup.is_none()
            && repo_resolution.candidate_roots.is_none()
    }

    async fn try_refresh_dynamic_roots(
        &self,
        context: &RequestContext<RoleServer>,
    ) -> anyhow::Result<()> {
        if !self.should_refresh_dynamic_roots() {
            return Ok(());
        }
        let supports_roots = context
            .client_capabilities()
            .and_then(|capabilities| capabilities.roots)
            .is_some();
        if !supports_roots {
            return Ok(());
        }
        let roots = context
            .peer
            .list_roots()
            .await
            .map_err(|error| anyhow::anyhow!("cannot request client roots: {error}"))?;
        let _ = self.load_client_roots_result(&roots.roots)?;
        Ok(())
    }

    async fn emit_tool_completion_log(
        &self,
        context: &RequestContext<RoleServer>,
        request: &CallToolRequestParams,
        response: &CallToolResponse,
        elapsed: Duration,
    ) {
        let level = tool_call_log_level(response);
        let threshold = self.requested_log_level();
        if !crate::logging::should_emit(threshold, level) {
            return;
        }
        let request_id = request_id_string(context);
        let message = format!(
            "request_id={request_id} method=tools/call tool={} success={} total_ms={}",
            request.name,
            !tool_response_is_error(response),
            elapsed.as_millis()
        );
        self.emit_logging_message(context, level, message).await;
    }

    async fn emit_trace_lifecycle_log(
        &self,
        context: &RequestContext<RoleServer>,
        threshold: TraceThreshold,
        level: crate::logging::LogLevel,
        message: String,
    ) {
        if !trace_enabled(self.trace_level(), threshold) {
            return;
        }
        self.emit_logging_message(context, level, message).await;
    }

    async fn emit_logging_message(
        &self,
        context: &RequestContext<RoleServer>,
        level: crate::logging::LogLevel,
        message: String,
    ) {
        let notification = logging_message_notification_param(level, &message);
        if context
            .peer
            .notify_logging_message(notification)
            .await
            .is_err()
        {
            crate::logging::write_stdio_log(level, &message);
        }
    }

    fn load_client_roots_result(&self, roots: &[Root]) -> anyhow::Result<Vec<String>> {
        let canonical = canonical_repo_roots_from_roots(roots)?;
        let mut repo_resolution = self
            .state
            .repo_resolution
            .lock()
            .expect("rmcp repo_resolution lock poisoned");
        repo_resolution.candidate_roots = Some(canonical.clone());
        match canonical.as_slice() {
            [only_root] => {
                repo_resolution.active = Some(ActiveRepoContext {
                    repo_root: only_root.clone(),
                    db_path: atlas_engine::paths::default_db_path(only_root),
                });
                repo_resolution.active_selection_source =
                    Some(RepoSelectionSource::CachedActiveRoot);
            }
            _ => {
                repo_resolution.active = repo_resolution.startup.clone();
                repo_resolution.active_selection_source = None;
            }
        }
        Ok(canonical)
    }

    pub(crate) fn call_tool_for_tests(
        &self,
        request: CallToolRequestParams,
    ) -> Result<CallToolResponse, McpError> {
        self.call_tool_impl(request, AtlasRmcpCallContext::default())
    }

    #[cfg(test)]
    fn call_tool_for_tests_with_context(
        &self,
        request: CallToolRequestParams,
        context: AtlasRmcpCallContext,
    ) -> Result<CallToolResponse, McpError> {
        self.call_tool_impl(request, context)
    }

    fn get_task_result(&self, request: GetTaskParams) -> Result<GetTaskResult, McpError> {
        let task = crate::tasks::task_record(self.repo_root(), &request.task_id)
            .map_err(map_task_api_error)?;
        crate::rmcp_types::get_task_result_from_record(&task)
            .map_err(crate::rmcp_error::internal_error)
    }

    fn update_task_result(&self, request: UpdateTaskParams) -> Result<(), McpError> {
        let params = serde_json::to_value(&request)
            .map_err(|error| crate::rmcp_error::invalid_params(error.to_string(), None))?;
        crate::tasks::tasks_update(Some(&params), self.repo_root(), OutputFormat::Json)
            .map_err(map_task_api_error)?;
        Ok(())
    }

    fn cancel_task_result(&self, request: CancelTaskParams) -> Result<(), McpError> {
        crate::tasks::tasks_cancel(&request.task_id, self.repo_root())
            .map_err(map_task_api_error)?;
        Ok(())
    }

    fn call_tool_impl(
        &self,
        request: CallToolRequestParams,
        context: AtlasRmcpCallContext,
    ) -> Result<CallToolResponse, McpError> {
        let tool_name = request.name.to_string();
        if !crate::tools::is_known_tool_name(&tool_name) {
            return Err(crate::rmcp_error::method_not_found(format!(
                "unknown tool: {tool_name}"
            )));
        }

        let request_params = serde_json::to_value(&request)
            .map_err(|error| crate::rmcp_error::invalid_params(error.to_string(), None))?;
        let client_capabilities =
            client_interaction_capabilities(context.client_capabilities.as_ref());
        if crate::tasks::task_ttl_from_request_params(&request_params).is_some()
            && !client_capabilities.supports_tasks
        {
            return Err(McpError::missing_required_client_capability(
                ClientCapabilities::builder().enable_tasks().build(),
            ));
        }
        let raw_args = request.arguments.clone().map(Value::Object);
        let args = raw_args.clone().map(strip_repo_selector_fields);
        let selection = resolve_repo_context_for_tool_call(
            &self.repo_resolution(),
            Some(&tool_name),
            raw_args.as_ref(),
            ToolRepoResolutionContext,
        )
        .map_err(|error| {
            crate::rmcp_error::invalid_params(error.message(), Some(error.error_data()))
        })?;
        let repo_context = selection.repo_context.clone();

        let runtime_context = AtlasRequestContext::new(
            Arc::new(|_| Ok(())),
            client_capabilities,
            "rmcp",
            None,
            context.authenticated_principal,
            context.request_id,
            CallToolRequestMethod::VALUE,
            Some(request_params.clone()),
        );

        if let Some(progress) = context.progress.as_ref() {
            let tx = progress.tx.clone();
            crate::progress::install(
                move |message, percentage| {
                    let _ = tx.send((message.to_owned(), percentage));
                },
                Arc::clone(&progress.cancel_flag),
            );
        }
        crate::runtime_context::install(runtime_context);
        crate::tasks::install_tool_call_request_params(Some(&request_params));
        let result = crate::tasks::execute_tool_call(
            &tool_name,
            args,
            &repo_context.repo_root,
            &repo_context.db_path,
        );
        crate::tasks::uninstall_tool_call_request_params();
        crate::runtime_context::uninstall();
        crate::progress::uninstall();

        let mut atlas_value = result.map_err(crate::rmcp_error::internal_error)?;
        annotate_tool_result_with_repo_selection(
            &mut atlas_value,
            &repo_context.repo_root,
            selection.selection_source,
            self.repo_resolution().dynamic_roots,
        );
        self.update_repo_resolution(repo_context, selection.selection_source);
        crate::rmcp_types::call_tool_response_from_atlas_value(atlas_value)
            .map_err(crate::rmcp_error::internal_error)
    }
}
