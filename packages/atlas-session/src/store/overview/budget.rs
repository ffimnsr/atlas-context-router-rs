//! ICM-G1 — central budget policy for overview bodies.
//!
//! Overview title, summary, description preview, and each list field are
//! bounded before storage. Oversized bodies are routed through the content
//! store by the shared ingest service; the record keeps only a preview plus
//! `source_id`.

use atlas_core::{AtlasError, Result};

use super::types::NewOverviewRecord;

pub const OVERVIEW_TITLE_MAX_CHARS: usize = 200;
pub const OVERVIEW_SUMMARY_MAX_CHARS: usize = 1_000;
pub const OVERVIEW_DESCRIPTION_PREVIEW_MAX_CHARS: usize = 2_000;
pub const OVERVIEW_LIST_ITEM_MAX_CHARS: usize = 500;
pub const OVERVIEW_LIST_MAX_ITEMS: usize = 50;
/// Full descriptions above this size are routed to the content store.
pub const OVERVIEW_INLINE_BODY_BUDGET_BYTES: usize = 4_096;

/// Truncation metadata recorded in `metadata_json.budget`.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OverviewBudgetMeta {
    pub title_truncated: bool,
    pub summary_truncated: bool,
    pub description_preview_truncated: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub truncated_lists: Vec<String>,
    pub omitted_list_items: usize,
    pub source_body_routed: bool,
}

pub fn truncate_chars(value: &str, max: usize) -> (String, bool) {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max).collect();
    let hit = chars.next().is_some();
    (truncated, hit)
}

fn bound_list(field: &str, values: &mut Vec<String>, meta: &mut OverviewBudgetMeta) {
    let mut truncated = false;
    let mut omitted = 0usize;
    if values.len() > OVERVIEW_LIST_MAX_ITEMS {
        omitted = values.len() - OVERVIEW_LIST_MAX_ITEMS;
        values.truncate(OVERVIEW_LIST_MAX_ITEMS);
        truncated = true;
    }
    for value in values.iter_mut() {
        let (bounded, hit) = truncate_chars(value, OVERVIEW_LIST_ITEM_MAX_CHARS);
        if hit {
            *value = bounded;
            truncated = true;
        }
    }
    if truncated {
        meta.truncated_lists.push(field.to_owned());
    }
    meta.omitted_list_items = meta.omitted_list_items.saturating_add(omitted);
}

/// Bound every inline overview field and return truncation metadata.
pub fn bound_new_overview_record(record: &mut NewOverviewRecord) -> Result<OverviewBudgetMeta> {
    validate_confidence(record.confidence)?;

    if record.summary.trim().is_empty() {
        return Err(AtlasError::Other(
            "overview record requires a non-empty summary".to_owned(),
        ));
    }

    let mut meta = OverviewBudgetMeta::default();

    let (title, title_hit) = truncate_chars(&record.title, OVERVIEW_TITLE_MAX_CHARS);
    record.title = title;
    meta.title_truncated = title_hit;

    let (summary, summary_hit) = truncate_chars(&record.summary, OVERVIEW_SUMMARY_MAX_CHARS);
    record.summary = summary;
    meta.summary_truncated = summary_hit;

    let (preview, preview_hit) = truncate_chars(
        &record.description_preview,
        OVERVIEW_DESCRIPTION_PREVIEW_MAX_CHARS,
    );
    record.description_preview = preview;
    meta.description_preview_truncated = preview_hit;

    bound_list("responsibilities", &mut record.responsibilities, &mut meta);
    bound_list("flow", &mut record.flow, &mut meta);
    bound_list("inputs", &mut record.inputs, &mut meta);
    bound_list("outputs", &mut record.outputs, &mut meta);
    bound_list("invariants", &mut record.invariants, &mut meta);
    bound_list("gotchas", &mut record.gotchas, &mut meta);
    bound_list("examples", &mut record.examples, &mut meta);
    bound_list("tags", &mut record.tags, &mut meta);

    // Trimmed empty entries are noise; keep list order otherwise.
    for values in [
        &mut record.responsibilities,
        &mut record.flow,
        &mut record.inputs,
        &mut record.outputs,
        &mut record.invariants,
        &mut record.gotchas,
        &mut record.examples,
        &mut record.tags,
    ] {
        values.retain(|value| !value.trim().is_empty());
    }

    Ok(meta)
}

pub fn validate_confidence(confidence: f64) -> Result<()> {
    if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
        return Err(AtlasError::Other(
            "overview confidence must be between 0.0 and 1.0".to_owned(),
        ));
    }
    Ok(())
}
