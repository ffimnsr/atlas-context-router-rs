use std::io::{IsTerminal, Read};

use anyhow::{Context, Result};
use camino::Utf8Path;
use serde_json::{Value, json};

use atlas_adapters::{
    ArtifactIdentity, derive_content_db_path, derive_session_db_path, generate_source_id,
    normalize_event, redact_payload,
};
use atlas_contentstore::{ContentStore, OutputRouting, SourceMeta};
use atlas_repo::find_repo_root;
use atlas_session::{NewSessionEvent, SessionEventType, SessionId, SessionStore};

use crate::cli::Cli;
use crate::cli_paths::canonicalize_cli_path;

use super::super::resolve_repo;
use super::metadata::build_hook_event_metadata;
use super::payload::sanitize_payload_for_storage;
use super::policy::{
    HookEventParts, HookMetadataContext, HookPayloadRouting, HookPersistence, HookPolicy,
    MAX_HOOK_STDIN_BYTES, resolve_hook_policy,
};

pub(crate) fn resolve_hook_repo(cli: &Cli) -> Result<String> {
    if cli.repo.is_some() {
        return canonicalize_cli_path(&resolve_repo(cli)?);
    }

    if let Ok(script_path) = std::env::var("ATLAS_HOOK_SCRIPT_PATH") {
        let script_path = script_path.trim();
        if !script_path.is_empty() {
            let script_path = Utf8Path::new(script_path);
            let start = if script_path.is_file() {
                script_path.parent().unwrap_or(script_path)
            } else {
                script_path
            };
            if let Ok(root) = find_repo_root(start) {
                return canonicalize_cli_path(root.as_str());
            }
        }
    }

    let cwd = resolve_repo(cli)?;
    if let Ok(root) = find_repo_root(Utf8Path::new(&cwd)) {
        return canonicalize_cli_path(root.as_str());
    }

    canonicalize_cli_path(&cwd)
}

pub(crate) fn persist_hook_event(
    repo: &str,
    graph_db_path: &str,
    frontend: &str,
    event: &str,
    payload: Value,
) -> Result<HookPersistence> {
    let session_id = SessionId::derive(repo, "", frontend);
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
        source_type: "hook_event".to_owned(),
        label,
        repo_root: Some(repo.to_owned()),
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

pub(crate) fn hook_frontend() -> String {
    std::env::var("ATLAS_HOOK_FRONTEND")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "hook".to_owned())
}

pub(crate) fn read_hook_payload() -> Result<Value> {
    let stdin = std::io::stdin();
    let stdin_is_terminal = stdin.is_terminal();
    read_hook_payload_from(stdin, stdin_is_terminal)
}

pub(crate) fn read_hook_payload_from<R: Read>(reader: R, stdin_is_terminal: bool) -> Result<Value> {
    if stdin_is_terminal {
        return Ok(Value::Null);
    }

    let mut raw = String::new();
    reader
        .take(MAX_HOOK_STDIN_BYTES)
        .read_to_string(&mut raw)
        .context("cannot read hook payload from stdin")?;

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Value::Null);
    }

    let parsed =
        serde_json::from_str::<Value>(trimmed).unwrap_or_else(|_| json!({ "raw": trimmed }));
    Ok(redact_payload(parsed))
}

pub(crate) fn build_hook_event(
    session_id: &SessionId,
    parts: HookEventParts<'_>,
) -> NewSessionEvent {
    let policy = resolve_hook_policy(parts.event).expect("recognized hook event");
    let mut payload = json!({
        "frontend": parts.frontend,
        "hook_event": policy.canonical_event,
        "payload": parts.payload,
        "hook_metadata": parts.hook_metadata,
    });

    if let Some(obj) = payload.as_object_mut() {
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
