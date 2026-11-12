//! ICM-G / VikingRAG — progressive L0/L1/L2 views over `overview_records`.
//!
//! Levels are pure projections of one persisted [`OverviewRecord`]; no second
//! storage layer exists. Each level carries `freshness` so `stale` and
//! `orphaned` records can never be mistaken for fresh evidence.

use serde::{Deserialize, Serialize};

use atlas_core::{AtlasError, Result};

use super::types::{OverviewFreshness, OverviewRecord};

/// Maximum characters for one rendered L0 abstract.
pub const OVERVIEW_L0_MAX_CHARS: usize = 600;
/// Maximum characters for one rendered L1 overview.
pub const OVERVIEW_L1_MAX_CHARS: usize = 4_000;
/// Maximum characters for one rendered L2 detail view.
pub const OVERVIEW_L2_MAX_CHARS: usize = 16_000;

/// Progressive disclosure level for an overview subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OverviewLevel {
    L0,
    #[default]
    L1,
    L2,
}

impl OverviewLevel {
    pub const ALL: [Self; 3] = [Self::L0, Self::L1, Self::L2];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::L0 => "l0",
            Self::L1 => "l1",
            Self::L2 => "l2",
        }
    }

    /// Documented character budget for this level.
    pub fn max_chars(self) -> usize {
        match self {
            Self::L0 => OVERVIEW_L0_MAX_CHARS,
            Self::L1 => OVERVIEW_L1_MAX_CHARS,
            Self::L2 => OVERVIEW_L2_MAX_CHARS,
        }
    }
}

impl std::fmt::Display for OverviewLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for OverviewLevel {
    type Err = AtlasError;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "l0" => Ok(Self::L0),
            "l1" => Ok(Self::L1),
            "l2" => Ok(Self::L2),
            other => Err(AtlasError::Other(format!(
                "unknown overview level: {other}; expected l0, l1, or l2"
            ))),
        }
    }
}

/// One rendered progressive-disclosure projection of an overview record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverviewLevelView {
    pub level: OverviewLevel,
    /// Freshness is always present so callers can branch on staleness.
    pub freshness: OverviewFreshness,
    pub title: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub responsibilities: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outputs: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invariants: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gotchas: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_start: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_end: Option<u32>,
    /// L2 hint telling the caller how to fetch the bounded source excerpt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_hint: Option<String>,
    /// `true` when the level budget forced field truncation.
    #[serde(default)]
    pub truncated: bool,
}

impl OverviewLevelView {
    /// Serialized character length used to enforce the per-level budget.
    pub fn rendered_chars(&self) -> usize {
        serde_json::to_string(self)
            .map(|rendered| rendered.chars().count())
            .unwrap_or(0)
    }
}

/// Render one level view of `record`.
///
/// L0 returns title + summary. L1 adds the structured overview sections.
/// L2 adds record identity plus a bounded source-excerpt hint.
pub fn render_level(record: &OverviewRecord, level: OverviewLevel) -> OverviewLevelView {
    let mut view = OverviewLevelView {
        level,
        freshness: record.freshness,
        title: record.title.clone(),
        summary: record.summary.clone(),
        description_preview: None,
        responsibilities: None,
        flow: None,
        inputs: None,
        outputs: None,
        invariants: None,
        gotchas: None,
        source_id: None,
        canonical_file_path: None,
        line_start: None,
        line_end: None,
        read_hint: None,
        truncated: false,
    };

    if level >= OverviewLevel::L1 {
        view.description_preview = Some(record.description_preview.clone());
        view.responsibilities = Some(record.responsibilities.clone());
        view.flow = Some(record.flow.clone());
        view.inputs = Some(record.inputs.clone());
        view.outputs = Some(record.outputs.clone());
        view.invariants = Some(record.invariants.clone());
        view.gotchas = Some(record.gotchas.clone());
    }

    if level == OverviewLevel::L2 {
        view.source_id = record.source_id.clone();
        view.canonical_file_path = record.canonical_file_path.clone();
        view.line_start = record.line_start;
        view.line_end = record.line_end;
        view.read_hint = Some(read_hint(record));
    }

    let budget = level.max_chars();
    while view.rendered_chars() > budget {
        if !shrink_once(&mut view) {
            break;
        }
        view.truncated = true;
    }
    view
}

fn read_hint(record: &OverviewRecord) -> String {
    match record.canonical_file_path.as_deref() {
        Some(path) => match (record.line_start, record.line_end) {
            (Some(start), Some(end)) => {
                format!(
                    "read_file_excerpt {{ file: \"{path}\", start_line: {start}, end_line: {end} }}"
                )
            }
            _ => format!("read_file_excerpt {{ file: \"{path}\" }}"),
        },
        None => match record.source_id.as_deref() {
            Some(source_id) => format!("read_saved_context {{ source_id: \"{source_id}\" }}"),
            None => "no source span recorded for this overview".to_owned(),
        },
    }
}

/// Remove or shrink the lowest-value field currently present.
///
/// Returns `false` when nothing else can be shrunk.
fn shrink_once(view: &mut OverviewLevelView) -> bool {
    let lists = [
        &mut view.gotchas,
        &mut view.invariants,
        &mut view.outputs,
        &mut view.inputs,
        &mut view.flow,
        &mut view.responsibilities,
    ];
    for list in lists {
        if let Some(values) = list.as_mut()
            && !values.is_empty()
        {
            values.pop();
            return true;
        }
    }

    if let Some(preview) = view.description_preview.as_mut()
        && !preview.is_empty()
    {
        let keep = preview.chars().count().saturating_sub(120);
        *preview = preview.chars().take(keep).collect();
        return true;
    }

    if !view.summary.is_empty() {
        let keep = view.summary.chars().count().saturating_sub(120);
        view.summary = view.summary.chars().take(keep).collect();
        return true;
    }

    if !view.title.is_empty() {
        let keep = view.title.chars().count().saturating_sub(40);
        view.title = view.title.chars().take(keep).collect();
        return true;
    }

    false
}

impl PartialOrd for OverviewLevel {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OverviewLevel {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rank().cmp(&other.rank())
    }
}

impl OverviewLevel {
    fn rank(self) -> u8 {
        match self {
            Self::L0 => 0,
            Self::L1 => 1,
            Self::L2 => 2,
        }
    }
}
