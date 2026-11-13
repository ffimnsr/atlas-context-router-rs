//! Near-match suggestions for unresolved `get_docs_section` heading lookups.
//!
//! Kept in its own module so the excerpt/discovery tool surface stays focused
//! on file reads while miss-resolution can grow independently.

use atlas_review::{DocsSectionLookup, suggest_docs_headings};
use atlas_store_sqlite::Store;

/// Build the unresolved lookup payload for a heading that does not exist,
/// populated with near-match candidates so the caller can retry exactly.
pub(super) fn unresolved_heading_lookup(
    store: &Store,
    repo_root: &str,
    file: &str,
    query: &str,
) -> DocsSectionLookup {
    let candidates = suggest_docs_headings(store, camino::Utf8Path::new(repo_root), file, query, 5)
        .unwrap_or_default();
    DocsSectionLookup {
        file: file.to_owned(),
        selector_kind: "heading".to_owned(),
        resolved: false,
        query: Some(query.to_owned()),
        title: None,
        heading_path: None,
        heading_slug: None,
        heading_level: None,
        start_line: None,
        end_line: None,
        line_count: None,
        file_hash: None,
        content: None,
        lines: Vec::new(),
        truncated: false,
        omitted_byte_count: 0,
        candidates,
        atlas_result_kind: "docs_section",
    }
}
