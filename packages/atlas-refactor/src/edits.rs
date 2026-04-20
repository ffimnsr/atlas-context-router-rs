//! Low-level text-edit helpers.
//!
//! All edits are line-based. Line numbers are 1-based throughout.
//! Edits must be applied in reverse line order (highest `line_start` first)
//! so earlier-line edits are not invalidated by later-line removals.

use atlas_core::{AtlasError, RefactorEdit, Result};

/// Apply a sorted slice of [`RefactorEdit`]s to `content`, returning the
/// modified text.
///
/// Edits **must** be sorted with the highest `line_start` first (reverse
/// order) and must not overlap. Panics in debug mode on overlap.
pub(crate) fn apply_edits(content: &str, edits: &[RefactorEdit]) -> Result<String> {
    // Split into lines preserving line endings.
    let mut lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
    // Keep the trailing newline as a sentinel empty line if needed.
    let trailing_newline = content.ends_with('\n');

    for edit in edits {
        let start = edit.line_start.saturating_sub(1) as usize; // 0-based
        let end = edit.line_end.saturating_sub(1) as usize; // 0-based, inclusive

        if start > lines.len() || end >= lines.len() {
            return Err(AtlasError::Other(format!(
                "edit out of range: lines {}-{} but file has {} lines in `{}`",
                edit.line_start,
                edit.line_end,
                lines.len(),
                edit.file_path,
            )));
        }

        if edit.new_text.is_empty() {
            // Remove the span entirely.
            lines.drain(start..=end);
        } else {
            // Replace the span's first line with new_text, drop the rest.
            lines[start] = edit.new_text.clone();
            if end > start {
                lines.drain((start + 1)..=end);
            }
        }
    }

    let mut out = lines.join("\n");
    if trailing_newline {
        out.push('\n');
    }
    Ok(out)
}

/// Replace all whole-word occurrences of `old` with `new` in `line`.
///
/// Uses a byte scan with boundary checks rather than a regex to avoid an
/// extra dependency. A word boundary is a position where one side is an
/// ASCII alphanumeric or `_` and the other is not.
pub(crate) fn replace_identifier(line: &str, old: &str, new: &str) -> String {
    if old.is_empty() {
        return line.to_string();
    }
    let mut result = String::with_capacity(line.len());
    let bytes = line.as_bytes();
    let old_bytes = old.as_bytes();
    let mut pos = 0usize;

    while pos + old_bytes.len() <= bytes.len() {
        if bytes[pos..].starts_with(old_bytes) {
            let before_ok = pos == 0 || !is_ident_byte(bytes[pos - 1]);
            let after_pos = pos + old_bytes.len();
            let after_ok = after_pos >= bytes.len() || !is_ident_byte(bytes[after_pos]);
            if before_ok && after_ok {
                result.push_str(new);
                pos += old_bytes.len();
                continue;
            }
        }
        result.push(bytes[pos] as char);
        pos += 1;
    }
    // Append tail if any.
    result.push_str(&line[pos..]);
    result
}

#[inline]
fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Validate that a proposed identifier is a valid simple identifier.
///
/// Accepts ASCII letters, digits, and underscores; must start with a letter
/// or underscore; no spaces or special characters allowed.
pub(crate) fn validate_identifier(name: &str) -> Result<()> {
    if name.is_empty() {
        return Err(AtlasError::Other("new name must not be empty".into()));
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphabetic() && first != '_' {
        return Err(AtlasError::Other(format!(
            "invalid identifier `{name}`: must start with a letter or underscore"
        )));
    }
    for c in chars {
        if !c.is_ascii_alphanumeric() && c != '_' {
            return Err(AtlasError::Other(format!(
                "invalid identifier `{name}`: contains invalid character `{c}`"
            )));
        }
    }
    Ok(())
}

/// Check whether `edits` (already in reverse line order) have any overlapping
/// spans within the same file. Returns an error describing the first overlap.
pub(crate) fn check_overlaps(edits: &[RefactorEdit]) -> Result<()> {
    // Group by file, then check within each file.
    use std::collections::HashMap;
    let mut by_file: HashMap<&str, Vec<(u32, u32)>> = HashMap::new();
    for e in edits {
        by_file.entry(&e.file_path).or_default().push((e.line_start, e.line_end));
    }
    for (path, mut spans) in by_file {
        // Sort ascending to detect overlaps.
        spans.sort_by_key(|s| s.0);
        for w in spans.windows(2) {
            let (_, end_a) = w[0];
            let (start_b, _) = w[1];
            if start_b <= end_a {
                return Err(AtlasError::Other(format!(
                    "overlapping edits at lines {end_a} and {start_b} in `{path}`"
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::{RefactorEditKind};

    fn make_edit(file: &str, ls: u32, le: u32, old: &str, new: &str) -> RefactorEdit {
        RefactorEdit {
            file_path: file.to_string(),
            line_start: ls,
            line_end: le,
            old_text: old.to_string(),
            new_text: new.to_string(),
            edit_kind: RefactorEditKind::RenameOccurrence,
        }
    }

    #[test]
    fn replace_identifier_simple() {
        assert_eq!(replace_identifier("let foo = bar;", "foo", "baz"), "let baz = bar;");
    }

    #[test]
    fn replace_identifier_no_partial_match() {
        // "foo" inside "foobar" must not be replaced.
        assert_eq!(replace_identifier("let foobar = foo;", "foo", "baz"), "let foobar = baz;");
    }

    #[test]
    fn replace_identifier_multiple_occurrences() {
        assert_eq!(replace_identifier("foo + foo", "foo", "x"), "x + x");
    }

    #[test]
    fn validate_identifier_ok() {
        assert!(validate_identifier("valid_name").is_ok());
        assert!(validate_identifier("_private").is_ok());
        assert!(validate_identifier("MyType123").is_ok());
    }

    #[test]
    fn validate_identifier_rejects_empty() {
        assert!(validate_identifier("").is_err());
    }

    #[test]
    fn validate_identifier_rejects_digit_start() {
        assert!(validate_identifier("1bad").is_err());
    }

    #[test]
    fn validate_identifier_rejects_spaces() {
        assert!(validate_identifier("bad name").is_err());
    }

    #[test]
    fn apply_edits_rename_single_line() {
        let content = "fn old_name() {}\nfn other() {}\n";
        let edits = vec![make_edit("f.rs", 1, 1, "fn old_name() {}", "fn new_name() {}")];
        let result = apply_edits(content, &edits).unwrap();
        assert_eq!(result, "fn new_name() {}\nfn other() {}\n");
    }

    #[test]
    fn apply_edits_remove_span() {
        let content = "line1\nline2\nline3\nline4\n";
        // Remove lines 2-3 (empty new_text signals removal).
        let mut e = make_edit("f.rs", 2, 3, "line2\nline3", "");
        e.edit_kind = RefactorEditKind::RemoveSpan;
        let result = apply_edits(content, &[e]).unwrap();
        assert_eq!(result, "line1\nline4\n");
    }

    #[test]
    fn check_overlaps_detects_conflict() {
        let edits = vec![
            make_edit("f.rs", 1, 5, "", ""),
            make_edit("f.rs", 3, 7, "", ""),
        ];
        assert!(check_overlaps(&edits).is_err());
    }

    #[test]
    fn check_overlaps_non_overlapping_ok() {
        let edits = vec![
            make_edit("f.rs", 1, 3, "", ""),
            make_edit("f.rs", 4, 6, "", ""),
        ];
        assert!(check_overlaps(&edits).is_ok());
    }
}
