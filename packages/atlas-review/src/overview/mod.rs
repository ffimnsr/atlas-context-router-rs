//! ICM-G — code overview memory service.
//!
//! Atlas stays non-LLM here: the service exports bounded evidence packets,
//! ingests externally produced analysis, and plans freshness refreshes. It
//! never calls a model provider and never writes overview bodies into the code
//! graph database.

mod docs;
mod export;
mod freshness;
mod ingest;
mod types;
mod util;

#[cfg(test)]
mod tests;

pub use docs::export_overview_docs;
pub use export::export_overview_packets;
pub use freshness::{OverviewFreshnessService, refresh_overview_plan, validate_since};
pub use ingest::ingest_overview_analysis;
pub use types::{
    OverviewDocsExport, OverviewExportResult, OverviewIngestResult, OverviewRefreshItem,
    OverviewRefreshPlan,
};
