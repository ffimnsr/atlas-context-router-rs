use anyhow::{Context, Result};
use serde_json::{Value, json};

use atlas_adapters::{
    ArtifactIdentity, derive_content_db_path, derive_session_db_path, generate_source_id,
    normalize_event,
};
use atlas_contentstore::{ContentStore, OutputRouting, SourceMeta};
use atlas_session::{NewSessionEvent, SessionEventType, SessionId, SessionStore};

use crate::actions::execute_hook_actions;
use crate::metadata::build_hook_event_metadata;
use crate::payload::sanitize_payload_for_storage;
use crate::policy::{
    HookEventParts, HookMetadataContext, HookPayloadRouting, HookPersistence, HookPolicy,
    resolve_hook_policy,
};

struct PersistEventContext<'a> {
    repo: &'a str,
    graph_db_path: &'a str,
    frontend: &'a str,
    event: &'a str,
    source: AgentEventSource,
    agent_id: Option<&'a str>,
}

/// Where an agent event entered the pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentEventSource {
    /// Native host hook (`atlas hook <event>`).
    Hook,
    /// Instruction-driven MCP fallback capture.
    McpFallback,
}

impl AgentEventSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hook => "hook",
            Self::McpFallback => "mcp_fallback",
        }
    }
}

/// Input for [`record_agent_event`].
///
/// `repo_root` MUST be a canonical repo path; it feeds artifact labels, source
/// ids, session derivation, and content-store routing (see crate docs).
pub struct AgentEventRequest {
    pub repo_root: String,
    pub graph_db_path: String,
    pub frontend: String,
    pub event: String,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub payload: Value,
    pub source: AgentEventSource,
}

/// Normalized outcome of a recorded agent event.
#[derive(Debug)]
pub struct AgentEventResult {
    pub event: String,
    pub canonical_event: String,
    pub frontend: String,
    pub session_id: String,
    pub pending_resume: bool,
    pub stored: bool,
    pub event_id: Option<i64>,
    pub source_id: Option<String>,
    pub storage_kind: Option<String>,
    pub snapshot: Option<Value>,
    pub actions: Value,
    pub warnings: Vec<String>,
}

/// Validate the event alias, persist the event, and execute policy actions.
///
/// This is the single entry point shared by native hooks and the MCP fallback
/// surface. Persistence and action failures are surfaced through `Result` /
/// action metadata; callers keep their own best-effort policy on top.
pub fn record_agent_event(req: AgentEventRequest) -> Result<AgentEventResult> {
    let policy = resolve_hook_policy(&req.event)?;
    let session_id = req
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|sid| !sid.is_empty())
        .map(|sid| SessionId(sid.to_owned()));
    let persisted = match session_id {
        Some(session_id) => persist_hook_event_with_session(
            PersistEventContext {
                repo: &req.repo_root,
                graph_db_path: &req.graph_db_path,
                frontend: &req.frontend,
                event: &req.event,
                source: req.source,
                agent_id: req.agent_id.as_deref(),
            },
            session_id,
            req.payload.clone(),
        )?,
        None => persist_hook_event_with_source(
            &req.repo_root,
            &req.graph_db_path,
            &req.frontend,
            &req.event,
            req.payload.clone(),
            req.source,
            req.agent_id.as_deref(),
        )?,
    };
    let actions = execute_hook_actions(
        &req.repo_root,
        &req.graph_db_path,
        &req.frontend,
        policy,
        &persisted,
        &req.payload,
    );

    Ok(AgentEventResult {
        event: req.event,
        canonical_event: policy.canonical_event.to_owned(),
        frontend: req.frontend,
        session_id: persisted.session_id.as_str().to_owned(),
        pending_resume: persisted.pending_resume,
        stored: persisted.stored_event_id.is_some(),
        event_id: persisted.stored_event_id,
        source_id: persisted.source_id,
        storage_kind: persisted.storage_kind.map(str::to_owned),
        snapshot: persisted.snapshot,
        actions,
        warnings: Vec::new(),
    })
}

pub fn persist_hook_event(
    repo: &str,
    graph_db_path: &str,
    frontend: &str,
    event: &str,
    payload: Value,
) -> Result<HookPersistence> {
    persist_hook_event_with_source(
        repo,
        graph_db_path,
        frontend,
        event,
        payload,
        AgentEventSource::Hook,
        None,
    )
}

fn persist_hook_event_with_source(
    repo: &str,
    graph_db_path: &str,
    frontend: &str,
    event: &str,
    payload: Value,
    source: AgentEventSource,
    agent_id: Option<&str>,
) -> Result<HookPersistence> {
    let session_id = SessionId::derive(repo, "", frontend);
    persist_hook_event_with_session(
        PersistEventContext {
            repo,
            graph_db_path,
            frontend,
            event,
            source,
            agent_id,
        },
        session_id,
        payload,
    )
}

fn persist_hook_event_with_session(
    context: PersistEventContext<'_>,
    session_id: SessionId,
    payload: Value,
) -> Result<HookPersistence> {
    let repo = context.repo;
    let graph_db_path = context.graph_db_path;
    let frontend = context.frontend;
    let event = context.event;
    let source = context.source;
    let agent_id = context.agent_id;
    let session_db_path = derive_session_db_path(graph_db_path);
    let mut store = SessionStore::open(&session_db_path)
        .with_context(|| format!("cannot open session store at {session_db_path}"))?;
    store
        .upsert_session_meta(session_id.clone(), repo, frontend, None)
        .context("cannot register hook session")?;

    let pending_resume = store
        .get_resume_snapshot(&session_id)?
        .as_ref()
        .is_some_and(|snapshot| !snapshot.consumed);

    let policy = resolve_hook_policy(event)?;
    let sanitized_payload = sanitize_payload_for_storage(policy, payload);
    let routed = route_hook_payload(
        repo,
        graph_db_path,
        &session_id,
        frontend,
        policy,
        sanitized_payload.clone(),
        agent_id,
    )?;
    let hook_metadata = build_hook_event_metadata(HookMetadataContext {
        repo,
        graph_db_path,
        store: &store,
        session_id: &session_id,
        policy,
        payload: &sanitized_payload,
        routed: &routed,
        pending_resume,
        event_source: source.as_str(),
        agent_id,
    });
    let event_row = build_hook_event(
        &session_id,
        HookEventParts {
            frontend,
            event,
            payload: routed.event_payload,
            hook_metadata,
            source_id: routed.source_id.as_deref(),
            storage_kind: routed.storage_kind,
            pending_resume,
            event_source: source.as_str(),
            agent_id,
        },
    );
    let stored_event_id = store.append_event(event_row)?.map(|row| row.id);

    let snapshot = if policy.build_resume_snapshot {
        let built = store.build_resume(&session_id)?;
        Some(json!({
            "event_count": built.event_count,
            "consumed": built.consumed,
            "updated_at": built.updated_at,
        }))
    } else {
        None
    };

    Ok(HookPersistence {
        session_id,
        pending_resume,
        stored_event_id,
        snapshot,
        source_id: routed.source_id,
        storage_kind: routed.storage_kind,
    })
}

fn route_hook_payload(
    repo: &str,
    graph_db_path: &str,
    session_id: &SessionId,
    frontend: &str,
    policy: &HookPolicy,
    payload: Value,
    agent_id: Option<&str>,
) -> Result<HookPayloadRouting> {
    if payload.is_null() {
        return Ok(HookPayloadRouting {
            event_payload: payload,
            source_id: None,
            storage_kind: None,
        });
    }

    let raw_payload = serde_json::to_string(&payload).context("cannot serialize hook payload")?;
    let label = format!("hook:{frontend}:{}", policy.canonical_event);
    let mut content_store = ContentStore::open(&derive_content_db_path(graph_db_path))
        .context("cannot open hook content store")?;
    content_store
        .migrate()
        .context("cannot migrate hook content store")?;

    let identity = ArtifactIdentity::artifact_label(format!("{repo}:{label}"));
    let meta = SourceMeta {
        id: generate_source_id(&identity, &raw_payload),
        session_id: Some(session_id.as_str().to_owned()),
        agent_id: agent_id.map(str::to_owned),
        source_type: "hook_event".to_owned(),
        label,
        repo_root: Some(repo.to_owned()),
        repo_roots: vec![repo.to_owned()],
        identity_kind: identity.kind_str().to_owned(),
        identity_value: identity.value().to_owned(),
    };

    match content_store.route_output(meta, &raw_payload, "application/json")? {
        OutputRouting::Raw(_) => Ok(HookPayloadRouting {
            event_payload: payload,
            source_id: None,
            storage_kind: None,
        }),
        OutputRouting::Preview { source_id, preview } => Ok(HookPayloadRouting {
            event_payload: json!({ "preview": preview }),
            source_id: Some(source_id),
            storage_kind: Some("preview"),
        }),
        OutputRouting::Pointer { source_id } => Ok(HookPayloadRouting {
            event_payload: Value::Null,
            source_id: Some(source_id),
            storage_kind: Some("pointer"),
        }),
    }
}

pub fn build_hook_event(session_id: &SessionId, parts: HookEventParts<'_>) -> NewSessionEvent {
    let policy = resolve_hook_policy(parts.event).expect("recognized hook event");
    let mut payload = json!({
        "frontend": parts.frontend,
        "event_source": parts.event_source,
        "hook_event": policy.canonical_event,
        "payload": parts.payload,
        "hook_metadata": parts.hook_metadata,
    });

    if let Some(obj) = payload.as_object_mut() {
        if let Some(agent_id) = parts.agent_id {
            obj.insert("agent_id".to_owned(), Value::String(agent_id.to_owned()));
        }
        if let Some(source_id) = parts.source_id {
            obj.insert("source_id".to_owned(), Value::String(source_id.to_owned()));
        }
        if let Some(storage_kind) = parts.storage_kind {
            obj.insert(
                "payload_storage".to_owned(),
                json!({ "kind": storage_kind, "content_type": "application/json" }),
            );
        }
    }

    if policy.session_start {
        NewSessionEvent {
            session_id: session_id.clone(),
            event_type: if parts.pending_resume {
                SessionEventType::SessionResume
            } else {
                SessionEventType::SessionStart
            },
            priority: policy.priority,
            payload: json!({
                "frontend": parts.frontend,
                "event_source": parts.event_source,
                "agent_id": parts.agent_id,
                "hook_event": policy.canonical_event,
                "pending_resume": parts.pending_resume,
                "payload": payload["payload"].clone(),
                "hook_metadata": payload["hook_metadata"].clone(),
            }),
            created_at: None,
        }
    } else {
        normalize_event(policy.event_type.clone(), policy.priority, payload)
            .bind(session_id.clone())
    }
}
