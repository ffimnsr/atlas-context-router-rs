//! VM2.1 — deterministic, non-destructive memory consolidation.
//!
//! VikingMem's `TIME_COMPRESS` merges an entity's evolving timeline without an
//! LLM. Atlas applies the same rule to [`MemoryRecord`] groups: every distinct
//! sentence or bullet from every member is preserved in chronological order,
//! near-duplicates collapse to the first observed text, and oversized results
//! route their full body to `context.db` while the `memories` row keeps a
//! bounded display copy.
//!
//! The synthesizer is pure; overflow routing is delegated to a caller-provided
//! sink so this crate never depends on content storage.

use std::collections::HashSet;

use atlas_core::Result;
use sha2::{Digest, Sha256};

use super::types::{MemoryRecord, OVERSIZED_BODY_CHARS};
use super::util::hex_encode;

/// Content-store source type used for consolidated-body overflow.
pub const CONSOLIDATION_OVERFLOW_SOURCE_TYPE: &str = "memory_consolidation";

/// Callback that persists an oversized synthesized body (typically into
/// `context.db`) under the given deterministic source id.
pub type OverflowSink<'a> = dyn FnMut(&str, Option<&str>, &str) -> Result<()> + 'a;

/// Result of deterministic body synthesis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsolidatedBody {
    /// Body stored on the consolidated `memories` row (bounded by
    /// [`OVERSIZED_BODY_CHARS`]).
    pub body: String,
    /// Full synthesized body; `Some` only when [`Self::overflow`] is true.
    pub full_body: Option<String>,
    /// Number of distinct units retained before truncation.
    pub distinct_units: usize,
    /// Whether the full body exceeded [`OVERSIZED_BODY_CHARS`].
    pub overflow: bool,
}

/// Synthesize one deterministic body from all consolidation members.
///
/// Members are ordered by `created_at ASC` (ties by id) so output order is
/// stable regardless of input order. Every distinct sentence/bullet is kept;
/// duplicates retain the first chronological original text.
pub fn synthesize_consolidated_body(members: &[MemoryRecord]) -> ConsolidatedBody {
    let mut ordered = members.to_vec();
    ordered.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut seen = HashSet::new();
    let mut units = Vec::new();
    for member in &ordered {
        for unit in split_units(&member.body) {
            let key = normalized_unit_key(&unit);
            if key.is_empty() {
                continue;
            }
            if seen.insert(key) {
                units.push(unit);
            }
        }
    }

    let full = units.join("\n");
    let overflow = full.chars().count() > OVERSIZED_BODY_CHARS;
    let body = if overflow {
        full.chars().take(OVERSIZED_BODY_CHARS).collect()
    } else {
        full.clone()
    };
    let full_body = overflow.then_some(full);

    ConsolidatedBody {
        body,
        full_body,
        distinct_units: units.len(),
        overflow,
    }
}

/// Deterministic source id for an overflow body, derived from the repo and the
/// merged member ids so repeat consolidations reuse the same artifact.
pub fn overflow_source_id(repo_root: &str, merged_ids: &[String]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(CONSOLIDATION_OVERFLOW_SOURCE_TYPE.as_bytes());
    hasher.update(b"\x00");
    hasher.update(repo_root.as_bytes());
    for id in merged_ids {
        hasher.update(b"\x00");
        hasher.update(id.as_bytes());
    }
    hex_encode(&hasher.finalize())
}

/// Route a full synthesized body to `context.db`, returning the source id.
///
/// Implemented by callers through [`OverflowSink`]; the session crate itself
/// keeps no content-storage dependency.
pub fn route_overflow_body(
    sink: &mut OverflowSink<'_>,
    repo_root: &str,
    merged_ids: &[String],
    session_id: Option<&str>,
    body: &str,
) -> Result<String> {
    let source_id = overflow_source_id(repo_root, merged_ids);
    sink(&source_id, session_id, body)?;
    Ok(source_id)
}

/// Split a body into sentence/bullet units while preserving original text.
///
/// Units split on newlines and on `. `, `! `, `? `, and `。` terminators. The
/// terminating punctuation stays attached to its sentence.
fn split_units(body: &str) -> Vec<String> {
    let mut units = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut start = 0usize;
        let chars = trimmed.char_indices().collect::<Vec<_>>();
        for (index, (byte_index, ch)) in chars.iter().enumerate() {
            let next = chars.get(index + 1).map(|(_, next)| *next);
            let boundary = match ch {
                '。' => true,
                '.' | '!' | '?' => next.is_none() || next.is_some_and(char::is_whitespace),
                _ => false,
            };
            if !boundary {
                continue;
            }
            let end = byte_index + ch.len_utf8();
            let unit = trimmed[start..end].trim();
            if !unit.is_empty() {
                units.push(unit.to_owned());
            }
            start = end;
        }
        if start < trimmed.len() {
            let unit = trimmed[start..].trim();
            if !unit.is_empty() {
                units.push(unit.to_owned());
            }
        }
    }
    units
}

/// Dedupe key: lowercase, whitespace-collapsed, trailing punctuation stripped,
/// and leading list markers removed.
fn normalized_unit_key(unit: &str) -> String {
    let collapsed = unit
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let without_marker = collapsed
        .trim_start_matches(['-', '*', '+'])
        .trim_start()
        .trim_start_matches(|ch: char| ch.is_ascii_digit())
        .trim_start_matches(['.', ')', ']'])
        .trim_start();
    without_marker
        .trim_end_matches(['.', '!', '?', ';', ':', '。', '、', ','])
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::Result;
    use serde_json::Value;

    fn member(id: &str, created_at: &str, body: &str) -> MemoryRecord {
        MemoryRecord {
            id: id.to_owned(),
            repo_root: "/repo".to_owned(),
            session_id: None,
            frontend: None,
            scope: Default::default(),
            topic: "deploy".to_owned(),
            title: "Deploy".to_owned(),
            body: body.to_owned(),
            importance: Default::default(),
            created_at: created_at.to_owned(),
            updated_at: created_at.to_owned(),
            last_accessed_at: created_at.to_owned(),
            decay_score: 0.0,
            source_id: None,
            metadata: Value::Null,
            superseded_by: None,
        }
    }

    fn synthesize(members: &[MemoryRecord]) -> Result<ConsolidatedBody> {
        Ok(synthesize_consolidated_body(members))
    }

    #[test]
    fn merges_unique_sentences_in_chronological_order() {
        let merged = synthesize(&[
            member("b", "2026-01-02T00:00:00Z", "second fact. shared fact."),
            member("a", "2026-01-01T00:00:00Z", "first fact. Shared fact!"),
        ])
        .unwrap();
        assert_eq!(merged.body, "first fact.\nShared fact!\nsecond fact.");
        assert_eq!(merged.distinct_units, 3);
        assert!(!merged.overflow);
        assert_eq!(merged.full_body, None);
    }

    #[test]
    fn dedupes_normalized_bullets_and_sentences() {
        let merged = synthesize(&[
            member("a", "2026-01-01T00:00:00Z", "- Helm deploy\n- rollback"),
            member("b", "2026-01-02T00:00:00Z", "helm deploy.\nROLLBACK"),
        ])
        .unwrap();
        assert_eq!(merged.body, "- Helm deploy\n- rollback");
        assert_eq!(merged.distinct_units, 2);
    }

    #[test]
    fn routes_oversized_body_to_full_body() {
        let source = "fact ".repeat(OVERSIZED_BODY_CHARS);
        let source = source.trim();
        let expected_chars = source.chars().count();
        let merged = synthesize(&[member("a", "2026-01-01T00:00:00Z", source)]).unwrap();
        assert!(merged.overflow);
        assert!(merged.body.chars().count() <= OVERSIZED_BODY_CHARS);
        assert_eq!(
            merged
                .full_body
                .as_deref()
                .map(str::chars)
                .map(Iterator::count),
            Some(expected_chars)
        );
    }

    #[test]
    fn overflow_source_id_is_stable_and_distinct() {
        let ids = vec!["a".to_owned(), "b".to_owned()];
        assert_eq!(
            overflow_source_id("/repo", &ids),
            overflow_source_id("/repo", &ids)
        );
        assert_ne!(
            overflow_source_id("/repo", &ids),
            overflow_source_id("/other", &ids)
        );
    }
}
