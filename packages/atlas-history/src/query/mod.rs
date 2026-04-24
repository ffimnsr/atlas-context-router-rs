mod dependency;
mod file;
mod module;
mod snapshot_loader;
mod symbol;
mod types;

use std::collections::BTreeSet;

use atlas_store_sqlite::HistoricalNode;

pub use dependency::query_dependency_history;
pub use file::{query_file_history, query_file_history_with_options};
pub use module::query_module_history;
pub use symbol::query_symbol_history;
pub use types::{
    DependencyHistoryPoint, EdgeHistoryFindings, EdgeHistoryReport, EdgeHistorySummary,
    FileHistoryFindings, FileHistoryPoint, FileHistoryReport, FileHistorySummary,
    ModuleHistoryFindings, ModuleHistoryPoint, ModuleHistoryReport, ModuleHistorySummary,
    NodeChangeRecord, NodeFilePathSnapshot, NodeHistoryFindings, NodeHistoryReport,
    NodeHistorySummary, NodeSignatureRecord, NodeSignatureSnapshot,
};

pub(crate) use snapshot_loader::{
    load_partial_snapshot_state_for_paths, load_snapshot_catalog, load_snapshot_states,
};
pub(crate) use types::SnapshotState;

pub(crate) fn node_signature_hash(node: &HistoricalNode) -> Option<String> {
    if node.params.is_none() && node.return_type.is_none() && node.modifiers.is_none() {
        return None;
    }
    let payload = format!(
        "{}\u{1f}{}\u{1f}{}",
        node.params.as_deref().unwrap_or(""),
        node.return_type.as_deref().unwrap_or(""),
        node.modifiers.as_deref().unwrap_or(""),
    );
    Some(sha256_hex(payload.as_bytes()))
}

pub(crate) fn signature_record_key(record: &NodeSignatureRecord) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}",
        record.file_path,
        record.kind,
        record.params.as_deref().unwrap_or(""),
        record.return_type.as_deref().unwrap_or(""),
        record.modifiers.as_deref().unwrap_or(""),
        record.signature_hash.as_deref().unwrap_or(""),
    )
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub(crate) fn sorted_strings(values: impl IntoIterator<Item = String>) -> Vec<String> {
    values
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(crate) fn node_identifier_for_source(value: &str) -> String {
    value.to_owned()
}
