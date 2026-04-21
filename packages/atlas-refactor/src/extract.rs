//! Extract-function candidate detection (Phase 24.4).
//!
//! Detection only — no auto-apply. Scores contiguous blocks within function
//! bodies by size, variable discipline, and repetition patterns.

use atlas_core::{ExtractFunctionCandidate, Node, NodeKind, Result};

/// Minimum block length (in lines) to consider for extraction.
const MIN_BLOCK_LINES: u32 = 8;
/// A function must be at least this many lines to have candidates.
const MIN_FUNCTION_LINES: u32 = 20;

/// Detect extract-function candidates within a single source file.
///
/// Candidates are function nodes whose bodies are large enough to benefit from
/// extraction. Scoring is heuristic: size, free variable count estimate, and
/// simple repetition detection. No auto-apply is performed.
pub(crate) fn detect_candidates(
    file_path: &str,
    file_content: &str,
    file_nodes: &[Node],
) -> Result<Vec<ExtractFunctionCandidate>> {
    let lines: Vec<&str> = file_content.lines().collect();
    let mut candidates = Vec::new();

    // Collect functions & methods that are large enough to analyse.
    let large_fns: Vec<&Node> = file_nodes
        .iter()
        .filter(|n| {
            matches!(n.kind, NodeKind::Function | NodeKind::Method)
                && n.line_end.saturating_sub(n.line_start) >= MIN_FUNCTION_LINES
        })
        .collect();

    for func_node in large_fns {
        let fn_start = (func_node.line_start as usize).saturating_sub(1);
        let fn_end = (func_node.line_end as usize).min(lines.len());
        if fn_start >= fn_end {
            continue;
        }
        let fn_lines = &lines[fn_start..fn_end];

        // Slide a window of MIN_BLOCK_LINES through the function body.
        let window = MIN_BLOCK_LINES as usize;
        if fn_lines.len() <= window {
            continue;
        }

        // We score at most one candidate per function: the largest contiguous
        // non-empty block that doesn't contain the function signature.
        // Simple heuristic: find the longest non-blank run in the body
        // (skip first 2 lines which typically hold the signature + `{`).
        let body_offset = 2.min(fn_lines.len());
        let body = &fn_lines[body_offset..];

        let (best_start, best_len) = longest_non_blank_run(body);
        if best_len < window {
            continue;
        }

        // Absolute line numbers (1-based).
        let abs_start = (fn_start + body_offset + best_start + 1) as u32;
        let abs_end = (fn_start + body_offset + best_start + best_len) as u32;

        let block_lines = &body[best_start..best_start + best_len];

        if !has_limited_side_effect_boundaries(block_lines) {
            continue;
        }

        let (proposed_inputs, proposed_outputs) = estimate_io(block_lines, fn_lines);
        let (score, reasons) = score_candidate(
            best_len as u32,
            &proposed_inputs,
            &proposed_outputs,
            fn_lines,
            block_lines,
        );

        candidates.push(ExtractFunctionCandidate {
            file_path: file_path.to_string(),
            line_start: abs_start,
            line_end: abs_end,
            proposed_inputs,
            proposed_outputs,
            difficulty_score: score,
            score_reasons: reasons,
        });
    }

    // Sort: highest scoring (easiest to extract) first.
    candidates.sort_by(|a, b| {
        b.difficulty_score
            .partial_cmp(&a.difficulty_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(candidates)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find the longest contiguous run of non-blank lines in `lines`.
///
/// Returns `(start_index, length)`.
fn longest_non_blank_run(lines: &[&str]) -> (usize, usize) {
    let (mut best_start, mut best_len) = (0, 0);
    let (mut cur_start, mut cur_len) = (0, 0);

    for (i, &line) in lines.iter().enumerate() {
        if !line.trim().is_empty() {
            if cur_len == 0 {
                cur_start = i;
            }
            cur_len += 1;
            if cur_len > best_len {
                best_len = cur_len;
                best_start = cur_start;
            }
        } else {
            cur_len = 0;
        }
    }
    (best_start, best_len)
}

/// Estimate input and output binding names used in `block`.
///
/// Heuristic: collect simple identifiers (ASCII word) appearing in the block
/// that also appear outside the block in the function body.
fn estimate_io<'a>(block: &[&'a str], fn_body: &[&'a str]) -> (Vec<String>, Vec<String>) {
    let block_idents = collect_idents(block);
    let outer_idents = collect_idents(fn_body);

    // Inputs: identifiers used in block that also appear in outer lines.
    let inputs: Vec<String> = block_idents
        .iter()
        .filter(|i| outer_idents.contains(*i) && !is_keyword(i))
        .take(6)
        .cloned()
        .collect();

    // Outputs: identifiers assigned (`let name =` or `name =`) in block
    // that appear in outer lines after the block — approximated as the set
    // of identifiers following `let` in the block.
    let outputs: Vec<String> = block
        .iter()
        .flat_map(|l| {
            let t = l.trim();
            if let Some(rest) = t
                .strip_prefix("let ")
                .or_else(|| t.strip_prefix("let mut "))
            {
                // Take the first word as the bound name.
                let name: String = rest.chars().take_while(|c| is_ident_char(*c)).collect();
                if !name.is_empty() && outer_idents.contains(&name) {
                    return vec![name];
                }
            }
            vec![]
        })
        .take(4)
        .collect();

    (inputs, outputs)
}

fn collect_idents(lines: &[&str]) -> std::collections::HashSet<String> {
    let mut set = std::collections::HashSet::new();
    for line in lines {
        let mut cur = String::new();
        for c in line.chars() {
            if is_ident_char(c) {
                cur.push(c);
            } else {
                if cur.len() > 1 && !is_keyword(&cur) {
                    set.insert(cur.clone());
                }
                cur.clear();
            }
        }
        if cur.len() > 1 {
            set.insert(cur);
        }
    }
    set
}

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Rough keyword set to suppress from IO estimates.
fn is_keyword(s: &str) -> bool {
    matches!(
        s,
        "let"
            | "mut"
            | "fn"
            | "if"
            | "else"
            | "for"
            | "while"
            | "loop"
            | "match"
            | "return"
            | "use"
            | "pub"
            | "mod"
            | "impl"
            | "struct"
            | "enum"
            | "trait"
            | "type"
            | "const"
            | "static"
            | "self"
            | "Self"
            | "true"
            | "false"
            | "in"
            | "as"
            | "where"
            | "async"
            | "await"
            | "move"
            | "ref"
            | "dyn"
            | "box"
            | "break"
            | "continue"
            | "crate"
            | "super"
            | "extern"
            | "unsafe"
            | "yield"
    )
}

/// Score a candidate block. Higher = better extraction candidate.
///
/// Factors:
/// - long block boost (+block_len / 10)
/// - low free-variable boost if inputs <= 3 (+2)
/// - low output count boost if outputs <= 2 (+2)
/// - repeated-pattern boost if a similar block exists in the function (+3)
fn score_candidate(
    block_len: u32,
    inputs: &[String],
    outputs: &[String],
    fn_body: &[&str],
    block: &[&str],
) -> (f64, Vec<String>) {
    let mut score = block_len as f64 / 10.0;
    let mut reasons = Vec::new();

    reasons.push(format!("block length: {block_len} lines"));

    if inputs.len() <= 3 {
        score += 2.0;
        reasons.push("low free-variable count".into());
    } else {
        reasons.push(format!(
            "{} free variables (higher complexity)",
            inputs.len()
        ));
    }

    if outputs.len() <= 2 {
        score += 2.0;
        reasons.push("few output bindings".into());
    }

    if has_repeated_pattern(fn_body, block) {
        score += 3.0;
        reasons.push("repeated block pattern detected".into());
    }

    let control_flow_complexity = estimate_control_flow_complexity(block);
    if control_flow_complexity <= 1 {
        score += 1.5;
        reasons.push("low control-flow complexity".into());
    } else {
        reasons.push(format!(
            "control-flow complexity: {control_flow_complexity}"
        ));
    }

    (score, reasons)
}

fn has_limited_side_effect_boundaries(block: &[&str]) -> bool {
    side_effect_boundary_count(block) <= 1
}

fn side_effect_boundary_count(block: &[&str]) -> usize {
    block
        .iter()
        .filter(|line| {
            let trimmed = line.trim();
            trimmed.starts_with("return ")
                || trimmed == "return"
                || trimmed.starts_with("break")
                || trimmed.starts_with("continue")
                || trimmed.contains("await")
                || trimmed.contains("yield")
                || trimmed.contains("panic!")
                || trimmed.contains("println!")
                || trimmed.contains("eprintln!")
        })
        .count()
}

fn estimate_control_flow_complexity(block: &[&str]) -> usize {
    block
        .iter()
        .map(|line| {
            let trimmed = line.trim();
            usize::from(trimmed.starts_with("if ") || trimmed.starts_with("if("))
                + usize::from(trimmed.starts_with("match "))
                + usize::from(trimmed.starts_with("for "))
                + usize::from(trimmed.starts_with("while "))
                + usize::from(trimmed.starts_with("loop"))
                + trimmed.matches("&&").count()
                + trimmed.matches("||").count()
        })
        .sum()
}

/// Rough repeated-pattern detection: look for another window in `fn_body`
/// that shares >50% of the non-blank lines with `block`.
fn has_repeated_pattern(fn_body: &[&str], block: &[&str]) -> bool {
    if block.is_empty() || fn_body.len() < block.len() * 2 {
        return false;
    }
    let block_set: std::collections::HashSet<&str> = block
        .iter()
        .copied()
        .filter(|l| !l.trim().is_empty())
        .collect();
    let threshold = (block_set.len() as f64 * 0.5).ceil() as usize;

    let window = block.len();
    for i in 0..fn_body.len().saturating_sub(window) {
        let slice = &fn_body[i..i + window];
        // Skip if this is exactly the same slice (same block).
        if slice.as_ptr() == block.as_ptr() {
            continue;
        }
        let overlap = slice
            .iter()
            .filter(|l| !l.trim().is_empty() && block_set.contains(*l))
            .count();
        if overlap >= threshold && threshold > 0 {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn longest_non_blank_run_basic() {
        let lines = ["a", "b", "", "c", "d", "e", "f"];
        let (start, len) = longest_non_blank_run(&lines);
        assert_eq!(len, 4); // c, d, e, f
        assert_eq!(start, 3);
    }

    #[test]
    fn detect_candidates_empty_file_no_crash() {
        let result = detect_candidates("f.rs", "", &[]);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn score_candidate_low_input_boost() {
        let block: Vec<&str> = (0..10).map(|_| "let x = 1;").collect();
        let (score, reasons) = score_candidate(10, &["a".to_string()], &[], &block, &block);
        assert!(score >= 3.0, "score={score}");
        assert!(reasons.iter().any(|r| r.contains("free-variable")));
    }

    #[test]
    fn control_flow_complexity_boost_recorded() {
        let block = ["let total = a + b;", "let output = total * 2;"];
        let (_, reasons) = score_candidate(8, &["a".to_string()], &[], &block, &block);
        assert!(
            reasons
                .iter()
                .any(|reason| reason.contains("low control-flow complexity"))
        );
    }

    #[test]
    fn side_effect_boundaries_detected() {
        let block = ["println!(\"hello\");", "return value;"];
        assert!(!has_limited_side_effect_boundaries(&block));
        assert!(side_effect_boundary_count(&block) >= 2);
    }
}
