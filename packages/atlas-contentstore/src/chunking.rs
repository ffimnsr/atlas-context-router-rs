//! Deterministic chunking of raw artifact text into retrieval-ready pieces.
//!
//! Rules (from CM2 spec):
//! - Markdown: split by headings first
//! - Plain text: split by paragraph blocks or line windows
//! - JSON: split by path and array batches
//! - Oversized chunks: subdivide further
//! - Each chunk preserves stable `chunk_index` for display order
//! - Each chunk carries a stable content-derived `chunk_id` for dedup/reuse
//! - Each chunk carries a human-readable `title` when possible

use sha2::{Digest, Sha256};

const MAX_CHUNK_BYTES: usize = 4096;
const LINE_WINDOW: usize = 60;

/// A single extractable piece of an artifact.
#[derive(Debug, Clone)]
pub struct Chunk {
    /// Stable content-derived identity: SHA-256 hex over (source_id + normalized content).
    /// Invariant across re-indexing as long as source and content are unchanged.
    pub chunk_id: String,
    /// Display/storage order; may change when content is inserted or removed.
    pub chunk_index: usize,
    pub title: Option<String>,
    pub content: String,
    pub content_type: String,
}

/// Compute a stable chunk identity from `source_id` and normalized `content`.
///
/// The hash is SHA-256 over `source_id + NUL + content.trim()`.  Moving a
/// chunk to a different index does not change its `chunk_id`; only a change
/// to the source path or the chunk's own content produces a new id.
pub fn compute_chunk_id(source_id: &str, content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(source_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(content.trim().as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Chunk raw text according to `content_type` heuristics.
///
/// `source_id` is incorporated into each chunk's [`Chunk::chunk_id`] so that
/// the identity is scoped to the originating artifact.
pub fn chunk_text(source_id: &str, raw: &str, content_type: &str) -> Vec<Chunk> {
    let base = match content_type {
        "text/markdown" | "markdown" => chunk_markdown(raw),
        "application/json" | "json" => chunk_json(raw),
        _ => chunk_plain(raw),
    };

    // Subdivide any chunk that still exceeds the byte cap.
    let mut out: Vec<Chunk> = Vec::new();
    let mut idx: usize = 0;
    for chunk in base {
        if chunk.content.len() <= MAX_CHUNK_BYTES {
            let chunk_id = compute_chunk_id(source_id, &chunk.content);
            out.push(Chunk {
                chunk_id,
                chunk_index: idx,
                ..chunk
            });
            idx += 1;
        } else {
            for sub in subdivide(&chunk.content, chunk.title.as_deref(), &chunk.content_type) {
                let chunk_id = compute_chunk_id(source_id, &sub.content);
                out.push(Chunk {
                    chunk_id,
                    chunk_index: idx,
                    ..sub
                });
                idx += 1;
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Markdown chunking — split by ATX headings
// ---------------------------------------------------------------------------

fn chunk_markdown(src: &str) -> Vec<Chunk> {
    let mut chunks: Vec<Chunk> = Vec::new();
    let mut current_title: Option<String> = None;
    let mut current_lines: Vec<&str> = Vec::new();

    for line in src.lines() {
        if let Some(heading) = extract_heading(line) {
            flush_text_chunk(
                &mut chunks,
                &mut current_lines,
                current_title.take(),
                "text/markdown",
            );
            current_title = Some(heading);
        } else {
            current_lines.push(line);
        }
    }
    flush_text_chunk(
        &mut chunks,
        &mut current_lines,
        current_title,
        "text/markdown",
    );
    if chunks.is_empty() {
        chunks.push(Chunk {
            chunk_id: String::new(),
            chunk_index: 0,
            title: None,
            content: src.to_string(),
            content_type: "text/markdown".into(),
        });
    }
    chunks
}

fn extract_heading(line: &str) -> Option<String> {
    let trimmed = line.trim_start_matches('#');
    if trimmed.len() < line.len() && line.starts_with('#') {
        let title = trimmed.trim();
        if !title.is_empty() {
            return Some(title.to_string());
        }
    }
    None
}

fn flush_text_chunk(
    chunks: &mut Vec<Chunk>,
    lines: &mut Vec<&str>,
    title: Option<String>,
    content_type: &str,
) {
    let text = lines.join("\n");
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        chunks.push(Chunk {
            // chunk_id and chunk_index will be set by chunk_text after subdivision.
            chunk_id: String::new(),
            chunk_index: 0,
            title,
            content: trimmed.to_string(),
            content_type: content_type.to_string(),
        });
    }
    lines.clear();
}

// ---------------------------------------------------------------------------
// Plain-text chunking — split by blank lines (paragraphs) with line-window cap
// ---------------------------------------------------------------------------

fn chunk_plain(src: &str) -> Vec<Chunk> {
    // Split into paragraphs at blank lines.
    let mut paras: Vec<String> = Vec::new();
    let mut current: Vec<&str> = Vec::new();

    for line in src.lines() {
        if line.trim().is_empty() {
            if !current.is_empty() {
                paras.push(current.join("\n"));
                current.clear();
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        paras.push(current.join("\n"));
    }

    if paras.is_empty() {
        // Fall back to fixed line windows.
        return line_window_chunks(src, LINE_WINDOW, "text/plain");
    }

    paras
        .into_iter()
        .map(|p| Chunk {
            chunk_id: String::new(),
            chunk_index: 0,
            title: None,
            content: p,
            content_type: "text/plain".into(),
        })
        .collect()
}

fn line_window_chunks(src: &str, window: usize, content_type: &str) -> Vec<Chunk> {
    src.lines()
        .collect::<Vec<_>>()
        .chunks(window)
        .map(|w| Chunk {
            chunk_id: String::new(),
            chunk_index: 0,
            title: None,
            content: w.join("\n"),
            content_type: content_type.to_string(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// JSON chunking — top-level keys or array element windows
// ---------------------------------------------------------------------------

fn chunk_json(src: &str) -> Vec<Chunk> {
    // Best-effort: try to parse and split top-level entries.
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(src) {
        match value {
            serde_json::Value::Object(map) => {
                return map
                    .into_iter()
                    .map(|(k, v)| Chunk {
                        chunk_id: String::new(),
                        chunk_index: 0,
                        title: Some(k),
                        content: serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string()),
                        content_type: "application/json".into(),
                    })
                    .collect();
            }
            serde_json::Value::Array(arr) => {
                return arr
                    .chunks(20)
                    .enumerate()
                    .map(|(i, batch)| Chunk {
                        chunk_id: String::new(),
                        chunk_index: 0,
                        title: Some(format!("array[{}..{}]", i * 20, i * 20 + batch.len())),
                        content: serde_json::to_string_pretty(batch)
                            .unwrap_or_else(|_| format!("{batch:?}")),
                        content_type: "application/json".into(),
                    })
                    .collect();
            }
            _ => {}
        }
    }
    // Fallback: treat as plain text.
    chunk_plain(src)
}

// ---------------------------------------------------------------------------
// Oversized chunk subdivision — line windows, then byte windows
// ---------------------------------------------------------------------------

fn subdivide(content: &str, parent_title: Option<&str>, content_type: &str) -> Vec<Chunk> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() > 1 {
        // Have multiple lines: split by line window.
        let window = (MAX_CHUNK_BYTES / 80).max(10);
        return lines
            .chunks(window)
            .enumerate()
            .map(|(i, w)| Chunk {
                // chunk_id computed by chunk_text after subdivision.
                chunk_id: String::new(),
                chunk_index: 0,
                title: parent_title.map(|t| format!("{t} (part {i})")),
                content: w.join("\n"),
                content_type: content_type.to_string(),
            })
            .collect();
    }

    // Single long line (or no newlines): split by byte chunks.
    let bytes = content.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut part = 0;
    while start < bytes.len() {
        let end = (start + MAX_CHUNK_BYTES).min(bytes.len());
        // Snap to a valid UTF-8 boundary.
        let end = snap_utf8(bytes, end);
        let slice = &content[start..end];
        out.push(Chunk {
            // chunk_id computed by chunk_text after subdivision.
            chunk_id: String::new(),
            chunk_index: 0,
            title: parent_title.map(|t| format!("{t} (part {part})")),
            content: slice.to_string(),
            content_type: content_type.to_string(),
        });
        start = end;
        part += 1;
    }
    out
}

/// Find the largest index ≤ `pos` that is a valid UTF-8 char boundary.
fn snap_utf8(bytes: &[u8], pos: usize) -> usize {
    let mut p = pos.min(bytes.len());
    while p > 0 && (bytes[p - 1] & 0xC0) == 0x80 {
        p -= 1;
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_split_by_headings() {
        let md = "# Intro\nhello\n## Details\nworld\n";
        let chunks = chunk_text("src/doc.md", md, "text/markdown");
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].title.as_deref(), Some("Intro"));
        assert_eq!(chunks[1].title.as_deref(), Some("Details"));
    }

    #[test]
    fn plain_text_split_by_paragraph() {
        let txt = "para one line one\npara one line two\n\npara two here\n";
        let chunks = chunk_text("src/notes.txt", txt, "text/plain");
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn json_object_split_by_key() {
        let json = r#"{"alpha": 1, "beta": 2}"#;
        let chunks = chunk_text("data.json", json, "application/json");
        assert_eq!(chunks.len(), 2);
    }

    #[test]
    fn chunk_indices_sequential() {
        let md = "# A\nfoo\n# B\nbar\n# C\nbaz\n";
        let chunks = chunk_text("doc.md", md, "markdown");
        for (i, c) in chunks.iter().enumerate() {
            assert_eq!(c.chunk_index, i);
        }
    }

    #[test]
    fn oversized_chunk_subdivided() {
        // Create text larger than MAX_CHUNK_BYTES with single paragraph.
        let big = "x ".repeat(3000);
        let chunks = chunk_text("big.txt", &big, "text/plain");
        assert!(chunks.len() > 1, "oversized chunk must be subdivided");
        for chunk in &chunks {
            assert!(chunk.content.len() <= MAX_CHUNK_BYTES + 200); // ±line tolerance
        }
    }

    // ── chunk_id stability tests (Patch R5) ──────────────────────────────────

    #[test]
    fn same_content_same_chunk_id() {
        let source_id = "src/lib.rs";
        let content = "fn foo() {}";
        let id1 = compute_chunk_id(source_id, content);
        let id2 = compute_chunk_id(source_id, content);
        assert_eq!(id1, id2, "same content must produce same chunk_id");
    }

    #[test]
    fn different_content_different_chunk_id() {
        let source_id = "src/lib.rs";
        let id1 = compute_chunk_id(source_id, "fn foo() {}");
        let id2 = compute_chunk_id(source_id, "fn bar() {}");
        assert_ne!(id1, id2, "changed content must produce different chunk_id");
    }

    #[test]
    fn different_source_different_chunk_id() {
        let content = "fn foo() {}";
        let id1 = compute_chunk_id("src/a.rs", content);
        let id2 = compute_chunk_id("src/b.rs", content);
        assert_ne!(
            id1, id2,
            "different source paths must produce different chunk_id"
        );
    }

    #[test]
    fn chunk_id_stable_across_chunk_text_calls() {
        // Chunk the same content twice; each chunk must have the same chunk_id.
        let source_id = "src/stable.rs";
        let content = "# A\nhello world\n# B\nmore content\n";
        let chunks1 = chunk_text(source_id, content, "text/markdown");
        let chunks2 = chunk_text(source_id, content, "text/markdown");
        assert_eq!(chunks1.len(), chunks2.len());
        for (c1, c2) in chunks1.iter().zip(chunks2.iter()) {
            assert_eq!(
                c1.chunk_id, c2.chunk_id,
                "identical content must give stable chunk_id"
            );
        }
    }

    #[test]
    fn moved_chunk_preserves_chunk_id_but_changes_index() {
        // Prepend a heading so original chunks shift to higher indices.
        // The original chunk content is unchanged so its chunk_id must stay the same.
        let source_id = "src/order.md";
        let original = "# B\noriginal content\n";
        let moved = "# A\nnew first section\n# B\noriginal content\n";
        let chunks_orig = chunk_text(source_id, original, "text/markdown");
        let chunks_moved = chunk_text(source_id, moved, "text/markdown");

        let orig_b = chunks_orig
            .iter()
            .find(|c| c.title.as_deref() == Some("B"))
            .unwrap();
        let moved_b = chunks_moved
            .iter()
            .find(|c| c.title.as_deref() == Some("B"))
            .unwrap();

        assert_eq!(
            orig_b.chunk_id, moved_b.chunk_id,
            "moved chunk keeps same chunk_id"
        );
        assert_ne!(
            orig_b.chunk_index, moved_b.chunk_index,
            "moved chunk gets a new chunk_index"
        );
    }

    #[test]
    fn changed_content_produces_new_chunk_id() {
        let source_id = "src/change.rs";
        let original_content = "fn original() {}";
        let changed_content = "fn changed() {}";
        let id1 = compute_chunk_id(source_id, original_content);
        let id2 = compute_chunk_id(source_id, changed_content);
        assert_ne!(id1, id2, "changing content must produce a new chunk_id");
    }

    #[test]
    fn chunk_id_is_hex_sha256() {
        let id = compute_chunk_id("path/to/file.rs", "some content");
        // SHA-256 produces 32 bytes → 64 hex chars.
        assert_eq!(id.len(), 64, "chunk_id must be a 64-char hex string");
        assert!(
            id.chars().all(|c| c.is_ascii_hexdigit()),
            "chunk_id must be hex"
        );
    }
}
