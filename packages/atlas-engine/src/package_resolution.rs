//! Weak same-package call-target resolution.
//!
//! Extracted from `call_resolution.rs` so that file stays below the 1,000 LOC
//! threshold and reachability guard work can build on the same helpers without
//! duplicating heuristics.

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use atlas_core::{Node, NodeKind};
use atlas_store_sqlite::Store;
use camino::Utf8Path;

/// Call-site metadata captured by the parser in `Edge.extra_json`.
#[derive(Debug, Clone)]
pub(crate) struct CallMeta {
    pub(crate) callee_name: String,
    pub(crate) receiver_text: Option<String>,
    pub(crate) receiver_type: Option<String>,
}

/// Import binding captured from an `Import` node.
#[derive(Debug, Clone)]
pub(crate) struct ImportBinding {
    pub(crate) source: String,
    pub(crate) local: String,
    pub(crate) imported: Option<String>,
    pub(crate) kind: String,
    pub(crate) relative_level: usize,
}

pub(crate) fn call_meta(edge: &atlas_core::Edge) -> Option<CallMeta> {
    let extra = edge.extra_json.as_object()?;
    let callee_name = extra.get("callee_name")?.as_str()?.to_owned();
    let receiver_text = extra
        .get("receiver_text")
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .filter(|value| !value.is_empty());
    let receiver_type = extra
        .get("receiver_type")
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .filter(|value| !value.is_empty());
    Some(CallMeta {
        callee_name,
        receiver_text,
        receiver_type,
    })
}

pub(crate) fn collect_import_bindings(nodes: &[Node]) -> Vec<ImportBinding> {
    let mut bindings = Vec::new();
    for node in nodes {
        if node.kind != NodeKind::Import {
            continue;
        }
        let Some(extra) = node.extra_json.as_object() else {
            continue;
        };
        let Some(source) = extra.get("source").and_then(|value| value.as_str()) else {
            continue;
        };
        let Some(items) = extra.get("bindings").and_then(|value| value.as_array()) else {
            continue;
        };
        for item in items {
            let Some(item_obj) = item.as_object() else {
                continue;
            };
            let Some(local) = item_obj.get("local").and_then(|value| value.as_str()) else {
                continue;
            };
            let imported = item_obj
                .get("imported")
                .and_then(|value| value.as_str())
                .map(str::to_owned);
            let kind = item_obj
                .get("kind")
                .and_then(|value| value.as_str())
                .unwrap_or("module")
                .to_owned();
            let relative_level = extra
                .get("relative_level")
                .and_then(|value| value.as_u64())
                .unwrap_or(0) as usize;
            bindings.push(ImportBinding {
                source: source.to_owned(),
                local: local.to_owned(),
                imported,
                kind,
                relative_level,
            });
        }
    }
    bindings
}

pub(crate) fn node_repo_id(node: &Node) -> Option<&str> {
    node.extra_json
        .as_object()
        .and_then(|extra| extra.get("repo_id"))
        .and_then(|value| value.as_str())
}

pub(crate) fn callable_candidates(
    store: &Store,
    language: &str,
    callee_name: &str,
    candidate_cache: &mut HashMap<(String, String), Vec<Node>>,
    allowed_repo_ids: &HashSet<String>,
) -> Result<Vec<Node>> {
    let key = (language.to_owned(), callee_name.to_owned());
    if let Some(nodes) = candidate_cache.get(&key) {
        return Ok(nodes.clone());
    }
    let nodes = store.callable_nodes_by_name(language, callee_name)?;
    candidate_cache.insert(key, nodes.clone());
    Ok(nodes
        .into_iter()
        .filter(|node| {
            node_repo_id(node)
                .map(|repo_id| allowed_repo_ids.contains(repo_id))
                .unwrap_or(true)
        })
        .collect())
}

/// Inputs for weak same-package resolution.
pub(crate) struct SamePackageContext<'a> {
    pub(crate) store: &'a Store,
    pub(crate) language: &'a str,
    pub(crate) path: &'a str,
    pub(crate) source_repo_id: &'a str,
    pub(crate) allowed_repo_ids: &'a HashSet<String>,
}

/// Resolve a call from a weak same-package fallback.
pub(crate) fn resolve_same_package_target(
    ctx: &SamePackageContext<'_>,
    meta: &CallMeta,
    candidate_cache: &mut HashMap<(String, String), Vec<Node>>,
    owner_cache: &mut HashMap<String, Option<String>>,
) -> Option<(Node, &'static str, f32)> {
    let mut candidates = callable_candidates(
        ctx.store,
        ctx.language,
        &meta.callee_name,
        candidate_cache,
        ctx.allowed_repo_ids,
    )
    .ok()?;
    if let Some(receiver_hint) = meta
        .receiver_type
        .as_deref()
        .or_else(|| meta.receiver_text.as_deref().and_then(receiver_type_hint))
    {
        let receiver_matches: Vec<Node> = candidates
            .iter()
            .filter(|node| matches_receiver_hint(node, receiver_hint))
            .cloned()
            .collect();
        if !receiver_matches.is_empty() {
            candidates = receiver_matches;
        }
    }
    let current_owner = cached_owner_id(ctx.store, ctx.source_repo_id, ctx.path, owner_cache);

    if let Some(current_owner) = current_owner {
        let same_owner_matches: Vec<Node> = candidates
            .iter()
            .filter(|node| node.file_path != ctx.path)
            .filter(|node| {
                cached_owner_id(ctx.store, ctx.source_repo_id, &node.file_path, owner_cache)
                    .as_deref()
                    == Some(current_owner.as_str())
            })
            .cloned()
            .collect();
        if let Some(node) = unique_node(same_owner_matches.clone()) {
            return Some((node, "same_package", 0.65));
        }

        let current_dir = Utf8Path::new(ctx.path)
            .parent()
            .unwrap_or_else(|| Utf8Path::new(""));
        let same_dir_matches: Vec<Node> = same_owner_matches
            .into_iter()
            .filter(|node| same_dir(current_dir, &node.file_path))
            .collect();
        return unique_node(same_dir_matches).map(|node| (node, "same_package", 0.65));
    }

    let current_dir = Utf8Path::new(ctx.path)
        .parent()
        .unwrap_or_else(|| Utf8Path::new(""));
    let matches: Vec<Node> = candidates
        .into_iter()
        .filter(|node| node.file_path != ctx.path)
        .filter(|node| same_dir(current_dir, &node.file_path))
        .collect();
    unique_node(matches).map(|node| (node, "same_package", 0.65))
}

pub(crate) fn cached_owner_id(
    store: &Store,
    source_repo_id: &str,
    path: &str,
    owner_cache: &mut HashMap<String, Option<String>>,
) -> Option<String> {
    if let Some(owner_id) = owner_cache.get(path) {
        return owner_id.clone();
    }
    let owner_id = store
        .file_owner_id_for_repo(source_repo_id, path)
        .ok()
        .flatten();
    owner_cache.insert(path.to_owned(), owner_id.clone());
    owner_id
}

fn receiver_type_hint(receiver: &str) -> Option<&str> {
    let receiver = receiver.trim();
    if receiver.is_empty() {
        return None;
    }

    let receiver = receiver
        .split('<')
        .next()
        .unwrap_or(receiver)
        .trim_matches('&')
        .trim();

    receiver
        .rsplit("::")
        .next()
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
}

fn matches_receiver_hint(node: &Node, receiver_hint: &str) -> bool {
    method_receiver_name(node).is_some_and(|candidate| candidate == receiver_hint)
}

fn method_receiver_name(node: &Node) -> Option<&str> {
    node.parent_name
        .as_deref()
        .and_then(|parent| parent.contains("::impl::").then_some(parent))
        .and_then(|parent| parent.rsplit("::impl::").next())
        .and_then(|tail| tail.rsplit("::").next())
        .or_else(|| {
            let (_, tail) = node.qualified_name.split_once("::method::")?;
            tail.rsplit_once("::")
                .map(|(receiver, _)| receiver)
                .or_else(|| tail.rsplit_once('.').map(|(receiver, _)| receiver))
        })
}

fn same_dir(current_dir: &Utf8Path, candidate_path: &str) -> bool {
    Utf8Path::new(candidate_path)
        .parent()
        .map(|parent| parent == current_dir)
        .unwrap_or(false)
}

pub(crate) fn unique_node(mut nodes: Vec<Node>) -> Option<Node> {
    if nodes.len() != 1 {
        return None;
    }
    nodes.pop()
}
