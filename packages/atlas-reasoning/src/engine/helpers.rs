use atlas_core::{
    ConfidenceTier, Edge, EdgeKind, ImpactClass, Node, NodeKind, Result, RiskLevel, SafetyBand,
};
use atlas_store_sqlite::Store;

/// Edge query cap for per-node lookups.
pub(super) const EDGE_QUERY_LIMIT: usize = 500;

pub(super) type ImpactNodeInfo = (Node, u32, Option<EdgeKind>);
pub(super) type BfsImpact = (Vec<ImpactNodeInfo>, Vec<Edge>);

/// Simple-name patterns that are always suppressed as entrypoints even when
/// they have no inbound edges in the graph.
pub(super) const ENTRYPOINT_NAMES: &[&str] = &[
    "main",
    "new",
    "init",
    "setup",
    "configure",
    "run",
    "start",
    "handler",
    "middleware",
];

pub(super) struct RiskInputs {
    pub(super) fan_in: usize,
    pub(super) fan_out: usize,
    pub(super) is_public: bool,
    pub(super) test_adj: bool,
    pub(super) cross_module: bool,
    pub(super) cross_package: bool,
    pub(super) unresolved: usize,
    pub(super) impacted_file_count: usize,
}

pub(super) fn classify_impact(
    _node: &Node,
    depth: u32,
    edge_kind: Option<EdgeKind>,
) -> ImpactClass {
    match edge_kind {
        Some(EdgeKind::Calls | EdgeKind::Imports | EdgeKind::Tests | EdgeKind::TestedBy) => {
            ImpactClass::Definite
        }
        Some(EdgeKind::Implements | EdgeKind::Extends) => ImpactClass::Definite,
        Some(EdgeKind::References) if depth <= 1 => ImpactClass::Definite,
        Some(EdgeKind::References) => ImpactClass::Probable,
        Some(EdgeKind::Contains | EdgeKind::Defines) => ImpactClass::Weak,
        None if depth == 0 => ImpactClass::Definite,
        _ => ImpactClass::Weak,
    }
}

pub(super) fn is_public_node(node: &Node) -> bool {
    let mods = node.modifiers.as_deref().unwrap_or("");
    mods.contains("pub") || mods.contains("export") || mods.contains("public")
}

pub(super) fn same_module(a: &Node, b: &Node) -> bool {
    let a_dir = parent_dir(&a.file_path);
    let b_dir = parent_dir(&b.file_path);
    a_dir == b_dir
}

fn parent_dir(path: &str) -> &str {
    match path.rfind('/') {
        Some(idx) => &path[..idx],
        None => "",
    }
}

fn different_package(a: &str, b: &str) -> bool {
    let a_top = a.split('/').next().unwrap_or("");
    let b_top = b.split('/').next().unwrap_or("");
    a_top != b_top && !a_top.is_empty() && !b_top.is_empty()
}

pub(super) fn file_paths_cross_package(store: &Store, a: &str, b: &str) -> Result<bool> {
    let owner_a = store.file_owner_id(a)?;
    let owner_b = store.file_owner_id(b)?;
    Ok(match (owner_a, owner_b) {
        (Some(owner_a), Some(owner_b)) => owner_a != owner_b,
        _ => different_package(a, b),
    })
}

pub(super) fn normalize_qn_kind_tokens(qname: &str) -> String {
    let Some(after_file) = qname.find("::") else {
        return qname.to_owned();
    };
    let (file_part, rest) = qname.split_at(after_file);
    let rest = &rest[2..];

    let (kind_token, symbol_rest) = if let Some(pos) = rest.find("::") {
        (&rest[..pos], &rest[pos..])
    } else {
        return qname.to_owned();
    };

    let kind_lower = kind_token.to_ascii_lowercase();
    let canonical_kind = match kind_lower.as_str() {
        "function" | "func" => "fn",
        "meth" => "method",
        "constant" => "const",
        other => other,
    };

    if canonical_kind == kind_token {
        return qname.to_owned();
    }
    format!("{file_part}::{canonical_kind}{symbol_rest}")
}

pub(super) fn dead_code_reasons(node: &Node) -> (Vec<String>, ConfidenceTier, Vec<String>) {
    let mut reasons = vec!["no inbound call, reference, or import edges".to_owned()];
    let mut blockers: Vec<String> = vec![];

    if matches!(
        node.kind,
        NodeKind::Function | NodeKind::Method | NodeKind::Constant
    ) {
        reasons.push(format!("{} with zero callers", node.kind.as_str()));
    }

    let certainty = match node.kind {
        NodeKind::Constant | NodeKind::Variable => {
            blockers.push("may be used via reflection or dynamic dispatch".to_owned());
            ConfidenceTier::Medium
        }
        NodeKind::Class
        | NodeKind::Struct
        | NodeKind::Enum
        | NodeKind::Trait
        | NodeKind::Interface => {
            blockers.push("may be instantiated via reflection, macro, or config".to_owned());
            ConfidenceTier::Medium
        }
        _ => ConfidenceTier::High,
    };

    (reasons, certainty, blockers)
}

pub(super) fn compute_safety_score(
    _node: &Node,
    fan_in: usize,
    fan_out: usize,
    linked_tests: usize,
    is_public: bool,
    cross_module_callers: usize,
    unresolved: usize,
) -> (f64, SafetyBand, Vec<String>, Vec<String>) {
    let mut score: f64 = 1.0;
    let mut reasons: Vec<String> = vec![];
    let mut validations: Vec<String> = vec![];

    if is_public {
        score -= 0.25;
        reasons.push("public/exported API — breaking change risk".to_owned());
        validations.push("run all integration tests after refactor".to_owned());
    }

    let fi_penalty = (fan_in as f64 * 0.04).min(0.3);
    if fan_in > 5 {
        score -= fi_penalty;
        reasons.push(format!("high fan-in: {fan_in} inbound references"));
        validations.push("search all call sites before changing signature".to_owned());
    } else if fan_in > 0 {
        score -= fi_penalty;
    }

    if cross_module_callers > 0 {
        let penalty = (cross_module_callers as f64 * 0.05).min(0.25);
        score -= penalty;
        reasons.push(format!("{cross_module_callers} cross-module callers"));
        validations.push("verify cross-module consumers compile after change".to_owned());
    }

    if fan_out > 10 {
        score -= 0.1;
        reasons.push(format!("high fan-out: {fan_out} outbound references"));
    }

    if linked_tests == 0 {
        score -= 0.15;
        reasons.push("no linked tests".to_owned());
        validations.push("add tests before refactoring".to_owned());
    } else if linked_tests >= 3 {
        score += 0.05;
        reasons.push(format!("strong test adjacency ({linked_tests} tests)"));
    }

    if unresolved > 0 {
        let penalty = (unresolved as f64 * 0.03).min(0.2);
        score -= penalty;
        reasons.push(format!(
            "{unresolved} low-confidence/unresolved edges — dynamic usage risk"
        ));
        validations.push("verify no dynamic dispatch before removing".to_owned());
    }

    score = score.clamp(0.0, 1.0);

    let band = if score >= 0.7 {
        SafetyBand::Safe
    } else if score >= 0.4 {
        SafetyBand::Caution
    } else {
        SafetyBand::Risky
    };

    (score, band, reasons, validations)
}

pub(super) fn rename_risk(
    node: &Node,
    reference_count: usize,
    manual_flags: usize,
    has_collisions: bool,
) -> RiskLevel {
    if has_collisions || (is_public_node(node) && reference_count > 20) {
        return RiskLevel::High;
    }
    if manual_flags > 0 || reference_count > 10 || is_public_node(node) {
        return RiskLevel::Medium;
    }
    RiskLevel::Low
}

pub(super) fn compute_risk_level(_node: &Node, inputs: RiskInputs) -> (RiskLevel, Vec<String>) {
    let mut score: i32 = 0;
    let mut factors: Vec<String> = vec![];

    if inputs.is_public {
        score += 3;
        factors.push("public/exported API touched".to_owned());
    }
    if !inputs.test_adj {
        score += 2;
        factors.push("no test adjacency".to_owned());
    }
    if inputs.cross_package {
        score += 3;
        factors.push("cross-package impact".to_owned());
    } else if inputs.cross_module {
        score += 2;
        factors.push("cross-module impact".to_owned());
    }
    if inputs.fan_in > 10 {
        score += 2;
        factors.push(format!("high inbound caller count ({})", inputs.fan_in));
    } else if inputs.fan_in > 3 {
        score += 1;
    }
    if inputs.unresolved > 0 {
        score += 1;
        factors.push(format!(
            "{} unresolved/dynamic references",
            inputs.unresolved
        ));
    }
    if inputs.fan_out > 15 {
        score += 1;
        factors.push(format!("high dependency fan-out ({})", inputs.fan_out));
    }
    if inputs.impacted_file_count > 10 {
        score += 1;
        factors.push(format!("{} impacted files", inputs.impacted_file_count));
    }

    let level = if score >= 8 {
        RiskLevel::Critical
    } else if score >= 5 {
        RiskLevel::High
    } else if score >= 2 {
        RiskLevel::Medium
    } else {
        RiskLevel::Low
    };

    (level, factors)
}

pub(super) fn build_review_focus(
    is_public: bool,
    cross_module: bool,
    cross_package: bool,
    fan_in: usize,
    tests: &[(Node, Edge)],
) -> Vec<String> {
    let mut focus: Vec<String> = vec![];
    if is_public {
        focus.push("review public API contract and downstream consumers".to_owned());
    }
    if cross_package {
        focus.push("audit cross-package dependencies for breakage".to_owned());
    } else if cross_module {
        focus.push("check cross-module call sites for compatibility".to_owned());
    }
    if fan_in > 5 {
        focus.push(format!(
            "verify all {fan_in} call sites handle change correctly"
        ));
    }
    if tests.is_empty() {
        focus.push("add tests before merging — symbol is uncovered".to_owned());
    }
    focus
}
