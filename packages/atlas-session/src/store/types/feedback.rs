use super::default_memory_metadata;
use super::*;

// ── ICM-C — feedback records (predicted vs actual corrections) ───────────────

/// Input shape for a manual feedback write shared by CLI and MCP surfaces.
///
/// `predicted` and `actual` are required; everything else is optional
/// enrichment. Call [`NewFeedback::validate`] before persisting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NewFeedback {
    pub repo_root: String,
    pub session_id: Option<String>,
    /// Tool that produced the prediction (e.g. `cli`, `mcp`).
    #[serde(default)]
    pub tool_name: String,
    /// Analysis kind the prediction came from (e.g. `dead_code`, `remove`,
    /// `safety`, `remove_dead`).
    #[serde(default)]
    pub analysis_kind: String,
    /// What the analysis predicted; required.
    pub predicted: String,
    /// What actually happened; required. `actual != predicted` marks the
    /// record as false-positive evidence for confidence adjustment.
    pub actual: String,
    /// Optional correction text explaining the right answer.
    #[serde(default)]
    pub correction: String,
    /// Symbol the prediction was about (exact qualified name).
    pub related_symbol: Option<String>,
    /// File the prediction was about (repo-relative path).
    pub related_file: Option<String>,
    /// Optional link to a saved-context artifact.
    pub source_id: Option<String>,
    #[serde(default = "default_memory_metadata")]
    pub metadata: Value,
}

impl NewFeedback {
    /// Rejects invalid feedback writes before they reach storage: both
    /// `predicted` and `actual` must be non-empty.
    pub fn validate(&self) -> atlas_core::Result<()> {
        if self.predicted.trim().is_empty() {
            return Err(AtlasError::Other(
                "feedback predicted must not be empty".to_owned(),
            ));
        }
        if self.actual.trim().is_empty() {
            return Err(AtlasError::Other(
                "feedback actual must not be empty".to_owned(),
            ));
        }
        Ok(())
    }
}

/// A stored feedback record as persisted in the `feedback_records` table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FeedbackRecord {
    pub id: String,
    pub repo_root: String,
    pub session_id: Option<String>,
    pub tool_name: String,
    pub analysis_kind: String,
    pub predicted: String,
    pub actual: String,
    pub correction: String,
    pub related_symbol: Option<String>,
    pub related_file: Option<String>,
    pub source_id: Option<String>,
    pub created_at: String,
    pub metadata: Value,
}

impl FeedbackRecord {
    /// Whether this record is false-positive evidence for confidence
    /// adjustment: the actual outcome differs from the prediction, or the
    /// metadata explicitly marks `false_positive`.
    pub fn is_false_positive_evidence(&self) -> bool {
        self.metadata.get("false_positive") == Some(&Value::Bool(true))
            || normalize_feedback_text(&self.predicted) != normalize_feedback_text(&self.actual)
    }
}

/// Collapse whitespace and lowercase for deterministic text comparison.
fn normalize_feedback_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// A feedback search hit pairing a record with its relevance score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FeedbackSearchHit {
    pub feedback: FeedbackRecord,
    /// Higher is more relevant; `0.0` for LIKE-fallback matches.
    pub relevance_score: f32,
}

/// Exact-match filters applied on top of feedback search.
#[derive(Debug, Clone, Default)]
pub struct FeedbackSearchFilter {
    pub tool_name: Option<String>,
    pub analysis_kind: Option<String>,
    pub related_symbol: Option<String>,
    pub related_file: Option<String>,
}

/// Deterministic feedback statistics (ICM-C2 `atlas feedback stats`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FeedbackStats {
    pub total_count: usize,
    /// Records carrying a correction text.
    pub correction_count: usize,
    /// Records where `actual != predicted` (or metadata marks false positive).
    pub false_positive_count: usize,
    pub by_analysis_kind: std::collections::BTreeMap<String, usize>,
    pub by_tool: std::collections::BTreeMap<String, usize>,
}
