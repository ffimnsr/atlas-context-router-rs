#![doc = include_str!("../README.md")]

use std::collections::{HashMap, HashSet};

use atlas_core::{
    GraphExpansionEvidence, HybridRankContribution, HybridRankingSource, HybridRrfEvidence,
    NodeKind, RankingEvidence, Result, RetrievalMode, ScoredNode, SearchMatchedField, SearchQuery,
};
use atlas_store_sqlite::Store;
use serde::Serialize;
use tracing::debug;

pub mod capabilities;
pub mod embed;
pub mod eval;
mod ranking;
pub mod semantic;

use capabilities::BackendCapabilities;
use ranking::{GraphSearchRankingPrimitives, sort_scored_nodes};

mod execution;
mod expand;
mod fuzzy;
mod hybrid;
mod scoring;
mod search;
mod tokenize;

use fuzzy::*;
use hybrid::*;
use scoring::*;
use tokenize::*;

pub use execution::{
    QueryExplainFiltersApplied, QueryExplainInput, QueryExplainMatch, QueryExplanation,
    execute_query, execute_query_with_embedding, explain_query, explain_query_with_embedding,
};
pub use expand::graph_expand;
pub use hybrid::reciprocal_rank_fusion;
pub use scoring::apply_ranking_boosts;
pub use search::{search, search_with_embedding};
pub use tokenize::build_fts_query;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;
