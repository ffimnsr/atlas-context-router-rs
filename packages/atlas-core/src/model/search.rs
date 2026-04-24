use serde::{Deserialize, Serialize};
use serde_json::json;

use super::graph::Node;

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalMode {
    Fts5,
    RegexStructuralScan,
    Vector,
    Hybrid,
    GraphExpand,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchMatchedField {
    Name,
    QualifiedName,
    FilePath,
    Content,
    Embedding,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FuzzyCorrectionEvidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corrected_term: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_distance: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fuzzy_threshold: Option<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphExpansionEvidence {
    pub hop_distance: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed_qualified_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HybridRankingSource {
    Fts5,
    Vector,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HybridRankContribution {
    pub source: HybridRankingSource,
    pub rank: u32,
    pub score_contribution: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HybridRrfEvidence {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<HybridRankContribution>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RankingEvidence {
    pub base_mode: RetrievalMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_score: Option<f64>,
    pub final_score: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matched_fields: Vec<SearchMatchedField>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub exact_name_match: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub exact_qualified_name_match: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub prefix_match: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fuzzy: Option<FuzzyCorrectionEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind_boost: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_exported_boost: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub same_directory_boost: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub same_language_boost: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recent_file_boost: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed_file_boost: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_expansion: Option<GraphExpansionEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hybrid_rrf: Option<HybridRrfEvidence>,
}

pub type ScoreEvidence = RankingEvidence;

impl RankingEvidence {
    pub fn new(base_mode: RetrievalMode, final_score: f64) -> Self {
        Self {
            base_mode,
            raw_score: None,
            final_score,
            matched_fields: Vec::new(),
            exact_name_match: false,
            exact_qualified_name_match: false,
            prefix_match: false,
            fuzzy: None,
            kind_boost: None,
            public_exported_boost: None,
            same_directory_boost: None,
            same_language_boost: None,
            recent_file_boost: None,
            changed_file_boost: None,
            graph_expansion: None,
            hybrid_rrf: None,
        }
    }

    pub fn with_raw_score(mut self, raw_score: f64) -> Self {
        self.raw_score = Some(raw_score);
        self
    }

    pub fn with_matched_field(mut self, field: SearchMatchedField) -> Self {
        self.add_matched_field(field);
        self
    }

    pub fn add_matched_field(&mut self, field: SearchMatchedField) {
        if !self.matched_fields.contains(&field) {
            self.matched_fields.push(field);
        }
    }

    pub fn merge_from(&mut self, other: &Self) {
        for field in &other.matched_fields {
            self.add_matched_field(field.clone());
        }
        self.exact_name_match |= other.exact_name_match;
        self.exact_qualified_name_match |= other.exact_qualified_name_match;
        self.prefix_match |= other.prefix_match;
        if self.raw_score.is_none() {
            self.raw_score = other.raw_score;
        }
        self.fuzzy = match (&self.fuzzy, &other.fuzzy) {
            (Some(existing), Some(incoming)) => Some(choose_fuzzy(existing, incoming)),
            (None, Some(incoming)) => Some(incoming.clone()),
            (Some(existing), None) => Some(existing.clone()),
            (None, None) => None,
        };
        merge_option_f64(&mut self.kind_boost, other.kind_boost);
        merge_option_f64(&mut self.public_exported_boost, other.public_exported_boost);
        merge_option_f64(&mut self.same_directory_boost, other.same_directory_boost);
        merge_option_f64(&mut self.same_language_boost, other.same_language_boost);
        merge_option_f64(&mut self.recent_file_boost, other.recent_file_boost);
        merge_option_f64(&mut self.changed_file_boost, other.changed_file_boost);
        self.graph_expansion = match (&self.graph_expansion, &other.graph_expansion) {
            (Some(existing), Some(incoming)) => Some(choose_graph_expansion(existing, incoming)),
            (None, Some(incoming)) => Some(incoming.clone()),
            (Some(existing), None) => Some(existing.clone()),
            (None, None) => None,
        };
        match (&mut self.hybrid_rrf, &other.hybrid_rrf) {
            (Some(existing), Some(incoming)) => {
                for source in &incoming.sources {
                    if !existing.sources.iter().any(|present| {
                        present.source == source.source
                            && present.rank == source.rank
                            && (present.score_contribution - source.score_contribution).abs()
                                < f64::EPSILON
                    }) {
                        existing.sources.push(source.clone());
                    }
                }
            }
            (None, Some(incoming)) => self.hybrid_rrf = Some(incoming.clone()),
            _ => {}
        }
    }
}

pub fn ranking_evidence_legend() -> serde_json::Value {
    json!({
        "base_mode": "Base retrieval path that produced initial score: fts5, regex_structural_scan, vector, hybrid, or graph_expand.",
        "raw_score": "Score before later ranking boosts or merges when available.",
        "final_score": "Final score after boosts, graph expansion, or hybrid fusion.",
        "matched_fields": "Fields that directly matched query evidence such as name, qualified_name, file_path, content, or embedding.",
        "exact_name_match": "Result name exactly matched query text.",
        "exact_qualified_name_match": "Result qualified_name exactly matched query text.",
        "prefix_match": "Result name matched query text as prefix.",
        "fuzzy": "Fuzzy correction metadata: corrected_term, edit_distance, and fuzzy_threshold.",
        "kind_boost": "Boost from preferred symbol kind ranking.",
        "public_exported_boost": "Boost from public/exported API visibility.",
        "same_directory_boost": "Boost from same-directory proximity to reference file.",
        "same_language_boost": "Boost from same-language proximity to reference language.",
        "recent_file_boost": "Boost from recently indexed file status.",
        "changed_file_boost": "Boost from current changed-file membership.",
        "graph_expansion": "Graph expansion metadata: hop_distance and seed_qualified_name.",
        "hybrid_rrf": "Hybrid fusion metadata: per-source rank and reciprocal-rank score contribution."
    })
}

fn merge_option_f64(target: &mut Option<f64>, incoming: Option<f64>) {
    if let Some(incoming) = incoming {
        match target {
            Some(existing) => {
                if incoming > *existing {
                    *existing = incoming;
                }
            }
            None => *target = Some(incoming),
        }
    }
}

fn choose_fuzzy(
    existing: &FuzzyCorrectionEvidence,
    incoming: &FuzzyCorrectionEvidence,
) -> FuzzyCorrectionEvidence {
    match (existing.edit_distance, incoming.edit_distance) {
        (Some(left), Some(right)) if right < left => incoming.clone(),
        (None, Some(_)) => incoming.clone(),
        _ => existing.clone(),
    }
}

fn choose_graph_expansion(
    existing: &GraphExpansionEvidence,
    incoming: &GraphExpansionEvidence,
) -> GraphExpansionEvidence {
    if incoming.hop_distance < existing.hop_distance {
        incoming.clone()
    } else {
        existing.clone()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub text: String,
    pub kind: Option<String>,
    pub language: Option<String>,
    /// Include file nodes in search results.
    ///
    /// Defaults to `false` because most callers want symbol-centric results.
    pub include_files: bool,
    pub file_path: Option<String>,
    /// Filter results whose `file_path` starts with this subpath prefix.
    pub subpath: Option<String>,
    pub is_test: Option<bool>,
    pub limit: usize,
    /// Expand FTS seed results through graph edges.
    pub graph_expand: bool,
    /// Maximum edge hops when `graph_expand` is true (default: 1).
    pub graph_max_hops: u32,
    /// Reference file path for same-directory boost. When set, results in the
    /// same directory as this file receive a ranking bonus.
    pub reference_file: Option<String>,
    /// Reference language for same-language boost. When set, results in the
    /// same language receive a ranking bonus.
    pub reference_language: Option<String>,
    /// Enable fuzzy (edit-distance) typo recovery for near-miss symbol names.
    /// Off by default because it adds O(results) edit-distance work plus a
    /// wider relaxed-candidate search.
    pub fuzzy_match: bool,
    /// Boost nodes whose file was among the most recently indexed (+4). Requires
    /// one extra DB read inside `atlas_search::search`; off by default.
    pub recent_file_boost: bool,
    /// Boost nodes whose file appears in this set of changed file paths (+5).
    /// Caller populates this with the paths from the current git diff.
    /// Empty vec disables the boost.
    pub changed_files: Vec<String>,
    /// Enable hybrid (FTS + vector) retrieval.
    ///
    /// When `true` and `ATLAS_EMBED_URL` is set, the search layer runs both
    /// FTS and vector retrieval and merges results with Reciprocal Rank Fusion.
    /// Falls back to FTS-only when no embedding backend is configured.
    pub hybrid: bool,
    /// FTS candidate pool size before RRF merge (default: 60).
    pub top_k_fts: usize,
    /// Vector candidate pool size before RRF merge (default: 60).
    pub top_k_vector: usize,
    /// Reciprocal Rank Fusion k constant (default: 60).
    pub rrf_k: u32,
    /// Optional regex pattern applied as a SQL-layer UDF filter (`atlas_regexp`) against
    /// `name` and `qualified_name`. When set and `text` is empty, the structural scan path
    /// is used instead of FTS5. When both `text` and `regex_pattern` are set, FTS5 runs
    /// first and the UDF filters the results inside SQLite.
    ///
    /// Patterns must be valid `regex` crate syntax. An invalid pattern is
    /// returned as an error rather than silently skipped.
    pub regex_pattern: Option<String>,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            text: String::new(),
            kind: None,
            language: None,
            include_files: false,
            file_path: None,
            subpath: None,
            is_test: None,
            limit: 20,
            graph_expand: false,
            graph_max_hops: 1,
            reference_file: None,
            reference_language: None,
            fuzzy_match: false,
            recent_file_boost: false,
            changed_files: vec![],
            hybrid: false,
            top_k_fts: 60,
            top_k_vector: 60,
            rrf_k: 60,
            regex_pattern: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredNode {
    pub node: Node,
    pub score: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ranking_evidence: Option<RankingEvidence>,
}

impl ScoredNode {
    pub fn new(node: Node, score: f64) -> Self {
        Self {
            node,
            score,
            ranking_evidence: None,
        }
    }

    pub fn with_ranking_evidence(
        node: Node,
        score: f64,
        mut ranking_evidence: RankingEvidence,
    ) -> Self {
        ranking_evidence.final_score = score;
        Self {
            node,
            score,
            ranking_evidence: Some(ranking_evidence),
        }
    }

    pub fn sync_ranking_score(&mut self) {
        if let Some(ranking_evidence) = &mut self.ranking_evidence {
            ranking_evidence.final_score = self.score;
        }
    }
}
