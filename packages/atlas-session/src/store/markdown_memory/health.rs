use std::collections::BTreeMap;
use std::collections::BTreeSet;

use atlas_core::Result;
use atlas_store_markdown::MarkdownStore;
use rusqlite::Connection;
use time::OffsetDateTime;

use super::super::memory::age_days;
use super::MemoryDecayPolicy;
use super::MemoryHealthCategory;
use super::MemoryHealthFinding;
use super::MemoryHealthReport;
use super::MemoryImportance;
use super::MemoryListFilter;
use super::NOISY_TOPIC_ENTRIES;
use super::OVERSIZED_BODY_CHARS;
use super::consolidate::normalize_text;
use super::read::matches_filter;
use super::scan::scan_lenient;

/// Deterministic health report, including malformed files and dangling links.
pub(crate) fn health_report(
    store: &MarkdownStore,
    _conn: &Connection,
    repo_root: &str,
    filter: &MemoryListFilter,
    policy: &MemoryDecayPolicy,
    now: OffsetDateTime,
    source_exists: &dyn Fn(&str) -> bool,
) -> Result<MemoryHealthReport> {
    let (all_records, issues) = scan_lenient(store, repo_root);
    let rows = all_records
        .iter()
        .filter(|record| matches_filter(record, filter))
        .cloned()
        .collect::<Vec<_>>();
    let known_ids = all_records
        .iter()
        .map(|record| record.id.clone())
        .collect::<BTreeSet<_>>();
    let mut findings = Vec::new();
    let mut by_category: BTreeMap<String, usize> = BTreeMap::new();
    let mut count = |category: MemoryHealthCategory| {
        *by_category.entry(category.as_str().to_owned()).or_insert(0) += 1;
    };

    for issue in issues {
        count(MemoryHealthCategory::Orphaned);
        findings.push(MemoryHealthFinding {
            category: MemoryHealthCategory::Orphaned,
            kind: "malformed_frontmatter".to_owned(),
            memory_id: None,
            topic: None,
            detail: issue.clone(),
            suggestion: "repair or delete the malformed memory file".to_owned(),
            command: "atlas doctor".to_owned(),
        });
    }

    for record in &rows {
        for link in &record.links {
            if !known_ids.contains(&link.id) {
                count(MemoryHealthCategory::Orphaned);
                findings.push(MemoryHealthFinding {
                    category: MemoryHealthCategory::Orphaned,
                    kind: "dangling_link".to_owned(),
                    memory_id: Some(record.id.clone()),
                    topic: (!record.topic.is_empty()).then(|| record.topic.clone()),
                    detail: format!(
                        "memory {} links ({}) to missing memory {}",
                        record.id, link.relation, link.id
                    ),
                    suggestion: "re-link or remove the dangling memory link".to_owned(),
                    command: format!("atlas memory link {} <target> {}", record.id, link.relation),
                });
            }
        }
    }

    for (index, row) in rows.iter().enumerate() {
        let age = age_days(&row.updated_at, now);
        let protected = policy.is_protected(row.importance);
        let stale = !protected && policy.score(row.importance, age) >= 1.0;
        if stale {
            count(MemoryHealthCategory::Stale);
            findings.push(MemoryHealthFinding {
                category: MemoryHealthCategory::Stale,
                kind: "stale_memory".to_owned(),
                memory_id: Some(row.id.clone()),
                topic: (!row.topic.is_empty()).then(|| row.topic.clone()),
                detail: format!(
                    "memory {} in topic '{}' is past its retention window",
                    row.id, row.topic
                ),
                suggestion: "refresh the fact or prune the row".to_owned(),
                command: prune_command(&row.topic),
            });
            continue;
        }

        let duplicated = rows[..index].iter().any(|other| {
            other.topic == row.topic
                && ((!other.title.trim().is_empty()
                    && normalize_text(&other.title) == normalize_text(&row.title))
                    || normalize_text(&other.body) == normalize_text(&row.body))
        });
        if duplicated {
            count(MemoryHealthCategory::Duplicated);
            findings.push(MemoryHealthFinding {
                category: MemoryHealthCategory::Duplicated,
                kind: "duplicated_memory".to_owned(),
                memory_id: Some(row.id.clone()),
                topic: (!row.topic.is_empty()).then(|| row.topic.clone()),
                detail: format!(
                    "memory {} repeats an earlier memory in topic '{}'",
                    row.id, row.topic
                ),
                suggestion: "merge the duplicates into one fact".to_owned(),
                command: consolidate_command(&row.topic),
            });
            continue;
        }

        let orphaned = row
            .source_id
            .as_deref()
            .is_some_and(|source_id| !source_exists(source_id));
        if orphaned {
            count(MemoryHealthCategory::Orphaned);
            findings.push(MemoryHealthFinding {
                category: MemoryHealthCategory::Orphaned,
                kind: "orphaned_source".to_owned(),
                memory_id: Some(row.id.clone()),
                topic: (!row.topic.is_empty()).then(|| row.topic.clone()),
                detail: format!(
                    "memory {} references a missing saved-context artifact",
                    row.id
                ),
                suggestion: "re-link the source artifact or delete the memory".to_owned(),
                command: format!("atlas memory delete {} --dry-run", row.id),
            });
            continue;
        }

        if row.body.chars().count() > OVERSIZED_BODY_CHARS {
            count(MemoryHealthCategory::Oversized);
            findings.push(MemoryHealthFinding {
                category: MemoryHealthCategory::Oversized,
                kind: "oversized_memory".to_owned(),
                memory_id: Some(row.id.clone()),
                topic: (!row.topic.is_empty()).then(|| row.topic.clone()),
                detail: format!(
                    "memory {} exceeds {} characters",
                    row.id, OVERSIZED_BODY_CHARS
                ),
                suggestion: "split the body into one-fact memories".to_owned(),
                command: format!("atlas memory store \"<fact>\" --topic {}", row.topic),
            });
        }
    }

    let mut topic_stats: BTreeMap<&str, (usize, bool)> = BTreeMap::new();
    for row in &rows {
        if row.topic.is_empty() {
            continue;
        }
        let entry = topic_stats.entry(row.topic.as_str()).or_insert((0, false));
        entry.0 += 1;
        entry.1 |= row.importance == MemoryImportance::Critical;
    }
    for (topic, (entries, has_critical)) in topic_stats {
        if entries > NOISY_TOPIC_ENTRIES {
            count(MemoryHealthCategory::Noisy);
            findings.push(MemoryHealthFinding {
                category: MemoryHealthCategory::Noisy,
                kind: "noisy_topic".to_owned(),
                memory_id: None,
                topic: Some(topic.to_owned()),
                detail: format!(
                    "topic '{topic}' has {entries} memories (noisy above {NOISY_TOPIC_ENTRIES})"
                ),
                suggestion: "consolidate the topic into fewer facts".to_owned(),
                command: consolidate_command(topic),
            });
        }
        if !has_critical {
            count(MemoryHealthCategory::Noisy);
            findings.push(MemoryHealthFinding {
                category: MemoryHealthCategory::Noisy,
                kind: "topic_without_critical".to_owned(),
                memory_id: None,
                topic: Some(topic.to_owned()),
                detail: format!(
                    "topic '{topic}' has {entries} memories but no critical decision memory"
                ),
                suggestion: "record the key decision with critical importance".to_owned(),
                command: format!(
                    "atlas memory store \"<decision>\" --topic {topic} --importance critical"
                ),
            });
        }
    }

    Ok(MemoryHealthReport {
        total_memories: rows.len(),
        findings,
        by_category,
    })
}

fn prune_command(topic: &str) -> String {
    if topic.is_empty() {
        "atlas memory prune --dry-run".to_owned()
    } else {
        format!("atlas memory prune --topic {topic} --dry-run")
    }
}

fn consolidate_command(topic: &str) -> String {
    if topic.is_empty() {
        "atlas memory consolidate --dry-run".to_owned()
    } else {
        format!("atlas memory consolidate --topic {topic} --dry-run")
    }
}
