//! Shared test fixtures for the session-tools unit tests (compiled only
//! under `cfg(test)`).

use super::tool_save_context_artifact;
use crate::output::OutputFormat;
use atlas_adapters::derive_session_db_path;
use atlas_session::{NewSessionEvent, SessionEventType, SessionId, SessionStore};
use serde_json::Value;
use tempfile::TempDir;

pub(super) fn setup_db_path(dir: &TempDir) -> String {
    dir.path()
        .join(".atlas")
        .join("worldtree.db")
        .to_string_lossy()
        .into_owned()
}

pub(super) fn setup_multi_repo_registry(repo_root: &str) -> String {
    use atlas_repo::{
        RepoRegistration, RepoRegistry, RepoRelationship, RepoRelationshipKind, TrustState,
        VcsMetadata, stable_repo_id,
    };
    use camino::Utf8Path;

    let root = Utf8Path::new(repo_root);
    let dep = root.join("dep-repo");
    std::fs::create_dir_all(dep.as_std_path()).unwrap();
    let dep_id = stable_repo_id(dep.as_path());
    let mut registry = RepoRegistry::new(stable_repo_id(root));
    registry.registrations = vec![
        RepoRegistration {
            repo_id: stable_repo_id(root),
            root: root.to_path_buf(),
            display_alias: ".".to_owned(),
            vcs: VcsMetadata {
                head: None,
                default_branch: None,
                remote_url: None,
            },
            relationship: RepoRelationship {
                kind: RepoRelationshipKind::Root,
                parent_repo_id: None,
                parent_path: None,
            },
            trust_state: TrustState::Trusted,
            enabled: true,
            include_globs: None,
            exclude_globs: None,
            dependencies: Vec::new(),
        },
        RepoRegistration {
            repo_id: dep_id.clone(),
            root: dep,
            display_alias: "dep-repo".to_owned(),
            vcs: VcsMetadata {
                head: None,
                default_branch: None,
                remote_url: None,
            },
            relationship: RepoRelationship {
                kind: RepoRelationshipKind::Submodule,
                parent_repo_id: Some(stable_repo_id(root)),
                parent_path: Some("dep-repo".to_owned()),
            },
            trust_state: TrustState::Trusted,
            enabled: true,
            include_globs: None,
            exclude_globs: None,
            dependencies: Vec::new(),
        },
    ];
    registry.save(root).unwrap();
    dep_id
}

pub(super) fn tool_body(result: &Value) -> Value {
    result
        .get("structuredContent")
        .cloned()
        .or_else(|| {
            result
                .get("content")
                .and_then(|content| content.get(0))
                .and_then(|item| item.get("text"))
                .and_then(|text| text.as_str())
                .and_then(|text| serde_json::from_str(text).ok())
        })
        .expect("tool body")
}

pub(super) fn install_purge_request_context(params: Value) {
    let client = crate::runtime_context::RequestContext::new(
        std::sync::Arc::new(|_| Ok(())),
        crate::runtime_context::ClientInteractionCapabilities {
            supports_elicitation_form: true,
            supports_elicitation_url: false,
            supports_tasks: false,
        },
        "stdio",
        None,
        None,
        "1",
        "tools/call",
        Some(params),
    );
    crate::runtime_context::install(client);
}

pub(super) fn purge_request_params(arguments: &Value) -> Value {
    serde_json::json!({
        "name": "purge_saved_context",
        "arguments": arguments,
    })
}

pub(super) fn open_session_store(db_path: &str) -> SessionStore {
    SessionStore::open(&derive_session_db_path(db_path)).unwrap()
}

pub(super) fn seed_session_meta(store: &mut SessionStore, repo_root: &str) -> SessionId {
    let session_id = SessionId::derive(repo_root, "", "mcp");
    store
        .upsert_session_meta(session_id.clone(), repo_root, "mcp", None)
        .unwrap();
    session_id
}

pub(super) fn append_session_event(
    store: &mut SessionStore,
    session_id: &SessionId,
    event_type: SessionEventType,
    payload: Value,
) {
    store
        .append_event(NewSessionEvent {
            session_id: session_id.clone(),
            event_type,
            priority: 1,
            payload,
            created_at: None,
        })
        .unwrap();
}

/// Index a medium-sized artifact (above DEFAULT_SMALL_OUTPUT_BYTES) so it
/// is actually stored and chunks are written, then read it back.
pub(super) fn save_indexed_artifact(
    repo_root: &str,
    db_path: &str,
    label: &str,
    content: &str,
    session_id: Option<&str>,
) -> String {
    let mut args = serde_json::json!({
        "content": content,
        "label": label,
    });
    if let Some(sid) = session_id {
        args["session_id"] = serde_json::json!(sid);
    }
    let result =
        tool_save_context_artifact(Some(&args), repo_root, db_path, OutputFormat::Json).unwrap();
    let body: Value = serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    // Return the source_id regardless of routing (preview or pointer).
    body["source_id"].as_str().unwrap_or("").to_string()
}

/// Build a string longer than DEFAULT_SMALL_OUTPUT_BYTES (512 B) so
/// `route_output` actually indexes it.
pub(super) fn medium_content(label: &str) -> String {
    let payload = std::iter::repeat_n("safe medium artifact payload", 40)
        .collect::<Vec<_>>()
        .join(" ");
    format!("{label}: {payload}")
}

pub(super) fn large_content(label: &str) -> String {
    let payload = std::iter::repeat_n("safe large artifact payload with spacing", 180)
        .collect::<Vec<_>>()
        .join(" ");
    format!("{label}: {payload}")
}

pub(super) fn oversized_content(paragraphs: usize) -> String {
    (0..paragraphs)
            .map(|i| {
                format!(
                    "paragraph {i} carries unique oversized artifact text with several safe words here\n\n"
                )
            })
            .collect()
}

pub(super) fn medium_secret_content(secret_pair: &str, token: &str) -> String {
    let payload = std::iter::repeat_n("visible safe payload text", 50)
        .collect::<Vec<_>>()
        .join(" ");
    format!("{secret_pair} token={token} {payload}")
}
