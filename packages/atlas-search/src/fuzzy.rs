use super::*;

// ---------------------------------------------------------------------------
// Fuzzy matching
// ---------------------------------------------------------------------------

/// Compute the Levenshtein edit distance between two strings, capped at
/// `cap + 1` so callers can reject clearly dissimilar strings early.
pub(super) fn edit_distance(a: &str, b: &str, cap: usize) -> usize {
    // Cheap bound: a length difference above the cap can never be within it.
    if a.chars().count().abs_diff(b.chars().count()) > cap {
        return cap + 1;
    }
    let distance = strsim::levenshtein(a, b);
    if distance > cap { cap + 1 } else { distance }
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
