//! Derive a decision-memory search query from a context request.
//!
//! Shared by the CLI `context` command and the MCP `get_context` tool, which
//! previously carried identical private copies.

use atlas_core::model::{ContextRequest, ContextTarget};

/// Build the free-text decision lookup query implied by `request`, or `None`
/// when the target carries no usable text.
pub fn decision_lookup_query(request: &ContextRequest) -> Option<String> {
    match &request.target {
        ContextTarget::QualifiedName { qname } => Some(qname.clone()),
        ContextTarget::SymbolName { name } => Some(name.clone()),
        ContextTarget::FilePath { path } => Some(path.clone()),
        ContextTarget::ChangedFiles { paths } => {
            let joined = paths.iter().take(3).cloned().collect::<Vec<_>>().join(" ");
            (!joined.is_empty()).then_some(joined)
        }
        ContextTarget::ChangedSymbols { qnames } => {
            let joined = qnames.iter().take(3).cloned().collect::<Vec<_>>().join(" ");
            (!joined.is_empty()).then_some(joined)
        }
        ContextTarget::EdgeQuerySeed { source_qname, .. } => Some(source_qname.clone()),
    }
}
