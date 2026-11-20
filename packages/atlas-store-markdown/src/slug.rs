//! Deterministic slugs and content-derived ids.

use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

/// Fallback slug for titles with no ASCII alphanumeric content.
pub const UNTITLED: &str = "untitled";

const MAX_SLUG_LEN: usize = 60;

/// Slugify a topic or title into a stable directory/file segment.
///
/// NFKD normalization strips accents; ASCII alphanumerics are lowercased;
/// every other run of characters becomes a single `-`. Unicode-only titles
/// fall back to [`UNTITLED`] deterministically.
pub fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut pending_separator = false;
    for normalized in input.nfkd() {
        let mut emitted = false;
        for ch in normalized.to_lowercase() {
            if ch.is_ascii_alphanumeric() {
                if pending_separator && !out.is_empty() {
                    out.push('-');
                }
                pending_separator = false;
                out.push(ch);
                emitted = true;
            }
        }
        if !emitted {
            pending_separator = true;
        }
    }
    let trimmed = out.trim_matches('-');
    let capped: String = trimmed.chars().take(MAX_SLUG_LEN).collect();
    let capped = capped.trim_end_matches('-').to_string();
    if capped.is_empty() {
        UNTITLED.to_string()
    } else {
        capped
    }
}

/// Derive a deterministic hex id from length-prefixed identity parts.
///
/// Identical parts always produce the same id; any part change produces a
/// different id.
pub fn derive_id(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_le_bytes());
        hasher.update(part.as_bytes());
    }
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_collisions_are_expected_and_stable() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("hello world"), "hello-world");
        assert_eq!(slugify("Hello   World"), "hello-world");
        assert_eq!(slugify("  padded  "), "padded");
    }

    #[test]
    fn unicode_and_empty_titles_fall_back() {
        assert_eq!(slugify(""), UNTITLED);
        assert_eq!(slugify("!!!"), UNTITLED);
        assert_eq!(slugify("\u{8a18}\u{61b6}"), UNTITLED);
        assert_eq!(slugify("Caf\u{e9} Notes"), "cafe-notes");
        assert_eq!(slugify("\u{1f600} emoji \u{1f680}"), "emoji");
    }

    #[test]
    fn slug_length_is_capped() {
        let long = "a".repeat(500);
        let slug = slugify(&long);
        assert_eq!(slug.len(), MAX_SLUG_LEN);
    }

    #[test]
    fn ids_are_deterministic_and_distinct() {
        let first = derive_id(&["topic", "title", "2026-01-01"]);
        let second = derive_id(&["topic", "title", "2026-01-01"]);
        let third = derive_id(&["topic", "title", "2026-01-02"]);
        assert_eq!(first, second);
        assert_ne!(first, third);
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|ch| ch.is_ascii_hexdigit()));
    }

    #[test]
    fn length_prefixing_prevents_boundary_collisions() {
        assert_ne!(derive_id(&["ab", "c"]), derive_id(&["a", "bc"]));
    }
}
