#![allow(dead_code, deprecated)]

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Result;
use rmcp::ErrorData as McpError;
use rmcp::ServerHandler;
use rmcp::model::{
    ArgumentInfo, CacheScope, CallToolRequestMethod, CallToolRequestParams, CallToolResponse,
    CancelTaskParams, CompleteRequestParams, CompleteResult, ConstString, CustomRequest,
    CustomResult, DiscoverResult, GetPromptRequestParams, GetPromptResponse, GetPromptResult,
    GetTaskParams, GetTaskResult, Implementation, ListPromptsResult, ListResourceTemplatesResult,
    ListResourcesResult, ListToolsResult, LoggingLevel, LoggingMessageNotificationParam,
    PaginatedRequestParams, ProgressNotificationParam, PromptsCapability, ProtocolVersion,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Reference,
    ResourcesCapability, Root, ServerCapabilities, ServerInfo, SetLevelRequestParams,
    SubscriptionFilter, ToolsCapability, UpdateTaskParams,
};
use rmcp::service::{NotificationContext, RequestContext, RoleServer, SubscriptionContext};
use serde_json::Value;
use serde_json::{Map, json};

use crate::completion;
use crate::output::OutputFormat;
use crate::prompts;
use crate::resources;
use crate::runtime_context::{
    ClientInteractionCapabilities, RequestContext as AtlasRequestContext,
};
use crate::spec;
use crate::tool_result::{ToolErrorCode, ToolErrorPayload, tool_execution_error_value};
use crate::tools;
use crate::transport::ServerOptions;
use crate::transport::helpers::{
    ToolRepoResolutionContext, annotate_tool_result_with_repo_selection,
    resolve_repo_context_for_tool_call,
};
use crate::transport::repo_selection::{RepoSelectionSource, strip_repo_selector_fields};
use crate::transport::{ActiveRepoContext, RepoResolutionState, TraceLevel, TraceThreshold};

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
            client_interaction_capabilities(context.client_capabilities.as_ref()),
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

impl ServerHandler for AtlasRmcpServer {
    async fn ping(&self, _context: RequestContext<RoleServer>) -> Result<(), McpError> {
        self.ping_result()
    }

    fn get_info(&self) -> ServerInfo {
        self.info()
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Owned(vec![ProtocolVersion::V_2026_07_28])
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

fn start_progress_forwarder(
    context: &RequestContext<RoleServer>,
    cancel_flag: Arc<AtomicBool>,
) -> Option<(AtlasRmcpProgressContext, tokio::task::JoinHandle<()>)> {
    let progress_token = context.meta.get_progress_token()?;
    let peer = context.peer.clone();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(String, Option<u32>)>();
    let seq = Arc::new(AtomicU64::new(0));
    let seq_for_task = Arc::clone(&seq);
    let handle = tokio::spawn(async move {
        while let Some((message, percentage)) = rx.recv().await {
            let progress = percentage
                .map(|value| value as f64)
                .unwrap_or_else(|| seq_for_task.fetch_add(1, Ordering::Relaxed) as f64);
            let mut notification = ProgressNotificationParam::new(progress_token.clone(), progress)
                .with_message(message);
            if percentage.is_some() {
                notification = notification.with_total(100.0);
            }
            let _ = peer.notify_progress(notification).await;
        }
    });
    Some((AtlasRmcpProgressContext { tx, cancel_flag }, handle))
}

fn tool_call_timeout(options: &ServerOptions, tool_name: &str) -> Duration {
    let timeout_ms = options
        .tool_timeout_ms_by_tool
        .get(tool_name)
        .copied()
        .unwrap_or(options.tool_timeout_ms)
        .clamp(1_000, 3_600_000);
    Duration::from_millis(timeout_ms)
}

fn timeout_call_tool_response(
    tool_name: &str,
    timeout: Duration,
) -> Result<CallToolResponse, McpError> {
    let timeout_ms = timeout.as_millis();
    let payload = ToolErrorPayload::new(
        ToolErrorCode::Timeout,
        format!("Tool '{tool_name}' timed out after {timeout_ms} ms"),
    )
    .with_tool(tool_name)
    .with_retry_guidance(
        "Reduce request scope, increase timeout if configurable, or retry when dependencies are responsive.",
    )
    .with_details(json!({
        "detail": format!("rmcp stdio timed out after {timeout_ms} ms while handling tool call"),
        "timeout_ms": timeout_ms,
        "tool": tool_name,
    }));
    let atlas_value = tool_execution_error_value(OutputFormat::Json, &payload)
        .map_err(crate::rmcp_error::internal_error)?;
    crate::rmcp_types::call_tool_response_from_atlas_value(atlas_value)
        .map_err(crate::rmcp_error::internal_error)
}

fn tool_response_is_error(response: &CallToolResponse) -> bool {
    match response {
        CallToolResponse::Complete(result) => result.is_error == Some(true),
        CallToolResponse::InputRequired(_) | CallToolResponse::Task(_) => false,
        _ => false,
    }
}

fn tool_call_log_level(response: &CallToolResponse) -> crate::logging::LogLevel {
    if tool_response_is_error(response) {
        crate::logging::LogLevel::Error
    } else {
        crate::logging::LogLevel::Info
    }
}

fn trace_enabled(level: TraceLevel, threshold: TraceThreshold) -> bool {
    match (level, threshold) {
        (TraceLevel::Off, _) => false,
        (TraceLevel::Messages, TraceThreshold::Messages) => true,
        (TraceLevel::Messages, TraceThreshold::Verbose) => false,
        (TraceLevel::Verbose, _) => true,
    }
}

fn logging_message_notification_param(
    level: crate::logging::LogLevel,
    message: &str,
) -> LoggingMessageNotificationParam {
    LoggingMessageNotificationParam::new(
        logging_level_to_rmcp(level),
        Value::String(message.to_owned()),
    )
    .with_logger("atlas-mcp")
}

fn trace_level_from_value(params: Option<&Value>) -> anyhow::Result<TraceLevel> {
    let level = params
        .and_then(|value| value.get("value"))
        .and_then(Value::as_str)
        .unwrap_or("off");
    match level {
        "off" => Ok(TraceLevel::Off),
        "messages" => Ok(TraceLevel::Messages),
        "verbose" => Ok(TraceLevel::Verbose),
        other => Err(anyhow::anyhow!(
            "invalid $/setTrace value: expected 'off', 'messages', or 'verbose', got '{other}'"
        )),
    }
}

fn logging_level_to_rmcp(level: crate::logging::LogLevel) -> LoggingLevel {
    match level {
        crate::logging::LogLevel::Debug => LoggingLevel::Debug,
        crate::logging::LogLevel::Info => LoggingLevel::Info,
        crate::logging::LogLevel::Notice => LoggingLevel::Notice,
        crate::logging::LogLevel::Warning => LoggingLevel::Warning,
        crate::logging::LogLevel::Error => LoggingLevel::Error,
    }
}

fn logging_level_from_rmcp(level: LoggingLevel) -> crate::logging::LogLevel {
    match level {
        LoggingLevel::Debug => crate::logging::LogLevel::Debug,
        LoggingLevel::Info => crate::logging::LogLevel::Info,
        LoggingLevel::Notice => crate::logging::LogLevel::Notice,
        LoggingLevel::Warning => crate::logging::LogLevel::Warning,
        LoggingLevel::Error
        | LoggingLevel::Critical
        | LoggingLevel::Alert
        | LoggingLevel::Emergency => crate::logging::LogLevel::Error,
    }
}

fn authenticated_principal(context: &RequestContext<RoleServer>) -> Option<String> {
    context
        .extensions
        .get::<AuthenticatedPrincipal>()
        .map(|principal| principal.0.clone())
}

fn canonical_repo_roots_from_roots(roots: &[Root]) -> anyhow::Result<Vec<String>> {
    use atlas_repo::{canonical_filesystem_path, find_repo_root};
    use camino::Utf8PathBuf;

    let mut canonical = Vec::new();
    for root in roots {
        let parsed = url::Url::parse(&root.uri)
            .map_err(|error| anyhow::anyhow!("invalid roots/list URI '{}': {error}", root.uri))?;
        anyhow::ensure!(
            parsed.scheme() == "file",
            "unsupported roots/list URI scheme '{}' for '{}'",
            parsed.scheme(),
            root.uri
        );
        let file_path = parsed.to_file_path().map_err(|_| {
            anyhow::anyhow!("roots/list URI is not a valid file path: {}", root.uri)
        })?;
        let utf8 = Utf8PathBuf::from_path_buf(file_path)
            .map_err(|_| anyhow::anyhow!("roots/list URI path is not valid UTF-8: {}", root.uri))?;
        let repo_root = find_repo_root(utf8.as_path()).unwrap_or(utf8);
        canonical.push(
            canonical_filesystem_path(repo_root.as_path())
                .map_err(|error| {
                    anyhow::anyhow!("invalid roots/list repo root '{}': {error}", repo_root)
                })?
                .into_string(),
        );
    }
    canonical.sort();
    canonical.dedup();
    Ok(canonical)
}

fn cache_scope_from_str(scope: &str) -> CacheScope {
    match scope {
        spec::CACHE_SCOPE_PRIVATE => CacheScope::Private,
        _ => CacheScope::Public,
    }
}

fn client_interaction_capabilities(
    client_capabilities: Option<&Value>,
) -> ClientInteractionCapabilities {
    let elicitation = client_capabilities.and_then(|value| value.get("elicitation"));
    ClientInteractionCapabilities {
        supports_elicitation_form: elicitation.and_then(|value| value.get("form")).is_some(),
        supports_elicitation_url: elicitation.and_then(|value| value.get("url")).is_some(),
    }
}

fn request_id_string(context: &RequestContext<RoleServer>) -> String {
    match serde_json::to_value(&context.id) {
        Ok(Value::String(value)) => value,
        Ok(other) => other.to_string(),
        Err(_) => context.id.to_string(),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct BootstrapNotificationPlan {
    send_tool_list_changed: bool,
    send_prompt_list_changed: bool,
    send_resource_list_changed: bool,
    resource_updates: Vec<String>,
}

fn legacy_completion_request_value(request: &CompleteRequestParams) -> anyhow::Result<Value> {
    let mut object = Map::new();
    object.insert(
        "argument".to_owned(),
        serde_json::to_value(&request.argument)?,
    );
    if let Some(context) = &request.context {
        object.insert("context".to_owned(), serde_json::to_value(context)?);
    }
    object.insert(
        "ref".to_owned(),
        legacy_completion_reference_value(&request.r#ref, &request.argument)?,
    );
    Ok(Value::Object(object))
}

fn legacy_completion_reference_value(
    reference: &Reference,
    argument: &ArgumentInfo,
) -> anyhow::Result<Value> {
    match reference {
        Reference::Prompt(prompt) => Ok(json!({ "name": prompt.name })),
        Reference::Resource(resource) => {
            if argument.name == "uri" {
                Ok(json!({ "name": "resources/read" }))
            } else {
                Ok(json!({ "uriTemplate": resource.uri }))
            }
        }
        _ => Err(anyhow::anyhow!(
            "unsupported completion reference type: {}",
            reference.reference_type()
        )),
    }
}

fn map_prompt_error(error: anyhow::Error) -> McpError {
    let detail = error.to_string();
    if detail.starts_with("unknown prompt:")
        || detail.starts_with("missing ")
        || detail.starts_with("argument '")
        || detail.contains("missing required argument:")
        || detail.contains("requires non-empty")
        || detail.contains("must be ")
        || detail.contains("invalid regex pattern")
    {
        crate::rmcp_error::invalid_params(detail, None)
    } else {
        crate::rmcp_error::internal_error(error)
    }
}

fn map_invalid_params_error(error: anyhow::Error) -> McpError {
    crate::rmcp_error::invalid_params(error.to_string(), None)
}

fn map_task_api_error(error: crate::tasks::TaskApiError) -> McpError {
    match error.kind() {
        crate::tasks::TaskApiErrorKind::InvalidParams => {
            crate::rmcp_error::invalid_params(error.message(), None)
        }
        crate::tasks::TaskApiErrorKind::NotFound => {
            crate::rmcp_error::invalid_params(error.message(), None)
        }
        crate::tasks::TaskApiErrorKind::Cancelled
        | crate::tasks::TaskApiErrorKind::Failed
        | crate::tasks::TaskApiErrorKind::Internal
        | crate::tasks::TaskApiErrorKind::NotReady => {
            crate::rmcp_error::internal_error(error.into_anyhow())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AtlasRmcpServer;
    use crate::completion;
    use crate::mrtr::RequestStateBinding;
    use crate::output::OutputFormat;
    use crate::prompts;
    use crate::resources;
    use crate::session_tools;
    use crate::spec;
    use crate::tools;
    use crate::transport::repo_selection::strip_repo_selector_fields;
    use crate::transport::{ActiveRepoContext, ServerOptions, TraceLevel};
    use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind};
    use atlas_session::{DurableTaskStatus, DurableTaskUpdate, NewDurableTask, SessionStore};
    use atlas_store_sqlite::Store;
    use rmcp::ServerHandler;
    use rmcp::model::{
        ArgumentInfo, CallToolRequestParams, CallToolResponse, CancelTaskParams,
        CompleteRequestParams, ErrorCode, GetPromptRequestParams, GetTaskParams, LoggingLevel,
        ReadResourceResponse, Reference, Root, SetLevelRequestParams, SubscriptionFilter,
        UpdateTaskParams,
    };
    use serde_json::{Value, json};
    use std::collections::{BTreeMap, HashMap};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use tempfile::TempDir;

    fn server() -> AtlasRmcpServer {
        AtlasRmcpServer::new(
            "/tmp/repo",
            "/tmp/repo/.atlas/index.db",
            ServerOptions::default(),
        )
    }

    #[test]
    fn constructor_preserves_repo_and_db_paths_exactly() {
        let options = ServerOptions {
            worker_threads: 7,
            tool_timeout_ms: 42_000,
            tool_timeout_ms_by_tool: HashMap::from([("query_graph".to_owned(), 9_001)]),
            #[cfg(feature = "http-transport")]
            http_auth: None,
        };
        let server = AtlasRmcpServer::new(
            "./relative/../repo-root",
            "./relative/../repo-root/.atlas/graph.db",
            options.clone(),
        );
        assert_eq!(server.repo_root(), "./relative/../repo-root");
        assert_eq!(server.db_path(), "./relative/../repo-root/.atlas/graph.db");
        assert_eq!(server.options().worker_threads, options.worker_threads);
        assert_eq!(server.options().tool_timeout_ms, options.tool_timeout_ms);
        assert_eq!(
            server.options().tool_timeout_ms_by_tool,
            options.tool_timeout_ms_by_tool
        );
    }

    #[test]
    fn get_info_matches_current_spec_server_info() {
        let server = server();
        let info = server.get_info();
        let expected = spec::server_info();
        assert_eq!(info.protocol_version.as_str(), spec::MCP_PROTOCOL_VERSION);
        assert_eq!(info.server_info.name, expected.name);
        assert_eq!(info.server_info.version, expected.version);
        assert_eq!(
            info.server_info.description.as_deref(),
            Some(expected.description.as_str())
        );
        assert!(info.capabilities.tools.is_some());
        assert!(info.capabilities.completions.is_some());
        assert!(info.capabilities.extensions.is_some());
        assert_eq!(
            info.capabilities
                .prompts
                .as_ref()
                .and_then(|capability| capability.list_changed),
            Some(true)
        );
        assert_eq!(
            info.capabilities
                .resources
                .as_ref()
                .and_then(|capability| capability.subscribe),
            Some(true)
        );
        assert_eq!(
            info.capabilities
                .resources
                .as_ref()
                .and_then(|capability| capability.list_changed),
            Some(true)
        );
        assert!(info.capabilities.supports_tasks());
    }

    #[test]
    fn supported_protocol_versions_only_exposes_current_version() {
        let versions = server().supported_protocol_versions();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].as_str(), spec::MCP_PROTOCOL_VERSION);
    }

    #[test]
    fn discover_uses_official_result_and_current_cache_policy() {
        let discover = server().discover_result();
        assert_eq!(discover.supported_versions.len(), 1);
        assert_eq!(
            discover.supported_versions[0].as_str(),
            spec::MCP_PROTOCOL_VERSION
        );
        assert_eq!(
            discover.instructions.as_deref(),
            Some(spec::DISCOVER_INSTRUCTIONS)
        );
        assert_eq!(discover.ttl_ms, spec::DISCOVER_CACHE_TTL_MS);
        assert_eq!(
            discover.server_info().expect("server info").name,
            spec::server_info().name
        );
    }

    #[test]
    fn list_tools_names_match_current_tool_registry() {
        let actual = server()
            .list_tools_result()
            .expect("tools")
            .tools
            .into_iter()
            .map(|tool| tool.name.into_owned())
            .collect::<Vec<_>>();
        let expected = json_array_strings(&tools::tool_list()["tools"], "name");
        assert_eq!(actual, expected);
    }

    #[test]
    fn list_prompts_names_match_current_prompt_registry() {
        let actual = server()
            .list_prompts_result()
            .expect("prompts")
            .prompts
            .into_iter()
            .map(|prompt| prompt.name)
            .collect::<Vec<_>>();
        let expected = prompts::prompt_descriptors()
            .into_iter()
            .map(|prompt| prompt.name)
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn list_resources_uris_match_current_resource_registry() {
        let actual = server()
            .list_resources_result(None)
            .expect("resources")
            .resources
            .into_iter()
            .map(|resource| resource.uri)
            .collect::<Vec<_>>();
        let current = resources::resources_list(None).expect("resource list");
        let expected = json_array_strings(&current["resources"], "uri");
        assert_eq!(actual, expected);
    }

    #[test]
    fn list_resource_templates_uri_templates_match_current_registry() {
        let actual = server()
            .list_resource_templates_result(None)
            .expect("resource templates")
            .resource_templates
            .into_iter()
            .map(|template| template.uri_template)
            .collect::<Vec<_>>();
        let current = resources::resources_templates_list(None).expect("template list");
        let expected = json_array_strings(&current["resourceTemplates"], "uriTemplate");
        assert_eq!(actual, expected);
    }

    #[test]
    fn get_prompt_names_match_current_prompt_registry() {
        let fixture = ToolFixture::new();
        let actual = fixture
            .server
            .list_prompts_result()
            .expect("prompts")
            .prompts
            .into_iter()
            .map(|prompt| prompt.name)
            .collect::<Vec<_>>();
        let expected = prompts::prompt_descriptors()
            .into_iter()
            .map(|prompt| prompt.name)
            .collect::<Vec<_>>();
        assert_eq!(actual, expected);
    }

    #[test]
    fn get_prompt_requires_required_arguments() {
        let fixture = ToolFixture::new();

        let inspect_error = fixture
            .server
            .get_prompt_result("inspect_symbol", Some(&serde_json::Map::new()))
            .expect_err("inspect_symbol missing symbol must fail");
        assert_eq!(inspect_error.code, ErrorCode::INVALID_PARAMS);
        assert!(
            inspect_error
                .message
                .as_ref()
                .contains("missing required argument: symbol")
        );

        let plan_error = fixture
            .server
            .get_prompt_result("plan_refactor", Some(&serde_json::Map::new()))
            .expect_err("plan_refactor missing target must fail");
        assert_eq!(plan_error.code, ErrorCode::INVALID_PARAMS);
        assert!(
            plan_error
                .message
                .as_ref()
                .contains("missing required argument: target")
        );
    }

    #[test]
    fn get_prompt_text_matches_current_prompt_renderer() {
        let fixture = ToolFixture::new();
        let request = GetPromptRequestParams::new("inspect_symbol").with_arguments(
            serde_json::from_value(json!({
                "symbol": "src/lib.rs::fn::greet",
                "question": "What depends on this?"
            }))
            .expect("prompt args"),
        );
        let current = prompts::prompt_get(
            "inspect_symbol",
            Some(&json!({
                "symbol": "src/lib.rs::fn::greet",
                "question": "What depends on this?"
            })),
        )
        .expect("handrolled prompt");
        let rmcp = fixture
            .server
            .get_prompt_result(&request.name, request.arguments.as_ref())
            .expect("rmcp prompt");

        assert_eq!(
            rmcp.description.as_deref(),
            current.get("description").and_then(Value::as_str)
        );
        assert_eq!(
            rmcp.messages
                .first()
                .and_then(|message| message.content.as_text())
                .map(|text| text.text.as_str()),
            current
                .pointer("/messages/0/content/text")
                .and_then(Value::as_str)
        );
    }

    #[test]
    fn read_resource_docs_index_matches_current_resource_renderer() {
        let fixture = ToolFixture::new();
        assert_read_resource_matches_handrolled(&fixture, "atlas://docs/index");
    }

    #[test]
    fn read_resource_health_status_matches_current_resource_renderer() {
        let fixture = ToolFixture::new();
        assert_read_resource_matches_handrolled(&fixture, "atlas://health/status");
    }

    #[test]
    fn read_resource_graph_provenance_matches_current_resource_renderer() {
        let fixture = ToolFixture::new();
        assert_read_resource_matches_handrolled(&fixture, "atlas://graph/provenance");
    }

    #[test]
    fn read_resource_saved_context_matches_current_resource_renderer() {
        let fixture = ToolFixture::new();
        crate::tools::call(
            "save_context_artifact",
            Some(&json!({
                "content": "saved context fixture body",
                "kind": "note",
                "source_id": "src-completion-123",
                "output_format": "json"
            })),
            &fixture.repo_root,
            &fixture.db_path,
        )
        .expect("save context artifact");
        assert_read_resource_matches_handrolled(
            &fixture,
            "atlas://saved-context/src-completion-123",
        );
    }

    #[test]
    fn read_resource_tool_docs_matches_current_resource_renderer() {
        let fixture = ToolFixture::new();
        assert_read_resource_matches_handrolled(&fixture, "atlas://tool-docs/query_graph");
    }

    #[test]
    fn read_resource_prompt_docs_matches_current_resource_renderer() {
        let fixture = ToolFixture::new();
        assert_read_resource_matches_handrolled(&fixture, "atlas://prompt-docs/review_change");
    }

    #[test]
    fn read_resource_docs_section_matches_current_resource_renderer() {
        let fixture = ToolFixture::new();
        assert_read_resource_matches_handrolled(&fixture, "atlas://docs/README.md#document.status");
    }

    #[test]
    fn list_resources_preserves_cursor_pagination_behavior() {
        let fixture = ToolFixture::new();
        let actual = fixture
            .server
            .list_resources_result(Some(
                serde_json::from_value(json!({ "cursor": "offset:1" })).expect("paginated params"),
            ))
            .expect("rmcp resources list");
        let expected = resources::resources_list(Some(&json!({ "cursor": "offset:1" })))
            .expect("current resources list");
        assert_eq!(
            actual
                .resources
                .iter()
                .map(|resource| resource.uri.clone())
                .collect::<Vec<_>>(),
            json_array_strings(&expected["resources"], "uri")
        );
        assert_eq!(
            actual.next_cursor.as_deref(),
            expected.get("nextCursor").and_then(Value::as_str)
        );
    }

    #[test]
    fn list_resource_templates_preserves_cursor_pagination_behavior() {
        let fixture = ToolFixture::new();
        let actual = fixture
            .server
            .list_resource_templates_result(Some(
                serde_json::from_value(json!({ "cursor": "offset:1" })).expect("paginated params"),
            ))
            .expect("rmcp resource templates list");
        let expected = resources::resources_templates_list(Some(&json!({ "cursor": "offset:1" })))
            .expect("current resource templates list");
        assert_eq!(
            actual
                .resource_templates
                .iter()
                .map(|template| template.uri_template.clone())
                .collect::<Vec<_>>(),
            json_array_strings(&expected["resourceTemplates"], "uriTemplate")
        );
        assert_eq!(
            actual.next_cursor.as_deref(),
            expected.get("nextCursor").and_then(Value::as_str)
        );
    }

    #[test]
    fn complete_tool_name_matches_current_completion() {
        let fixture = ToolFixture::new();
        assert_completion_matches_handrolled(
            &fixture,
            CompleteRequestParams::new(
                Reference::for_prompt("tools/call"),
                ArgumentInfo::new("name", "get_"),
            ),
        );
    }

    #[test]
    fn complete_prompt_arguments_matches_current_completion() {
        let fixture = ToolFixture::new();
        assert_completion_matches_handrolled(
            &fixture,
            CompleteRequestParams::new(
                Reference::for_prompt("inspect_symbol"),
                ArgumentInfo::new("symbol", "gre"),
            ),
        );
    }

    #[test]
    fn complete_resource_uri_matches_current_completion() {
        let fixture = ToolFixture::new();
        assert_completion_matches_handrolled(
            &fixture,
            CompleteRequestParams::new(
                Reference::for_resource("atlas://docs/index"),
                ArgumentInfo::new("uri", "atlas://"),
            ),
        );
    }

    #[test]
    fn complete_source_id_matches_current_completion() {
        let fixture = ToolFixture::new();
        crate::tools::call(
            "save_context_artifact",
            Some(&json!({
                "content": "saved context fixture body",
                "kind": "note",
                "source_id": "src-completion-123",
                "output_format": "json"
            })),
            &fixture.repo_root,
            &fixture.db_path,
        )
        .expect("save context artifact");
        assert_completion_matches_handrolled(
            &fixture,
            CompleteRequestParams::new(
                Reference::for_resource("atlas://saved-context/{source_id}"),
                ArgumentInfo::new("source_id", "src-"),
            ),
        );
    }

    #[test]
    fn complete_docs_heading_matches_current_completion() {
        let fixture = ToolFixture::new();
        let request = CompleteRequestParams::new(
            Reference::for_resource("atlas://docs/{file}#{heading}"),
            ArgumentInfo::new("heading", "document.st"),
        )
        .with_context(
            serde_json::from_value(json!({
                "arguments": { "file": "README.md" }
            }))
            .expect("completion context"),
        );
        assert_completion_matches_handrolled(&fixture, request);
    }

    #[test]
    fn complete_git_ref_matches_current_completion() {
        let fixture = ToolFixture::new();
        assert_completion_matches_handrolled(
            &fixture,
            CompleteRequestParams::new(
                Reference::for_prompt("review_change"),
                ArgumentInfo::new("base", "ma"),
            ),
        );
    }

    #[test]
    fn complete_unsupported_field_returns_empty_set() {
        let fixture = ToolFixture::new();
        let result = fixture
            .server
            .complete_result(CompleteRequestParams::new(
                Reference::for_prompt("tools/call"),
                ArgumentInfo::new("output_format", "j"),
            ))
            .expect("rmcp completion");
        assert_eq!(result.completion.values, Vec::<String>::new());
        assert_eq!(result.completion.has_more, Some(false));
    }

    #[test]
    fn accepted_subscription_filter_is_supported_subset() {
        let fixture = ToolFixture::new();
        let requested = SubscriptionFilter::builder()
            .tools_list_changed()
            .prompts_list_changed()
            .resources_list_changed()
            .resource_subscription("atlas://docs/index")
            .resource_subscription("atlas://prompt-docs/review_change")
            .build();
        let accepted = fixture
            .server
            .accepted_subscription_filter_result(&requested);
        assert_eq!(accepted, requested);
    }

    #[test]
    fn bootstrap_notification_plan_only_emits_requested_categories() {
        let fixture = ToolFixture::new();
        let accepted = SubscriptionFilter::builder()
            .tools_list_changed()
            .resource_subscription("atlas://docs/index")
            .resource_subscription("atlas://prompt-docs/review_change")
            .build();
        let plan = fixture.server.bootstrap_notification_plan(&accepted);
        assert_eq!(
            plan,
            super::BootstrapNotificationPlan {
                send_tool_list_changed: true,
                send_prompt_list_changed: false,
                send_resource_list_changed: false,
                resource_updates: vec![
                    "atlas://docs/index".to_owned(),
                    "atlas://prompt-docs/review_change".to_owned(),
                ],
            }
        );
    }

    #[test]
    fn call_tool_query_graph_matches_handrolled_structured_content() {
        let fixture = ToolFixture::new();
        let args = json!({"text": "greet", "output_format": "json"});
        assert_call_tool_structured_content_matches_handrolled(&fixture, "query_graph", args);
    }

    #[test]
    fn call_tool_status_matches_handrolled_structured_content() {
        let fixture = ToolFixture::new();
        let args = json!({"output_format": "json"});
        assert_call_tool_structured_content_matches_handrolled(&fixture, "status", args);
    }

    #[test]
    fn call_tool_get_context_matches_handrolled_structured_content() {
        let fixture = ToolFixture::new();
        let args = json!({
            "target": {"kind": "query", "query": "greet"},
            "output_format": "json"
        });
        assert_call_tool_structured_content_matches_handrolled(&fixture, "get_context", args);
    }

    #[test]
    fn call_tool_search_files_matches_handrolled_structured_content() {
        let fixture = ToolFixture::new();
        let args = json!({"pattern": "*.rs", "output_format": "json"});
        assert_call_tool_structured_content_matches_handrolled(&fixture, "search_files", args);
    }

    #[test]
    fn call_tool_invalid_input_returns_user_visible_error_result() {
        let fixture = ToolFixture::new();
        let args = json!({"query": "(", "is_regex": true, "output_format": "json"});

        let handrolled =
            handrolled_tools_call(&fixture, "search_content", &args).expect("handrolled");
        let rmcp = fixture
            .server
            .call_tool_for_tests(call_tool_request("search_content", Some(args.clone())))
            .expect("rmcp call");
        let rmcp_complete = expect_complete(rmcp);

        assert_eq!(rmcp_complete.is_error, Some(true));
        assert_eq!(
            rmcp_complete.structured_content,
            handrolled.get("structuredContent").cloned()
        );
        assert_eq!(
            rmcp_complete
                .content
                .first()
                .and_then(|item| item.as_text())
                .map(|text| text.text.as_str()),
            handrolled
                .pointer("/content/0/text")
                .and_then(Value::as_str)
        );
    }

    #[test]
    fn call_tool_unknown_tool_returns_protocol_error() {
        let fixture = ToolFixture::new();
        let error = fixture
            .server
            .call_tool_for_tests(call_tool_request("missing_tool", Some(json!({}))))
            .expect_err("unknown tool must fail");
        assert_eq!(error.code, ErrorCode::METHOD_NOT_FOUND);
        assert_eq!(error.message.as_ref(), "unknown tool: missing_tool");
        assert_eq!(
            error
                .data
                .as_ref()
                .and_then(|data| data.get("atlas_error_code")),
            Some(&json!("method_not_found"))
        );
    }

    #[test]
    fn call_tool_query_graph_increments_session_event_count() {
        let rmcp_fixture = ToolFixture::new();
        let rmcp_before = session_event_count(&rmcp_fixture.repo_root, &rmcp_fixture.db_path);
        rmcp_fixture
            .server
            .call_tool_for_tests(call_tool_request(
                "query_graph",
                Some(json!({"text": "greet", "output_format": "json"})),
            ))
            .expect("rmcp query_graph");
        let rmcp_after = session_event_count(&rmcp_fixture.repo_root, &rmcp_fixture.db_path);

        let handrolled_fixture = ToolFixture::new();
        let handrolled_before =
            session_event_count(&handrolled_fixture.repo_root, &handrolled_fixture.db_path);
        handrolled_tools_call(
            &handrolled_fixture,
            "query_graph",
            &json!({"text": "greet", "output_format": "json"}),
        )
        .expect("handrolled query_graph");
        let handrolled_after =
            session_event_count(&handrolled_fixture.repo_root, &handrolled_fixture.db_path);

        assert_eq!(
            rmcp_after - rmcp_before,
            handrolled_after - handrolled_before,
            "rmcp delta={}, handrolled delta={}",
            rmcp_after - rmcp_before,
            handrolled_after - handrolled_before,
        );
    }

    #[test]
    fn get_task_returns_official_completed_detailed_task() {
        let fixture = ToolFixture::new();
        seed_durable_task(
            &fixture,
            "task-completed",
            DurableTaskStatus::Completed,
            Some(json!({
                "resultType": "complete",
                "content": [{"type": "text", "text": "done"}],
                "structuredContent": {"ok": true}
            })),
            None,
            None,
            None,
        );

        let result = fixture
            .server
            .get_task_result(GetTaskParams::new("task-completed"))
            .expect("rmcp get_task");

        assert_eq!(result.task.task.task_id, "task-completed");
        assert_eq!(result.task.task.status, rmcp::model::TaskStatus::Completed);
        match result.task.payload {
            rmcp::model::TaskPayload::Completed { result } => {
                assert_eq!(result.get("resultType"), Some(&json!("complete")));
                assert_eq!(result.get("structuredContent"), Some(&json!({"ok": true})));
            }
            other => panic!("expected completed payload, got {other:?}"),
        }
    }

    #[test]
    fn get_task_returns_official_input_required_detailed_task() {
        let fixture = ToolFixture::new();
        seed_durable_task(
            &fixture,
            "task-input-required",
            DurableTaskStatus::InputRequired,
            None,
            None,
            Some(json!({
                "confirmation": {
                    "method": "elicitation/create",
                    "params": {
                        "message": "Confirm destructive action",
                        "requestedSchema": {
                            "type": "object",
                            "properties": {
                                "confirmation": {"type": "string"}
                            },
                            "required": ["confirmation"]
                        }
                    }
                }
            })),
            Some("sealed-request-state"),
        );
        let mut store = SessionStore::open_in_repo(&fixture.repo_root).expect("open session store");
        store
            .update_durable_task(
                "task-input-required",
                &DurableTaskUpdate {
                    progress: Some(json!({"message": "awaiting confirmation", "percentage": 10})),
                    ..Default::default()
                },
            )
            .expect("seed progress");

        let result = fixture
            .server
            .get_task_result(GetTaskParams::new("task-input-required"))
            .expect("rmcp get_task input_required");

        assert_eq!(result.task.task.task_id, "task-input-required");
        assert_eq!(
            result.task.task.status,
            rmcp::model::TaskStatus::InputRequired
        );
        assert_eq!(
            result
                .meta
                .as_ref()
                .and_then(|meta| meta.0.get(crate::rmcp_types::ATLAS_TASK_META_PROGRESS)),
            Some(&json!({"message": "awaiting confirmation", "percentage": 10}))
        );
        assert_eq!(
            result
                .meta
                .as_ref()
                .and_then(|meta| meta.0.get(crate::rmcp_types::ATLAS_TASK_META_REQUEST_STATE)),
            Some(&json!("sealed-request-state"))
        );
        match result.task.payload {
            rmcp::model::TaskPayload::InputRequired { input_requests } => {
                assert!(input_requests.contains_key("confirmation"));
            }
            other => panic!("expected input_required payload, got {other:?}"),
        }
    }

    #[test]
    fn update_task_accepts_rmcp_input_responses() {
        let fixture = ToolFixture::new();
        seed_durable_task(
            &fixture,
            "task-input",
            DurableTaskStatus::InputRequired,
            None,
            None,
            Some(json!({
                "confirmation": {
                    "method": "elicitation/create",
                    "params": {
                        "message": "Confirm destructive action",
                        "requestedSchema": {
                            "type": "object",
                            "properties": {
                                "confirmation": {"type": "string"}
                            },
                            "required": ["confirmation"]
                        }
                    }
                }
            })),
            Some("sealed-request-state"),
        );

        fixture
            .server
            .update_task_result(UpdateTaskParams::new(
                "task-input",
                BTreeMap::from([(String::from("confirmation"), json!({"action": "accept"}))]),
            ))
            .expect("rmcp update_task");

        let updated = SessionStore::open_in_repo(&fixture.repo_root)
            .expect("open session store")
            .get_durable_task("task-input")
            .expect("reload task")
            .expect("task exists");
        assert_eq!(updated.status, DurableTaskStatus::Working);
        assert_eq!(
            updated.progress,
            Some(json!({"clientInput": {"confirmation": {"action": "accept"}}}))
        );
    }

    #[test]
    fn cancel_task_marks_durable_task_cancelled() {
        let fixture = ToolFixture::new();
        seed_durable_task(
            &fixture,
            "task-working",
            DurableTaskStatus::Working,
            None,
            None,
            None,
            None,
        );

        fixture
            .server
            .cancel_task_result(CancelTaskParams::new("task-working"))
            .expect("rmcp cancel_task");

        let cancelled = SessionStore::open_in_repo(&fixture.repo_root)
            .expect("open session store")
            .get_durable_task("task-working")
            .expect("reload task")
            .expect("task exists");
        assert_eq!(cancelled.status, DurableTaskStatus::Cancelled);
        assert!(cancelled.cancel_requested);
    }

    #[test]
    fn ping_returns_successful_empty_result() {
        let fixture = ToolFixture::new();
        fixture.server.ping_result().expect("ping");
    }

    #[test]
    fn initialized_marks_server_ready() {
        let fixture = ToolFixture::new();
        assert!(!fixture.server.is_initialized());
        fixture.server.mark_initialized();
        assert!(fixture.server.is_initialized());
    }

    #[test]
    fn set_level_maps_rmcp_levels_to_atlas_thresholds() {
        let fixture = ToolFixture::new();
        fixture
            .server
            .set_level_result(SetLevelRequestParams::new(LoggingLevel::Warning));
        assert_eq!(
            fixture.server.requested_log_level(),
            Some(crate::logging::LogLevel::Warning)
        );

        fixture
            .server
            .set_level_result(SetLevelRequestParams::new(LoggingLevel::Critical));
        assert_eq!(
            fixture.server.requested_log_level(),
            Some(crate::logging::LogLevel::Error)
        );
    }

    #[test]
    fn set_trace_accepts_supported_values_and_rejects_invalid_values() {
        let fixture = ToolFixture::new();
        fixture
            .server
            .set_trace_level_result(Some(&json!({"value": "messages"})))
            .expect("messages trace level");
        assert_eq!(fixture.server.trace_level(), TraceLevel::Messages);

        fixture
            .server
            .set_trace_level_result(Some(&json!({"value": "verbose"})))
            .expect("verbose trace level");
        assert_eq!(fixture.server.trace_level(), TraceLevel::Verbose);

        let error = fixture
            .server
            .set_trace_level_result(Some(&json!({"value": "loud"})))
            .expect_err("invalid trace must fail");
        assert_eq!(error.code, ErrorCode::INVALID_PARAMS);
        assert!(error.message.as_ref().contains("invalid $/setTrace value"));
    }

    #[test]
    fn logging_notification_shape_uses_official_notification_payload() {
        let notification = super::logging_message_notification_param(
            crate::logging::LogLevel::Warning,
            "demo message",
        );
        let value = serde_json::to_value(notification).expect("serialize logging notification");
        assert_eq!(value["level"], json!("warning"));
        assert_eq!(value["logger"], json!("atlas-mcp"));
        assert_eq!(value["data"], json!("demo message"));
    }

    #[test]
    fn logging_threshold_filters_completion_diagnostics() {
        let ok_response = CallToolResponse::Complete(rmcp::model::CallToolResult::success(vec![]));
        let err_response = CallToolResponse::Complete(rmcp::model::CallToolResult::error(vec![]));

        assert_eq!(
            super::tool_call_log_level(&ok_response),
            crate::logging::LogLevel::Info
        );
        assert_eq!(
            super::tool_call_log_level(&err_response),
            crate::logging::LogLevel::Error
        );
        assert!(!crate::logging::should_emit(
            Some(crate::logging::LogLevel::Warning),
            super::tool_call_log_level(&ok_response)
        ));
        assert!(crate::logging::should_emit(
            Some(crate::logging::LogLevel::Warning),
            super::tool_call_log_level(&err_response)
        ));
    }

    #[test]
    fn custom_trace_unknown_method_returns_method_not_found() {
        let error = crate::rmcp_error::method_not_found("$/unknownTrace".to_owned());
        assert_eq!(error.code, ErrorCode::METHOD_NOT_FOUND);
        assert_eq!(error.message.as_ref(), "$/unknownTrace");
    }

    #[test]
    fn dynamic_roots_refresh_only_runs_when_repo_context_missing() {
        let fixed = ToolFixture::new();
        assert!(!fixed.server.should_refresh_dynamic_roots());

        let dynamic = AtlasRmcpServer::new_with_dynamic_roots(None, None, ServerOptions::default());
        assert!(dynamic.should_refresh_dynamic_roots());

        dynamic.set_candidate_roots_for_tests(Some(vec!["/tmp/repo".to_owned()]));
        assert!(!dynamic.should_refresh_dynamic_roots());
    }

    #[test]
    fn explicit_task_persists_input_required_payload_for_rmcp_tasks_get() {
        let fixture = ToolFixture::new();
        let response = fixture
            .server
            .call_tool_for_tests_with_context(
                call_tool_request(
                    "purge_saved_context",
                    Some(json!({"keep_days": 30, "output_format": "json", "task": {"ttl": 1000}})),
                ),
                super::AtlasRmcpCallContext {
                    request_id: "req-task-input".to_owned(),
                    client_capabilities: Some(json!({"elicitation": {"form": {}}})),
                    authenticated_principal: Some("user@example.com".to_owned()),
                    progress: None,
                },
            )
            .expect("rmcp explicit task");
        let CallToolResponse::Task(task_result) = response else {
            panic!("expected task result");
        };
        let task_id = task_result.task.task_id.clone();

        for _ in 0..50 {
            let task = SessionStore::open_in_repo(&fixture.repo_root)
                .expect("open session store")
                .get_durable_task(&task_id)
                .expect("reload task")
                .expect("task exists");
            if task.status == DurableTaskStatus::InputRequired {
                assert!(task.request_state.as_deref().is_some());
                assert!(
                    task.input_requests
                        .as_ref()
                        .and_then(|value| value.get("confirmation"))
                        .is_some()
                );
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("timed out waiting for input_required durable task state");
    }

    #[test]
    fn purge_confirmation_accept_retry_completes_purge() {
        let fixture = ToolFixture::new();
        let first = fixture
            .server
            .call_tool_for_tests_with_context(
                call_tool_request(
                    "purge_saved_context",
                    Some(json!({"keep_days": 30, "output_format": "json"})),
                ),
                super::AtlasRmcpCallContext {
                    request_id: "req-accept-1".to_owned(),
                    client_capabilities: Some(json!({"elicitation": {"form": {}}})),
                    authenticated_principal: Some("user@example.com".to_owned()),
                    progress: None,
                },
            )
            .expect("first purge call");
        let CallToolResponse::InputRequired(first) = first else {
            panic!("expected input_required first response");
        };
        let request_state = first.request_state.clone().expect("requestState");

        let second = fixture
            .server
            .call_tool_for_tests_with_context(
                call_tool_request(
                    "purge_saved_context",
                    Some(json!({"keep_days": 30, "output_format": "json"})),
                )
                .with_request_state(request_state)
                .with_input_responses(BTreeMap::from([(
                    String::from("confirmation"),
                    json!({
                        "action": "accept",
                        "content": {"confirmation": "confirm"}
                    }),
                )])),
                super::AtlasRmcpCallContext {
                    request_id: "req-accept-2".to_owned(),
                    client_capabilities: Some(json!({"elicitation": {"form": {}}})),
                    authenticated_principal: Some("user@example.com".to_owned()),
                    progress: None,
                },
            )
            .expect("accepted retry");
        let complete = expect_complete(second);
        assert_ne!(complete.is_error, Some(true));
    }

    #[test]
    fn purge_confirmation_decline_retry_returns_user_visible_error() {
        let fixture = ToolFixture::new();
        let first = fixture
            .server
            .call_tool_for_tests_with_context(
                call_tool_request(
                    "purge_saved_context",
                    Some(json!({"keep_days": 30, "output_format": "json"})),
                ),
                super::AtlasRmcpCallContext {
                    request_id: "req-decline-1".to_owned(),
                    client_capabilities: Some(json!({"elicitation": {"form": {}}})),
                    authenticated_principal: Some("user@example.com".to_owned()),
                    progress: None,
                },
            )
            .expect("first purge call");
        let CallToolResponse::InputRequired(first) = first else {
            panic!("expected input_required first response");
        };
        let request_state = first.request_state.clone().expect("requestState");

        let second = fixture
            .server
            .call_tool_for_tests_with_context(
                call_tool_request(
                    "purge_saved_context",
                    Some(json!({"keep_days": 30, "output_format": "json"})),
                )
                .with_request_state(request_state)
                .with_input_responses(BTreeMap::from([(
                    String::from("confirmation"),
                    json!({
                        "action": "decline"
                    }),
                )])),
                super::AtlasRmcpCallContext {
                    request_id: "req-decline-2".to_owned(),
                    client_capabilities: Some(json!({"elicitation": {"form": {}}})),
                    authenticated_principal: Some("user@example.com".to_owned()),
                    progress: None,
                },
            )
            .expect("declined retry");
        let complete = expect_complete(second);
        assert_eq!(complete.is_error, Some(true));
        assert!(
            complete
                .structured_content
                .as_ref()
                .and_then(|value| value.get("message"))
                .and_then(Value::as_str)
                .is_some_and(|message| message.contains("cancelled by client"))
        );
    }

    #[test]
    fn tool_execution_sees_client_capabilities_and_authenticated_principal() {
        let fixture = ToolFixture::new();
        let response = fixture
            .server
            .call_tool_for_tests_with_context(
                call_tool_request(
                    "purge_saved_context",
                    Some(json!({"keep_days": 30, "output_format": "json"})),
                ),
                super::AtlasRmcpCallContext {
                    request_id: "req-1".to_owned(),
                    client_capabilities: Some(json!({"elicitation": {"form": {}}})),
                    authenticated_principal: Some("user@example.com".to_owned()),
                    progress: None,
                },
            )
            .expect("rmcp purge_saved_context");
        let CallToolResponse::InputRequired(result) = response else {
            panic!("expected input_required response");
        };
        let request_state = result.request_state.as_deref().expect("requestState");
        assert!(
            result
                .input_requests
                .as_ref()
                .is_some_and(|requests| !requests.is_empty())
        );
        crate::mrtr::validate_request_state(
            request_state,
            RequestStateBinding {
                method: "tools/call",
                tool: "purge_saved_context",
                arguments: Some(&json!({"keep_days": 30, "output_format": "json"})),
                principal: Some("user@example.com"),
            },
        )
        .expect("requestState binds authenticated principal");
    }

    #[test]
    fn explicit_repo_root_selector_is_canonicalized_on_rmcp_tool_path() {
        let repo_a =
            setup_graph_repo_fixture("src/alpha.rs", "compute", "src/alpha.rs::fn::compute");
        let repo_b = setup_graph_repo_fixture("src/beta.rs", "compute", "src/beta.rs::fn::compute");
        let repo_b_root = repo_b
            ._dir
            .path()
            .join("src")
            .to_string_lossy()
            .into_owned();
        let server = AtlasRmcpServer::new(
            repo_a._dir.path().to_string_lossy().as_ref(),
            &repo_a.db_path,
            ServerOptions::default(),
        );
        let response = server
            .call_tool_for_tests(call_tool_request(
                "query_graph",
                Some(json!({
                    "repo_root": repo_b_root,
                    "text": "compute",
                    "output_format": "json"
                })),
            ))
            .expect("rmcp explicit repo selector");
        let complete = expect_complete(response);
        assert_eq!(
            complete
                .meta
                .as_ref()
                .and_then(|meta| meta.get("atlas:repoRoot")),
            Some(&json!(
                repo_b
                    ._dir
                    .path()
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            ))
        );
    }

    #[test]
    fn explicit_repo_selector_switches_active_repo_in_dynamic_mode() {
        let repo_a = setup_graph_repo_fixture(
            "src/alpha.rs",
            "alpha_compute",
            "src/alpha.rs::fn::alpha_compute",
        );
        let repo_b = setup_graph_repo_fixture(
            "src/beta.rs",
            "beta_compute",
            "src/beta.rs::fn::beta_compute",
        );
        let repo_a_root = repo_a
            ._dir
            .path()
            .canonicalize()
            .expect("canonical repo a")
            .to_string_lossy()
            .into_owned();
        let repo_b_root = repo_b
            ._dir
            .path()
            .canonicalize()
            .expect("canonical repo b")
            .to_string_lossy()
            .into_owned();
        let server = AtlasRmcpServer::new_with_dynamic_roots(None, None, ServerOptions::default());
        server.set_candidate_roots_for_tests(Some(vec![repo_a_root, repo_b_root.clone()]));

        let first = server
            .call_tool_for_tests(call_tool_request(
                "query_graph",
                Some(json!({
                    "repo_root": repo_b_root.clone(),
                    "text": "beta_compute"
                })),
            ))
            .expect("dynamic explicit repo selector");
        let first = expect_complete(first);
        assert_eq!(
            first
                .meta
                .as_ref()
                .and_then(|meta| meta.get("atlas:repoRoot")),
            Some(&json!(repo_b_root.clone()))
        );
        assert_eq!(
            server.repo_resolution().active,
            Some(ActiveRepoContext {
                repo_root: repo_b_root.clone(),
                db_path: atlas_engine::paths::default_db_path(&repo_b_root),
            })
        );

        let second = server
            .call_tool_for_tests(call_tool_request(
                "query_graph",
                Some(json!({
                    "text": "beta_compute"
                })),
            ))
            .expect("cached active repo after explicit selector");
        let second = expect_complete(second);
        assert_eq!(
            second
                .meta
                .as_ref()
                .and_then(|meta| meta.get("atlas:repoSelection"))
                .and_then(|value| value.get("selectionSource")),
            Some(&json!("cached_active_root"))
        );
        assert_eq!(
            second
                .meta
                .as_ref()
                .and_then(|meta| meta.get("atlas:repoRoot")),
            Some(&json!(repo_b_root))
        );
    }

    #[test]
    fn dynamic_multi_workspace_without_selector_fails_closed_with_candidate_roots() {
        let repo_a = setup_graph_repo_fixture(
            "src/alpha.rs",
            "alpha_compute",
            "src/alpha.rs::fn::alpha_compute",
        );
        let repo_b = setup_graph_repo_fixture(
            "src/beta.rs",
            "beta_compute",
            "src/beta.rs::fn::beta_compute",
        );
        let repo_a_root = repo_a
            ._dir
            .path()
            .canonicalize()
            .expect("canonical repo a")
            .to_string_lossy()
            .into_owned();
        let repo_b_root = repo_b
            ._dir
            .path()
            .canonicalize()
            .expect("canonical repo b")
            .to_string_lossy()
            .into_owned();
        let server = AtlasRmcpServer::new_with_dynamic_roots(None, None, ServerOptions::default());
        server.set_candidate_roots_for_tests(Some(vec![repo_a_root.clone(), repo_b_root.clone()]));

        let error = server
            .call_tool_for_tests(call_tool_request(
                "query_graph",
                Some(json!({
                    "text": "compute"
                })),
            ))
            .expect_err("dynamic ambiguity should fail closed");
        assert_eq!(error.code, rmcp::model::ErrorCode::INVALID_PARAMS);
        assert!(
            error
                .message
                .contains("ambiguous across multiple workspace roots")
        );
        let data = error.data.expect("repo-selection error data");
        assert_eq!(data["atlas_error_code"], json!("invalid_params"));
        assert_eq!(
            data["atlas_repo_selection"]["candidate_roots"],
            json!([repo_a_root, repo_b_root])
        );
        assert_eq!(
            data["atlas_repo_selection"]["session_mode"],
            json!("dynamic")
        );
    }

    #[test]
    fn roots_list_changed_invalidates_cached_candidate_roots_in_dynamic_mode() {
        let server = AtlasRmcpServer::new_with_dynamic_roots(None, None, ServerOptions::default());
        server.set_candidate_roots_for_tests(Some(vec!["/tmp/demo".to_owned()]));
        server.invalidate_dynamic_roots();
        assert_eq!(server.repo_resolution().candidate_roots, None);
    }

    #[test]
    fn roots_list_canonicalizes_noncanonical_file_uris() {
        let repo = tempfile::tempdir().expect("repo tempdir");
        fs::create_dir_all(repo.path().join(".git")).expect("create git dir");
        fs::create_dir_all(repo.path().join("src")).expect("create src");
        let nested = repo.path().join("src");
        let uri = url::Url::from_file_path(&nested)
            .expect("file url")
            .to_string();
        let server = AtlasRmcpServer::new_with_dynamic_roots(None, None, ServerOptions::default());
        let roots = vec![Root::new(uri)];
        let canonical = server.load_client_roots_result(&roots).expect("load roots");
        assert_eq!(canonical.len(), 1);
        assert_eq!(
            canonical[0],
            repo.path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        );
        assert_eq!(server.repo_resolution().candidate_roots, Some(canonical));
    }

    fn json_array_strings(array: &Value, key: &str) -> Vec<String> {
        array
            .as_array()
            .expect("array")
            .iter()
            .map(|item| item[key].as_str().expect("string").to_owned())
            .collect()
    }

    fn call_tool_request(name: &str, args: Option<Value>) -> CallToolRequestParams {
        let mut request = CallToolRequestParams::new(name.to_owned());
        if let Some(args) = args {
            request = request.with_arguments(
                serde_json::from_value(args).expect("arguments object for CallToolRequestParams"),
            );
        }
        request
    }

    fn expect_complete(response: CallToolResponse) -> rmcp::model::CallToolResult {
        match response {
            CallToolResponse::Complete(result) => result,
            other => panic!("expected complete response, got {other:?}"),
        }
    }

    fn expect_read_resource_complete(
        response: ReadResourceResponse,
    ) -> rmcp::model::ReadResourceResult {
        match response {
            ReadResourceResponse::Complete(result) => result,
            other => panic!("expected complete resource response, got {other:?}"),
        }
    }

    fn assert_call_tool_structured_content_matches_handrolled(
        fixture: &ToolFixture,
        tool_name: &str,
        args: Value,
    ) {
        let handrolled = handrolled_tools_call(fixture, tool_name, &args)
            .expect("handrolled tools/call response");
        let rmcp = fixture
            .server
            .call_tool_for_tests(call_tool_request(tool_name, Some(args)))
            .expect("rmcp tools/call response");
        let rmcp_complete = expect_complete(rmcp);

        assert_eq!(
            rmcp_complete.structured_content,
            handrolled.get("structuredContent").cloned(),
            "structured content mismatch for {tool_name}"
        );
    }

    fn assert_read_resource_matches_handrolled(fixture: &ToolFixture, uri: &str) {
        let handrolled = resources::resources_read(
            Some(&json!({ "uri": uri })),
            &fixture.repo_root,
            &fixture.db_path,
        )
        .expect("handrolled resource read");
        let rmcp = fixture
            .server
            .read_resource_result(uri)
            .expect("rmcp resource read");
        let rmcp_complete =
            expect_read_resource_complete(ReadResourceResponse::Complete(rmcp.clone()));

        assert_eq!(
            serde_json::to_value(&rmcp_complete).expect("serialize rmcp resource"),
            handrolled
        );
    }

    fn assert_completion_matches_handrolled(fixture: &ToolFixture, request: CompleteRequestParams) {
        let legacy_request =
            super::legacy_completion_request_value(&request).expect("legacy completion request");
        let handrolled =
            completion::complete(Some(&legacy_request), &fixture.repo_root, &fixture.db_path)
                .expect("handrolled completion");
        let rmcp = fixture
            .server
            .complete_result(request)
            .expect("rmcp completion");
        let handrolled_values = handrolled["completion"]["values"]
            .as_array()
            .expect("handrolled values")
            .iter()
            .map(|item| item["value"].as_str().expect("value").to_owned())
            .collect::<Vec<_>>();

        assert_eq!(rmcp.completion.values, handrolled_values);
        assert_eq!(
            rmcp.completion.total,
            handrolled["completion"]["total"]
                .as_u64()
                .map(|value| value as u32)
        );
        assert_eq!(
            rmcp.completion.has_more,
            handrolled["completion"]["hasMore"].as_bool()
        );
    }

    fn handrolled_tools_call(
        fixture: &ToolFixture,
        tool_name: &str,
        args: &Value,
    ) -> anyhow::Result<Value> {
        let request_params = json!({
            "name": tool_name,
            "arguments": args,
        });
        let stripped_args = Some(strip_repo_selector_fields(args.clone()));
        let runtime_context = crate::runtime_context::RequestContext::new(
            std::sync::Arc::new(|_| Ok(())),
            crate::runtime_context::ClientInteractionCapabilities::default(),
            "stdio",
            None,
            None,
            "1",
            "tools/call",
            Some(request_params.clone()),
        );
        crate::runtime_context::install(runtime_context);
        crate::tasks::install_tool_call_request_params(Some(&request_params));
        let result = crate::tasks::execute_tool_call(
            tool_name,
            stripped_args,
            &fixture.repo_root,
            &fixture.db_path,
        );
        crate::tasks::uninstall_tool_call_request_params();
        crate::runtime_context::uninstall();
        result
    }

    fn session_event_count(repo_root: &str, db_path: &str) -> i64 {
        tool_body(
            &session_tools::tool_get_session_status(None, repo_root, db_path, OutputFormat::Json)
                .expect("session status"),
        )["event_count"]
            .as_i64()
            .expect("event_count")
    }

    fn tool_body(result: &Value) -> Value {
        result
            .get("structuredContent")
            .cloned()
            .or_else(|| {
                result
                    .get("content")
                    .and_then(|content| content.get(0))
                    .and_then(|item| item.get("text"))
                    .and_then(Value::as_str)
                    .and_then(|text| serde_json::from_str(text).ok())
            })
            .expect("tool body")
    }

    struct ToolFixture {
        _dir: TempDir,
        repo_root: String,
        db_path: String,
        server: AtlasRmcpServer,
    }

    struct RepoSelectionFixture {
        _dir: TempDir,
        db_path: String,
    }

    impl ToolFixture {
        fn new() -> Self {
            let (dir, _db_path_path, db_path) = setup_repo();
            let repo_root = dir.path().to_string_lossy().into_owned();
            crate::tools::call(
                "build_or_update_graph",
                Some(&json!({"operation": {"kind": "build"}, "output_format": "json"})),
                &repo_root,
                &db_path,
            )
            .expect("build graph");
            seed_schema_graph(&db_path);
            Self {
                server: AtlasRmcpServer::new(&repo_root, &db_path, ServerOptions::default()),
                _dir: dir,
                repo_root,
                db_path,
            }
        }
    }

    fn seed_durable_task(
        fixture: &ToolFixture,
        task_id: &str,
        status: DurableTaskStatus,
        result: Option<Value>,
        error: Option<Value>,
        input_requests: Option<Value>,
        request_state: Option<&str>,
    ) {
        let mut store = SessionStore::open_in_repo(&fixture.repo_root).expect("open session store");
        store
            .create_durable_task(&NewDurableTask {
                task_id: task_id.to_owned(),
                originating_method: "tools/call".to_owned(),
                request_id: Some("request-1".to_owned()),
                tool_name: Some("doctor".to_owned()),
                transport_kind: Some("rmcp".to_owned()),
                session_id: None,
                status: DurableTaskStatus::Working,
                status_message: Some("working".to_owned()),
                ttl_ms: Some(5_000),
            })
            .expect("create durable task");
        store
            .update_durable_task(
                task_id,
                &DurableTaskUpdate {
                    status: Some(status),
                    status_message: Some(status.as_str().to_owned()),
                    result,
                    error,
                    input_requests,
                    request_state: request_state.map(str::to_owned),
                    ..Default::default()
                },
            )
            .expect("update durable task");
    }

    fn setup_graph_repo_fixture(
        primary_file: &str,
        primary_name: &str,
        primary_qn: &str,
    ) -> RepoSelectionFixture {
        let dir = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(dir.path().join(".git")).expect("create git dir");
        let db_path = dir.path().join(".atlas").join("worldtree.db");
        fs::create_dir_all(db_path.parent().expect("atlas dir")).expect("create atlas dir");
        if let Some(parent) = Path::new(primary_file).parent() {
            fs::create_dir_all(dir.path().join(parent)).expect("create primary parent dir");
        }
        let db_path = db_path.to_string_lossy().to_string();

        let mut store = Store::open(&db_path).expect("open store");
        let primary = make_node(
            atlas_core::NodeKind::Function,
            primary_name,
            primary_qn,
            primary_file,
        );
        store
            .replace_file_graph(
                primary_file,
                &format!("hash:{primary_file}"),
                Some("rust"),
                Some(5),
                std::slice::from_ref(&primary),
                &[],
            )
            .expect("replace primary graph");

        RepoSelectionFixture { _dir: dir, db_path }
    }

    fn setup_repo() -> (TempDir, PathBuf, String) {
        let dir = tempfile::tempdir().expect("tempdir");
        let src_dir = dir.path().join("src");
        fs::create_dir_all(&src_dir).expect("create src dir");
        fs::write(
            src_dir.join("lib.rs"),
            "pub mod service;\npub fn greet() -> &'static str { \"hi\" }\n",
        )
        .expect("write fixture source");
        fs::write(
            src_dir.join("service.rs"),
            "pub fn compute() -> i32 { 1 }\n",
        )
        .expect("write fixture service source");
        fs::write(
            src_dir.join("api.rs"),
            "pub fn handle_request() -> i32 { crate::service::compute() }\n",
        )
        .expect("write fixture api source");
        let tests_dir = dir.path().join("tests");
        fs::create_dir_all(&tests_dir).expect("create tests dir");
        fs::write(
            tests_dir.join("service_test.rs"),
            "#[test]\nfn compute_test() { assert_eq!(crate::service::compute(), 1); }\n",
        )
        .expect("write fixture test source");
        fs::write(
            dir.path().join("README.md"),
            "# Fixture Repo\n\n## Status\n\nFixture status content.\n",
        )
        .expect("write fixture readme");
        fs::create_dir_all(dir.path().join("config")).expect("create config dir");
        fs::write(dir.path().join("config/app.toml"), "name = \"fixture\"\n")
            .expect("write fixture config");
        fs::create_dir_all(dir.path().join("templates")).expect("create templates dir");
        fs::write(
            dir.path().join("templates/index.html"),
            "<html><body>{{ greet }}</body></html>\n",
        )
        .expect("write fixture template");
        fs::create_dir_all(dir.path().join("queries")).expect("create queries dir");
        fs::write(dir.path().join("queries/example.sql"), "select 1;\n")
            .expect("write fixture sql");
        git(dir.path(), &["init", "--quiet"]);
        git(dir.path(), &["config", "user.name", "Atlas Tests"]);
        git(
            dir.path(),
            &["config", "user.email", "atlas-tests@example.com"],
        );
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "--quiet", "-m", "fixture baseline"]);
        let db_path = dir.path().join(".atlas").join("worldtree.db");
        (dir, db_path.clone(), db_path.to_string_lossy().into_owned())
    }

    fn git(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn make_node(kind: NodeKind, name: &str, qn: &str, file: &str) -> Node {
        Node {
            id: NodeId::UNSET,
            kind,
            name: name.to_owned(),
            qualified_name: qn.to_owned(),
            file_path: file.to_owned(),
            line_start: 1,
            line_end: 5,
            language: "rust".to_owned(),
            parent_name: None,
            params: Some("()".to_owned()),
            return_type: None,
            modifiers: None,
            is_test: kind == NodeKind::Test,
            file_hash: format!("hash:{file}"),
            extra_json: serde_json::json!({}),
            repo_provenance: None,
        }
    }

    fn make_edge(kind: EdgeKind, source_qn: &str, target_qn: &str, file: &str) -> Edge {
        Edge {
            id: 0,
            kind,
            source_qn: source_qn.to_owned(),
            target_qn: target_qn.to_owned(),
            file_path: file.to_owned(),
            line: Some(1),
            confidence: 1.0,
            confidence_tier: None,
            extra_json: serde_json::json!({}),
            repo_provenance: None,
        }
    }

    fn seed_schema_graph(db_path: &str) {
        let mut store = Store::open(db_path).expect("open store");

        let greet = make_node(
            NodeKind::Function,
            "greet",
            "src/lib.rs::fn::greet",
            "src/lib.rs",
        );
        store
            .replace_file_graph(
                "src/lib.rs",
                "hash:src/lib.rs",
                Some("rust"),
                Some(5),
                std::slice::from_ref(&greet),
                &[],
            )
            .expect("seed lib graph");

        let compute = make_node(
            NodeKind::Function,
            "compute",
            "src/service.rs::fn::compute",
            "src/service.rs",
        );
        store
            .replace_file_graph(
                "src/service.rs",
                "hash:src/service.rs",
                Some("rust"),
                Some(5),
                std::slice::from_ref(&compute),
                &[],
            )
            .expect("seed service graph");

        let handle = make_node(
            NodeKind::Function,
            "handle_request",
            "src/api.rs::fn::handle_request",
            "src/api.rs",
        );
        let handle_calls_compute = make_edge(
            EdgeKind::Calls,
            "src/api.rs::fn::handle_request",
            "src/service.rs::fn::compute",
            "src/api.rs",
        );
        store
            .replace_file_graph(
                "src/api.rs",
                "hash:src/api.rs",
                Some("rust"),
                Some(5),
                std::slice::from_ref(&handle),
                &[handle_calls_compute],
            )
            .expect("seed api graph");

        let compute_test = make_node(
            NodeKind::Test,
            "compute_test",
            "tests/service_test.rs::fn::compute_test",
            "tests/service_test.rs",
        );
        let test_targets_compute = make_edge(
            EdgeKind::Tests,
            "tests/service_test.rs::fn::compute_test",
            "src/service.rs::fn::compute",
            "tests/service_test.rs",
        );
        store
            .replace_file_graph(
                "tests/service_test.rs",
                "hash:tests/service_test.rs",
                Some("rust"),
                Some(5),
                std::slice::from_ref(&compute_test),
                &[test_targets_compute],
            )
            .expect("seed test graph");
    }
}
