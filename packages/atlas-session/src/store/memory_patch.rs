//! VM2.2 — VikingMem EUA (`Entity Update Algorithm`) patch parser and applier.
//!
//! A patch is a sequence of blocks:
//!
//! ```text
//! <<<< SEARCH
//! old text
//! ====
//! new text
//! >>>> REPLACE
//! ```
//!
//! Exact substring replacement runs first and requires a unique match. When no
//! exact match exists, a bounded Levenshtein scan finds the closest window no
//! larger than the search text. Parsing and matching are deterministic and
//! never call a model or the network.

use sha2::{Digest, Sha256};

use atlas_core::{AtlasError, Result};

use super::memory_ops::{MemoryOp, OpOutcome, apply_memory_op_on, memory_op_now};
use super::util::hex_encode;

const SEARCH_MARKER: &str = "<<<< SEARCH";
const REPLACE_MARKER: &str = ">>>> REPLACE";
const SEPARATOR_MARKER: &str = "====";

/// Default fuzzy threshold as a fraction of the search length.
const DEFAULT_EDIT_DISTANCE_RATIO: f64 = 0.15;
/// Maximum caller-supplied threshold as a fraction of the search length.
const MAX_EDIT_DISTANCE_RATIO: f64 = 0.5;

/// One parsed search/replace block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchBlock {
    pub search: String,
    pub replace: String,
}

/// Deterministic result of applying one block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchApplication {
    pub body: String,
    /// Lowercase hex SHA-256 of the search text.
    pub search_hash: String,
    pub replace_len: usize,
    pub edit_distance: usize,
    pub matched_exact: bool,
}

/// Parse every EUA block in `patch` in document order.
///
/// Unbalanced, nested, or unknown markers fail with `memory_patch_invalid`.
pub fn parse_patch_blocks(patch: &str) -> Result<Vec<PatchBlock>> {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Outside,
        Search,
        Replace,
    }

    let mut state = State::Outside;
    let mut search = Vec::new();
    let mut replace = Vec::new();
    let mut blocks = Vec::new();

    for (index, raw_line) in patch.lines().enumerate() {
        let line_no = index + 1;
        let line = raw_line.trim_end();
        let marker = line.trim();
        match state {
            State::Outside => {
                if marker.is_empty() {
                    continue;
                }
                if marker == SEARCH_MARKER {
                    state = State::Search;
                    search.clear();
                    replace.clear();
                } else {
                    return Err(invalid(format!(
                        "line {line_no}: expected '{SEARCH_MARKER}' or blank line"
                    )));
                }
            }
            State::Search => {
                if marker == SEPARATOR_MARKER {
                    state = State::Replace;
                } else if is_marker(marker) {
                    return Err(invalid(format!(
                        "line {line_no}: unbalanced marker '{marker}' inside SEARCH"
                    )));
                } else {
                    search.push(line);
                }
            }
            State::Replace => {
                if marker == REPLACE_MARKER {
                    let search = search.join("\n");
                    if search.trim().is_empty() {
                        return Err(invalid(format!(
                            "line {line_no}: SEARCH text must not be empty"
                        )));
                    }
                    blocks.push(PatchBlock {
                        search,
                        replace: replace.join("\n"),
                    });
                    state = State::Outside;
                } else if is_marker(marker) {
                    return Err(invalid(format!(
                        "line {line_no}: unbalanced marker '{marker}' inside REPLACE"
                    )));
                } else {
                    replace.push(line);
                }
            }
        }
    }

    match state {
        State::Outside => {}
        State::Search => {
            return Err(invalid(
                "unterminated patch: missing '====' separator".to_owned(),
            ));
        }
        State::Replace => {
            return Err(invalid(
                "unterminated patch: missing '>>>> REPLACE' marker".to_owned(),
            ));
        }
    }

    if blocks.is_empty() {
        return Err(invalid("patch contains no blocks".to_owned()));
    }
    Ok(blocks)
}

fn is_marker(line: &str) -> bool {
    line == SEARCH_MARKER || line == REPLACE_MARKER || line == SEPARATOR_MARKER
}

/// Apply one block to `body` with exact-first, bounded-Levenshtein fallback.
pub fn apply_patch_block(
    body: &str,
    search: &str,
    replace: &str,
    max_edit_distance: Option<usize>,
) -> Result<PatchApplication> {
    if search.trim().is_empty() {
        return Err(invalid("search text must not be empty".to_owned()));
    }
    let search_len = search.chars().count();
    let threshold = resolve_max_edit_distance(search_len, max_edit_distance)?;

    let matches = body.match_indices(search).collect::<Vec<_>>();
    let (updated, edit_distance, matched_exact) = match matches.len() {
        1 => {
            let (start, matched) = matches[0];
            let mut updated = String::with_capacity(body.len() + replace.len());
            updated.push_str(&body[..start]);
            updated.push_str(replace);
            updated.push_str(&body[start + matched.len()..]);
            (updated, 0, true)
        }
        0 => {
            let (start, end, distance) = best_window(body, search, threshold)?;
            let mut updated = String::with_capacity(body.len() + replace.len());
            updated.push_str(&body[..start]);
            updated.push_str(replace);
            updated.push_str(&body[end..]);
            (updated, distance, false)
        }
        count => {
            return Err(AtlasError::Other(format!(
                "memory_patch_ambiguous: search text matches {count} locations; \
                 include more surrounding text"
            )));
        }
    };

    if updated.trim().is_empty() {
        return Err(invalid(
            "patch would produce an empty body; use the delete command instead".to_owned(),
        ));
    }

    Ok(PatchApplication {
        body: updated,
        search_hash: sha256_hex(search),
        replace_len: replace.chars().count(),
        edit_distance,
        matched_exact,
    })
}

/// Resolve the effective edit-distance threshold and enforce the hard ceiling.
fn resolve_max_edit_distance(search_len: usize, requested: Option<usize>) -> Result<usize> {
    let ceiling = (search_len as f64 * MAX_EDIT_DISTANCE_RATIO).floor() as usize;
    if let Some(requested) = requested {
        if requested > ceiling {
            return Err(invalid(format!(
                "max_edit_distance {requested} exceeds 50% of search length {search_len}"
            )));
        }
        return Ok(requested);
    }
    let default = (search_len as f64 * DEFAULT_EDIT_DISTANCE_RATIO).ceil() as usize;
    Ok(default.max(1).min(ceiling.max(1)))
}

/// Locate the closest equal-length window, failing on ties or out-of-threshold.
fn best_window(body: &str, search: &str, threshold: usize) -> Result<(usize, usize, usize)> {
    let body_chars = body.char_indices().collect::<Vec<_>>();
    let search_chars = search.chars().count();
    if body_chars.len() < search_chars {
        return Err(no_match(threshold));
    }

    let mut best: Option<(usize, usize, usize)> = None;
    let mut tied = false;
    for window_start in 0..=body_chars.len() - search_chars {
        let start_byte = body_chars[window_start].0;
        let end_char = window_start + search_chars;
        let end_byte = body_chars
            .get(end_char)
            .map(|(offset, _)| *offset)
            .unwrap_or(body.len());
        // Only compare on character boundaries; `strsim` measures char distance.
        let distance = strsim::levenshtein(&body[start_byte..end_byte], search);
        if distance > threshold {
            continue;
        }
        match best {
            None => best = Some((start_byte, end_byte, distance)),
            Some((_, _, current)) if distance < current => {
                best = Some((start_byte, end_byte, distance));
                tied = false;
            }
            Some((_, _, current)) if distance == current => tied = true,
            _ => {}
        }
    }

    match best {
        Some((_, _, _)) if tied => Err(AtlasError::Other(
            "memory_patch_ambiguous: multiple windows within max_edit_distance; \
             include more surrounding text"
                .to_owned(),
        )),
        Some((start, end, distance)) => Ok((start, end, distance)),
        None => Err(no_match(threshold)),
    }
}

fn no_match(threshold: usize) -> AtlasError {
    AtlasError::Other(format!(
        "memory_patch_invalid: no match within max_edit_distance {threshold}"
    ))
}

fn invalid(message: String) -> AtlasError {
    AtlasError::Other(format!("memory_patch_invalid: {message}"))
}

fn sha256_hex(value: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    hex_encode(&hasher.finalize())
}

impl super::SessionStore {
    /// Parse and apply an EUA patch document to one memory row.
    ///
    /// Every block runs in document order inside one transaction; the returned
    /// outcomes carry one `eua_patch` history entry per block.
    pub fn apply_memory_patch(
        &mut self,
        repo_root: &str,
        memory_id: &str,
        patch_text: &str,
        max_edit_distance: Option<usize>,
    ) -> Result<Vec<OpOutcome>> {
        let blocks = parse_patch_blocks(patch_text)?;
        let now = memory_op_now();
        let tx = self
            .conn
            .transaction()
            .map_err(|error| AtlasError::Db(error.to_string()))?;
        let mut outcomes = Vec::with_capacity(blocks.len());
        for block in blocks {
            let op = MemoryOp::EuaPatch {
                search: block.search,
                replace: block.replace,
                max_edit_distance,
            };
            outcomes.push(apply_memory_op_on(&tx, repo_root, memory_id, &op, &now)?);
        }
        tx.commit()
            .map_err(|error| AtlasError::Db(error.to_string()))?;
        Ok(outcomes)
    }
}
