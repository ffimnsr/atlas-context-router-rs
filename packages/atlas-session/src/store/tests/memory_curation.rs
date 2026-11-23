// ── ICM-B — memory decay, stale, prune, health, and consolidation ─────────────

use time::OffsetDateTime;

use super::*;

/// Fixed "now" for deterministic age math: 2026-06-01T00:00:00Z.
const NOW: i64 = 1_780_272_000;
/// 2026-01-01T00:00:00Z — 151 days before NOW.
const OLD: i64 = 1_767_225_600;
/// 2026-05-01T00:00:00Z — 31 days before NOW.
const RECENT: i64 = 1_777_593_600;

fn ts(seconds: i64) -> String {
    atlas_core::format_rfc3339(
        OffsetDateTime::from_unix_timestamp(seconds)
            .expect("valid unix timestamp")
            .replace_nanosecond(0)
            .expect("0 nanoseconds is always valid"),
    )
}

fn now_ts() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(NOW).expect("valid unix timestamp")
}

fn default_policy() -> MemoryDecayPolicy {
    MemoryDecayPolicy::default()
}

/// Local seed with full control over title, source id, and metadata category.
struct SeedSpec<'a> {
    at: i64,
    topic: &'a str,
    title: &'a str,
    body: &'a str,
    importance: MemoryImportance,
    source: Option<&'a str>,
    category: Option<&'a str>,
}

fn seed(store: &SessionStore, id: &str, spec: SeedSpec<'_>) -> MemoryRecord {
    let mut metadata = serde_json::json!({});
    if let Some(category) = spec.category {
        metadata["category"] = serde_json::json!(category);
    }
    let input = NewMemory {
        repo_root: "/repo".to_owned(),
        session_id: None,
        frontend: None,
        scope: MemoryScope::Project,
        topic: spec.topic.to_owned(),
        title: spec.title.to_owned(),
        body: spec.body.to_owned(),
        importance: spec.importance,
        source_id: spec.source.map(str::to_owned),
        tags: Vec::new(),
        links: Vec::new(),
        metadata,
    };
    super::markdown_memory::store_memory_at(&store.markdown, &store.conn, &input, &ts(spec.at), id)
        .unwrap()
}

// ── Decay scoring and policy ─────────────────────────────────────────────────

mod consolidate;
mod decay;
mod health;
mod prune;
