use super::*;

// ---------------------------------------------------------------------------
// Fuzzy matching
// ---------------------------------------------------------------------------

/// Compute the edit distance (Levenshtein) between two strings, capped at
/// `cap + 1` so we can bail out early for clearly dissimilar strings.
pub(super) fn edit_distance(a: &str, b: &str, cap: usize) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let m = a.len();
    let n = b.len();

    // Quick bounds check — if length difference alone exceeds cap, bail.
    if m.abs_diff(n) > cap {
        return cap + 1;
    }

    // Two-row DP (space-efficient).
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];

    for i in 1..=m {
        curr[0] = i;
        let mut row_min = i;
        for j in 1..=n {
            curr[j] = if a[i - 1] == b[j - 1] {
                prev[j - 1]
            } else {
                1 + prev[j - 1].min(prev[j]).min(curr[j - 1])
            };
            row_min = row_min.min(curr[j]);
        }
        // Early exit if entire row exceeds cap.
        if row_min > cap {
            return cap + 1;
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

/// Return the edit-distance threshold for a query of length `len`.
///
/// Short queries need tighter matching to avoid noise:
///   len ≤ 3 → 0 (exact only)
///   len ≤ 5 → 1
///   len ≤ 8 → 2
///   len > 8 → 3
pub(super) fn fuzzy_threshold(len: usize) -> usize {
    match len {
        0..=3 => 0,
        4..=5 => 1,
        6..=8 => 2,
        _ => 3,
    }
}

pub(super) fn is_non_code_language(language: &str) -> bool {
    matches!(
        language.to_ascii_lowercase().as_str(),
        "markdown" | "md" | "json" | "toml" | "yaml" | "yml"
    )
}

pub(super) fn fuzzy_typo_details(
    node: &atlas_core::Node,
    q_lower: &str,
    fuzzy_cap: usize,
    primitives: &GraphSearchRankingPrimitives,
) -> Option<(usize, f64)> {
    if fuzzy_cap == 0 {
        return None;
    }

    let dist = edit_distance(q_lower, &node.name.to_lowercase(), fuzzy_cap);
    if dist > fuzzy_cap {
        return None;
    }

    let distance_bonus = primitives.fuzzy_distance_bonus(dist);

    let kind_bonus: f64 = match node.kind {
        NodeKind::Function | NodeKind::Method => 10.0,
        NodeKind::Class
        | NodeKind::Struct
        | NodeKind::Trait
        | NodeKind::Interface
        | NodeKind::Enum
        | NodeKind::Constant
        | NodeKind::Variable
        | NodeKind::Test => 8.0,
        NodeKind::Module | NodeKind::Package => 5.0,
        NodeKind::Import => -4.0,
        NodeKind::File => -8.0,
    };

    let language_penalty: f64 = if is_non_code_language(&node.language) {
        -6.0
    } else {
        0.0
    };

    Some((
        dist,
        (distance_bonus + kind_bonus + language_penalty).max(0.0_f64),
    ))
}
