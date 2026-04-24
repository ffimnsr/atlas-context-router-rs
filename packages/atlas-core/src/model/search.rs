use serde::{Deserialize, Serialize};

use super::graph::Node;

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
}
