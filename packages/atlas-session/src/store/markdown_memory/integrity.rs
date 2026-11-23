use std::collections::BTreeSet;

use atlas_store_markdown::MarkdownStore;

use super::scan::scan_lenient;

// ── Integrity ─────────────────────────────────────────────────────────────────

/// Markdown-tree integrity issues surfaced by `atlas db check`: malformed
/// frontmatter and dangling links, reported per file/id.
pub(crate) fn tree_integrity(store: &MarkdownStore, repo_root: &str) -> Vec<String> {
    let (records, issues) = scan_lenient(store, repo_root);
    let mut out = issues;
    let known = records
        .iter()
        .map(|record| record.id.as_str())
        .collect::<BTreeSet<_>>();
    for record in &records {
        for link in &record.links {
            if !known.contains(link.id.as_str()) {
                out.push(format!(
                    "dangling memory link: {} -> {} ({})",
                    record.id, link.id, link.relation
                ));
            }
        }
    }
    out
}
