use std::collections::BTreeSet;

use atlas_core::AtlasError;
use atlas_core::Result;
use atlas_store_markdown::MarkdownStore;
use rusqlite::Connection;
use serde_json::Value as JsonValue;

use super::super::memory::format_memory_now;
use super::MemoryConsolidationGroup;
use super::MemoryConsolidationPlan;
use super::MemoryImportance;
use super::MemoryLink;
use super::MemoryListFilter;
use super::MemoryRecord;
use super::NewMemory;
use super::RELATION_SUPERSEDES;
use super::memory_consolidate::OverflowSink;
use super::read::list_memories;
use super::write::store_memory;

// ── Consolidation ─────────────────────────────────────────────────────────────

/// Deterministic consolidation planner/applier. Apply mode writes one new
/// consolidated file whose `supersedes` links replace the members; member
/// files are never rewritten.
pub(crate) fn consolidation_plan(
    store: &MarkdownStore,
    conn: &Connection,
    repo_root: &str,
    filter: &MemoryListFilter,
    dry_run: bool,
    mut overflow: Option<&mut OverflowSink<'_>>,
) -> Result<MemoryConsolidationPlan> {
    let rows = list_memories(store, conn, repo_root, filter)?;

    #[derive(Clone)]
    struct MemberKeys {
        title: Option<String>,
        body: String,
        source: Option<String>,
        category: Option<String>,
    }

    struct Group {
        members: Vec<MemoryRecord>,
        keys: Vec<MemberKeys>,
    }

    let mut groups: Vec<Group> = Vec::new();
    for row in rows {
        let keys = MemberKeys {
            title: (!row.title.trim().is_empty()).then(|| normalize_text(&row.title)),
            body: normalize_text(&row.body),
            source: row.source_id.clone(),
            category: memory_category(&row.metadata),
        };
        let matches = |other: &MemberKeys| -> bool {
            (keys.title.is_some() && other.title.is_some() && keys.title == other.title)
                || keys.body == other.body
                || (keys.source.is_some() && keys.source == other.source)
                || (keys.category.is_some() && keys.category == other.category)
        };
        let mut matched: Vec<usize> = groups
            .iter()
            .enumerate()
            .filter(|(_, group)| {
                group.members[0].topic == row.topic && group.keys.iter().any(&matches)
            })
            .map(|(index, _)| index)
            .collect();
        if matched.is_empty() {
            groups.push(Group {
                members: vec![row],
                keys: vec![keys],
            });
        } else {
            matched.sort_unstable();
            let first = matched.remove(0);
            for extra in matched.into_iter().rev() {
                let merged = groups.remove(extra);
                groups[first].members.extend(merged.members);
                groups[first].keys.extend(merged.keys);
            }
            groups[first].members.push(row);
            groups[first].keys.push(keys);
        }
    }

    let _now = format_memory_now();
    let mut plan_groups = Vec::new();
    let mut kept_ids = Vec::new();
    let mut merged_ids = Vec::new();

    for group in groups.into_iter().filter(|group| group.members.len() >= 2) {
        let members = group.members;
        let kept = &members[0];
        let group_key = consolidation_group_key(&members);
        let source_ids = members
            .iter()
            .filter_map(|member| member.source_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let merged = members[1..]
            .iter()
            .map(|member| member.id.clone())
            .collect::<Vec<_>>();

        let synthesis = super::memory_consolidate::synthesize_consolidated_body(&members);
        let member_ids = members
            .iter()
            .map(|member| member.id.clone())
            .collect::<Vec<_>>();
        let planned_overflow_id = synthesis
            .overflow
            .then(|| super::memory_consolidate::overflow_source_id(repo_root, &member_ids));
        let unrouted_overflow = synthesis.overflow && !dry_run && overflow.is_none();
        let overflow_id = if unrouted_overflow {
            None
        } else {
            planned_overflow_id
        };
        let row_body = if unrouted_overflow {
            synthesis
                .full_body
                .clone()
                .unwrap_or_else(|| synthesis.body.clone())
        } else {
            synthesis.body.clone()
        };

        let importance = members
            .iter()
            .map(|member| importance_rank(member.importance))
            .min()
            .map(importance_from_rank)
            .unwrap_or(kept.importance);
        let mut metadata = kept.metadata.clone();
        metadata["consolidated"] = serde_json::json!(true);
        metadata["merged_ids"] = serde_json::to_value(&merged).unwrap_or_default();
        metadata["merged_source_ids"] = serde_json::to_value(&source_ids).unwrap_or_default();
        metadata["merged_unit_count"] = serde_json::json!(synthesis.distinct_units);
        metadata["overflow_source_id"] = serde_json::to_value(&overflow_id).unwrap_or_default();
        let metadata_json = serde_json::to_string(&metadata).map_err(|error| {
            AtlasError::Other(format!("serialize consolidated metadata: {error}"))
        })?;

        let consolidated_id = if dry_run {
            None
        } else {
            if let (Some(full_body), Some(sink)) =
                (synthesis.full_body.as_deref(), overflow.as_deref_mut())
            {
                super::memory_consolidate::route_overflow_body(
                    sink,
                    repo_root,
                    &member_ids,
                    kept.session_id.as_deref(),
                    full_body,
                )?;
            }
            let input = NewMemory {
                repo_root: repo_root.to_owned(),
                session_id: kept.session_id.clone(),
                frontend: kept.frontend.clone(),
                scope: kept.scope,
                topic: kept.topic.clone(),
                title: kept.title.clone(),
                body: row_body.clone(),
                importance,
                source_id: kept.source_id.clone(),
                tags: kept.tags.clone(),
                metadata,
                links: member_ids
                    .iter()
                    .map(|id| MemoryLink {
                        id: id.clone(),
                        relation: RELATION_SUPERSEDES.to_owned(),
                    })
                    .collect(),
            };
            let record = store_memory(store, conn, &input)?;
            Some(record.id)
        };

        kept_ids.push(kept.id.clone());
        merged_ids.extend(merged.iter().cloned());
        plan_groups.push(MemoryConsolidationGroup {
            topic: kept.topic.clone(),
            group_key,
            kept_memory_id: kept.id.clone(),
            merged_memory_ids: merged,
            source_ids: source_ids.clone(),
            body: row_body,
            metadata_json,
            merged_source_ids: source_ids,
            overflow_source_id: overflow_id,
            consolidated_id,
        });
    }

    Ok(MemoryConsolidationPlan {
        dry_run,
        groups: plan_groups,
        kept_ids,
        merged_ids,
    })
}

/// Deterministic grouping reason, in fixed priority order.
fn consolidation_group_key(members: &[MemoryRecord]) -> String {
    for (a, b) in pairs(members) {
        if !a.title.trim().is_empty()
            && !b.title.trim().is_empty()
            && normalize_text(&a.title) == normalize_text(&b.title)
        {
            return "same_title".to_owned();
        }
    }
    for (a, b) in pairs(members) {
        if normalize_text(&a.body) == normalize_text(&b.body) {
            return "same_body".to_owned();
        }
    }
    for (a, b) in pairs(members) {
        if a.source_id.is_some() && a.source_id == b.source_id {
            return "same_source".to_owned();
        }
    }
    for (a, b) in pairs(members) {
        if memory_category(&a.metadata).is_some()
            && memory_category(&a.metadata) == memory_category(&b.metadata)
        {
            return "same_category".to_owned();
        }
    }
    "same_body".to_owned()
}

fn pairs<T>(items: &[T]) -> impl Iterator<Item = (&T, &T)> {
    (0..items.len()).flat_map(move |i| (i + 1..items.len()).map(move |j| (&items[i], &items[j])))
}

pub(crate) fn normalize_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn memory_category(metadata: &JsonValue) -> Option<String> {
    metadata
        .get("category")
        .and_then(JsonValue::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn importance_rank(importance: MemoryImportance) -> u8 {
    match importance {
        MemoryImportance::Critical => 0,
        MemoryImportance::High => 1,
        MemoryImportance::Normal => 2,
        MemoryImportance::Low => 3,
    }
}

fn importance_from_rank(rank: u8) -> MemoryImportance {
    match rank {
        0 => MemoryImportance::Critical,
        1 => MemoryImportance::High,
        2 => MemoryImportance::Normal,
        _ => MemoryImportance::Low,
    }
}
