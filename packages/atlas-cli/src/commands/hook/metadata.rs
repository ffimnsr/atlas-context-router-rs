use std::collections::BTreeSet;

use serde_json::{Value, json};

use atlas_adapters::derive_content_db_path;
use atlas_contentstore::{ContentStore, SearchFilters};
use atlas_core::model::ContextTarget;
use atlas_review::query_parser;
use atlas_session::{ResumeSnapshot, SessionId, SessionStore};

use super::payload::{collect_source_ids, extract_changed_files, extract_prompt_text};
use super::policy::{
    HookMetadataContext, HookPolicy, HookStorage, MAX_HOOK_EVENT_SCAN, MAX_HOOK_PROMPT_HITS,
    MAX_HOOK_SOURCE_HINTS, PromptRoutingMetadata,
};

pub(crate) fn build_restore_metadata(
    store: &SessionStore,
    session_id: &SessionId,
    pending_resume: bool,
) -> Value {
    match store.get_resume_snapshot(session_id) {
        Ok(snapshot) => json!({
            "pending_resume": pending_resume,
            "has_resume_snapshot": snapshot.is_some(),
            "snapshot_consumed": snapshot.as_ref().map(|row| row.consumed),
            "snapshot_event_count": snapshot.as_ref().map(|row| row.event_count),
        }),
        Err(error) => json!({
            "pending_resume": pending_resume,
            "error": error.to_string(),
        }),
    }
}

pub(crate) fn compute_prompt_routing(
    repo: &str,
    graph_db_path: &str,
    payload: &Value,
) -> Option<PromptRoutingMetadata> {
    let prompt_text = extract_prompt_text(payload)?;
    let request = query_parser::parse_query(&prompt_text);
    let query = prompt_lookup_query(&request.target, &prompt_text);
    let hits = search_saved_context_previews(repo, graph_db_path, &query);

    Some(PromptRoutingMetadata {
        prompt_excerpt: excerpt(&prompt_text, 160),
        query,
        intent: serde_json::to_value(request.intent).unwrap_or_else(|_| json!("symbol")),
        target: serde_json::to_value(request.target).unwrap_or(Value::Null),
        hits,
    })
}

pub(crate) fn push_unique_string(values: &mut Vec<String>, candidate: String, limit: usize) {
    if values.iter().any(|existing| existing == &candidate) {
        return;
    }
    values.push(candidate);
    if values.len() > limit {
        values.truncate(limit);
    }
}

pub(crate) fn push_unique_value(values: &mut Vec<Value>, candidate: Value, limit: usize) {
    if values.iter().any(|existing| existing == &candidate) {
        return;
    }
    values.push(candidate);
    if values.len() > limit {
        values.truncate(limit);
    }
}

pub(crate) fn build_hook_event_metadata(context: HookMetadataContext<'_>) -> Value {
    let mut retrieval_hints = Vec::new();
    let mut saved_artifact_refs = Vec::new();
    let storage_mode = match context.policy.storage {
        HookStorage::SessionOnly => "session_only",
        HookStorage::SessionAndContent => "session_and_content",
    };

    if let Some(source_id) = context.routed.source_id.as_ref() {
        push_unique_string(
            &mut saved_artifact_refs,
            source_id.clone(),
            MAX_HOOK_SOURCE_HINTS,
        );
        push_unique_value(
            &mut retrieval_hints,
            json!({
                "kind": "hook_payload",
                "event": context.policy.canonical_event,
                "source_id": source_id,
                "storage_kind": context.routed.storage_kind,
            }),
            MAX_HOOK_PROMPT_HITS,
        );
    }

    if context.policy.prompt_routing
        && let Some(prompt_routing) =
            compute_prompt_routing(context.repo, context.graph_db_path, context.payload)
    {
        let prompt_query = prompt_routing.query.clone();
        push_unique_value(
            &mut retrieval_hints,
            json!({
                "kind": "prompt_query",
                "query": prompt_query,
                "intent": prompt_routing.intent.clone(),
                "target": prompt_routing.target.clone(),
            }),
            MAX_HOOK_PROMPT_HITS,
        );

        for hit in prompt_routing.hits.iter().take(MAX_HOOK_SOURCE_HINTS) {
            if let Some(source_id) = hit.get("source_id").and_then(Value::as_str) {
                push_unique_string(
                    &mut saved_artifact_refs,
                    source_id.to_owned(),
                    MAX_HOOK_SOURCE_HINTS,
                );
                push_unique_value(
                    &mut retrieval_hints,
                    json!({
                        "kind": "saved_context_hit",
                        "query": prompt_routing.query,
                        "source_id": source_id,
                        "label": hit.get("label").cloned().unwrap_or(Value::Null),
                        "source_type": hit.get("source_type").cloned().unwrap_or(Value::Null),
                    }),
                    MAX_HOOK_PROMPT_HITS,
                );
            }
        }
    }

    let source_summaries = read_source_summaries(context.graph_db_path, &saved_artifact_refs);
    let freshness = build_freshness_metadata(context.policy, context.repo, context.payload);

    json!({
        "storage_mode": storage_mode,
        "restore_metadata": build_restore_metadata(
            context.store,
            context.session_id,
            context.pending_resume,
        ),
        "retrieval_hints": retrieval_hints,
        "saved_artifact_refs": saved_artifact_refs,
        "source_summaries": source_summaries,
        "freshness": freshness,
    })
}

pub(crate) fn build_freshness_metadata(policy: &HookPolicy, repo: &str, payload: &Value) -> Value {
    if !policy.freshness {
        return Value::Null;
    }

    json!({
        "status": "stale",
        "event": policy.canonical_event,
        "stale": true,
        "changed_files": extract_changed_files(repo, payload),
        "inline_content_persisted": false,
    })
}

pub(crate) fn search_saved_context_previews(
    repo: &str,
    graph_db_path: &str,
    query: &str,
) -> Vec<Value> {
    if query.trim().len() < 3 {
        return Vec::new();
    }

    let mut content_store = match ContentStore::open(&derive_content_db_path(graph_db_path)) {
        Ok(store) => store,
        Err(_) => return Vec::new(),
    };
    if content_store.migrate().is_err() {
        return Vec::new();
    }

    let filters = SearchFilters {
        repo_root: Some(repo.to_owned()),
        ..SearchFilters::default()
    };
    let chunks = match content_store.search_with_fallback(query, &filters) {
        Ok(chunks) => chunks,
        Err(_) => return Vec::new(),
    };

    let mut seen = BTreeSet::new();
    let mut previews = Vec::new();
    for chunk in chunks {
        if !seen.insert(chunk.source_id.clone()) {
            continue;
        }
        let source = content_store.get_source(&chunk.source_id).ok().flatten();
        previews.push(json!({
            "source_id": chunk.source_id,
            "label": source.as_ref().map(|row| row.label.clone()),
            "source_type": source.as_ref().map(|row| row.source_type.clone()),
            "preview": excerpt(&chunk.content, 256),
            "content_type": chunk.content_type,
        }));
        if previews.len() >= MAX_HOOK_PROMPT_HITS {
            break;
        }
    }

    previews
}

pub(crate) fn collect_context_hints(
    repo: &str,
    graph_db_path: &str,
    store: &SessionStore,
    session_id: &SessionId,
) -> Value {
    let events = match store.list_events(session_id) {
        Ok(events) => events,
        Err(_) => {
            return json!({
                "recent_files": Vec::<String>::new(),
                "recent_source_ids": Vec::<String>::new(),
                "recent_saved_context": Vec::<Value>::new(),
                "recent_prompts": Vec::<String>::new(),
            });
        }
    };

    let mut files = BTreeSet::new();
    let mut source_ids = BTreeSet::new();
    let mut prompts = Vec::new();
    let mut hook_events = Vec::new();

    for event in events.iter().rev().take(MAX_HOOK_EVENT_SCAN) {
        let Ok(payload) = serde_json::from_str::<Value>(&event.payload_json) else {
            continue;
        };

        if let Some(name) = payload.get("hook_event").and_then(Value::as_str)
            && !hook_events.iter().any(|existing| existing == name)
        {
            hook_events.push(name.to_owned());
        }
        if prompts.len() < MAX_HOOK_SOURCE_HINTS
            && let Some(prompt) = extract_prompt_text(&payload)
        {
            let sample = excerpt(&prompt, 120);
            if !prompts.iter().any(|existing| existing == &sample) {
                prompts.push(sample);
            }
        }

        collect_source_ids(&payload, &mut source_ids);
        for path in extract_changed_files(repo, &payload) {
            files.insert(path);
        }
    }

    let recent_source_ids: Vec<String> = source_ids
        .iter()
        .take(MAX_HOOK_SOURCE_HINTS)
        .cloned()
        .collect();
    let recent_saved_context = read_source_summaries(graph_db_path, &recent_source_ids);

    json!({
        "recent_files": files.into_iter().take(MAX_HOOK_EVENT_SCAN).collect::<Vec<_>>(),
        "recent_source_ids": recent_source_ids,
        "recent_saved_context": recent_saved_context,
        "recent_prompts": prompts,
        "recent_hook_events": hook_events,
    })
}

pub(crate) fn read_source_summaries(graph_db_path: &str, source_ids: &[String]) -> Vec<Value> {
    if source_ids.is_empty() {
        return Vec::new();
    }

    let mut content_store = match ContentStore::open(&derive_content_db_path(graph_db_path)) {
        Ok(store) => store,
        Err(_) => return Vec::new(),
    };
    if content_store.migrate().is_err() {
        return Vec::new();
    }

    source_ids
        .iter()
        .filter_map(|source_id| {
            let source = content_store.get_source(source_id).ok().flatten()?;
            Some(json!({
                "source_id": source.id,
                "label": source.label,
                "source_type": source.source_type,
                "created_at": source.created_at,
            }))
        })
        .collect()
}

pub(crate) fn snapshot_to_value(snapshot: &ResumeSnapshot) -> Value {
    json!({
        "event_count": snapshot.event_count,
        "consumed": snapshot.consumed,
        "created_at": snapshot.created_at,
        "updated_at": snapshot.updated_at,
        "snapshot": parse_json_or_string(&snapshot.snapshot),
    })
}

pub(crate) fn parse_json_or_string(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_owned()))
}

pub(crate) fn prompt_lookup_query(target: &ContextTarget, prompt_text: &str) -> String {
    match target {
        ContextTarget::QualifiedName { qname } => qname.clone(),
        ContextTarget::SymbolName { name } => name.clone(),
        ContextTarget::FilePath { path } => path.clone(),
        ContextTarget::ChangedFiles { paths } => paths
            .first()
            .cloned()
            .unwrap_or_else(|| prompt_text.to_owned()),
        ContextTarget::ChangedSymbols { qnames } => qnames
            .first()
            .cloned()
            .unwrap_or_else(|| prompt_text.to_owned()),
        ContextTarget::EdgeQuerySeed { source_qname, .. } => source_qname.clone(),
    }
}

pub(crate) fn excerpt(text: &str, max_chars: usize) -> String {
    let truncated: String = text.chars().take(max_chars).collect();
    if truncated.chars().count() == text.chars().count() {
        truncated
    } else {
        format!("{truncated}...")
    }
}
