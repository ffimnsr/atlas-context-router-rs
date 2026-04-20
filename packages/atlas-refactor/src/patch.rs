//! Minimal unified-diff generation for refactor patch previews.
//!
//! Implements a standard LCS-based line diff so the output is stable and
//! idiomatic. No external crates required.

// ---------------------------------------------------------------------------
// Internal diff types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum DiffOp<'a> {
    Equal(&'a str),
    Remove(&'a str),
    Insert(&'a str),
}

struct AnnotatedLine<'a> {
    op: DiffOp<'a>,
    old_line: usize,
    new_line: usize,
}

// ---------------------------------------------------------------------------
// Core diff algorithm
// ---------------------------------------------------------------------------

/// Compute an LCS-based edit script between `old` and `new` line slices.
fn diff_lines<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<DiffOp<'a>> {
    let m = old.len();
    let n = new.len();

    // DP table — forward pass.
    let mut dp = vec![vec![0usize; n + 1]; m + 1];
    for i in (0..m).rev() {
        for j in (0..n).rev() {
            dp[i][j] = if old[i] == new[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    // Traceback.
    let mut ops = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < m && j < n {
        if old[i] == new[j] {
            ops.push(DiffOp::Equal(old[i]));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            ops.push(DiffOp::Remove(old[i]));
            i += 1;
        } else {
            ops.push(DiffOp::Insert(new[j]));
            j += 1;
        }
    }
    while i < m {
        ops.push(DiffOp::Remove(old[i]));
        i += 1;
    }
    while j < n {
        ops.push(DiffOp::Insert(new[j]));
        j += 1;
    }
    ops
}

/// Annotate diff ops with real 1-based line numbers and group into hunks.
fn build_hunks<'a>(ops: &[DiffOp<'a>], context: usize) -> Vec<Vec<AnnotatedLine<'a>>> {
    // Annotate every op with old/new line numbers.
    let mut annotated: Vec<(DiffOp<'a>, usize, usize)> = Vec::with_capacity(ops.len());
    let (mut old_ln, mut new_ln) = (1usize, 1usize);
    for op in ops {
        let (ol, nl) = (old_ln, new_ln);
        match op {
            DiffOp::Equal(_) => { old_ln += 1; new_ln += 1; }
            DiffOp::Remove(_) => { old_ln += 1; }
            DiffOp::Insert(_) => { new_ln += 1; }
        }
        annotated.push((op.clone(), ol, nl));
    }

    // Positions of changed ops.
    let changed: Vec<usize> = annotated
        .iter()
        .enumerate()
        .filter(|(_, (op, _, _))| !matches!(op, DiffOp::Equal(_)))
        .map(|(i, _)| i)
        .collect();

    if changed.is_empty() {
        return vec![];
    }

    // Merge into ranges with surrounding context.
    let last_idx = annotated.len().saturating_sub(1);
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut hunk_start = changed[0].saturating_sub(context);
    let mut hunk_end = (changed[0] + context).min(last_idx);

    for &idx in &changed[1..] {
        let ctx_start = idx.saturating_sub(context);
        let ctx_end = (idx + context).min(last_idx);
        if ctx_start <= hunk_end + 1 {
            hunk_end = hunk_end.max(ctx_end);
        } else {
            ranges.push((hunk_start, hunk_end));
            hunk_start = ctx_start;
            hunk_end = ctx_end;
        }
    }
    ranges.push((hunk_start, hunk_end));

    ranges
        .into_iter()
        .map(|(s, e)| {
            annotated[s..=e]
                .iter()
                .map(|(op, ol, nl)| AnnotatedLine { op: op.clone(), old_line: *ol, new_line: *nl })
                .collect()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Generate a unified diff string between `old` and `new` content for `path`.
///
/// Returns an empty string when `old == new`. Uses 3-line context.
pub(crate) fn unified_diff_annotated(path: &str, old: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let ops = diff_lines(&old_lines, &new_lines);
    if ops.iter().all(|op| matches!(op, DiffOp::Equal(_))) {
        return String::new();
    }

    let hunks = build_hunks(&ops, 3);
    if hunks.is_empty() {
        return String::new();
    }

    let mut out = String::new();
    out.push_str(&format!("--- a/{path}\n"));
    out.push_str(&format!("+++ b/{path}\n"));

    for hunk in &hunks {
        let old_start = hunk.first().map(|l| l.old_line).unwrap_or(1);
        let new_start = hunk.first().map(|l| l.new_line).unwrap_or(1);
        let old_count = hunk.iter().filter(|l| !matches!(l.op, DiffOp::Insert(_))).count();
        let new_count = hunk.iter().filter(|l| !matches!(l.op, DiffOp::Remove(_))).count();

        out.push_str(&format!(
            "@@ -{old_start},{old_count} +{new_start},{new_count} @@\n"
        ));
        for line in hunk {
            match &line.op {
                DiffOp::Equal(s) => out.push_str(&format!(" {s}\n")),
                DiffOp::Remove(s) => out.push_str(&format!("-{s}\n")),
                DiffOp::Insert(s) => out.push_str(&format!("+{s}\n")),
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_identical_content_empty() {
        let d = unified_diff_annotated("f.rs", "hello\nworld\n", "hello\nworld\n");
        assert!(d.is_empty());
    }

    #[test]
    fn diff_single_line_change() {
        let old = "fn foo() {}\nfn bar() {}\n";
        let new = "fn foo() {}\nfn baz() {}\n";
        let d = unified_diff_annotated("f.rs", old, new);
        assert!(d.contains("-fn bar()"), "expected removal: {d}");
        assert!(d.contains("+fn baz()"), "expected addition: {d}");
    }

    #[test]
    fn diff_header_present() {
        let d = unified_diff_annotated("src/lib.rs", "a\n", "b\n");
        assert!(d.starts_with("--- a/src/lib.rs\n"), "bad header: {d}");
        assert!(d.contains("+++ b/src/lib.rs\n"), "bad header: {d}");
    }

    #[test]
    fn diff_removal_only() {
        let old = "line1\nline2\nline3\n";
        let new = "line1\nline3\n";
        let d = unified_diff_annotated("f.rs", old, new);
        assert!(d.contains("-line2"), "expected removal: {d}");
    }
}
