use super::*;

// ---------------------------------------------------------------------------
// Boundary detection
// ---------------------------------------------------------------------------

/// Detect cross-module, cross-package, and cross-repo impacts.
///
/// Module  = unique directory prefix of a node's `file_path`.
/// Package = `owner_id` from node metadata when present, else top-level path component.
/// Repo    = `repo_id` from node metadata when present, else top-level path component.
pub(crate) fn detect_boundary_violations(base: &ImpactResult) -> Vec<BoundaryViolation> {
    let module_of = |path: &str| -> String {
        // Use the directory part of the path (everything before the last `/`).
        match path.rfind('/') {
            Some(idx) => path[..idx].to_string(),
            None => ".".to_string(),
        }
    };

    let package_of = |node: &Node| -> String {
        node.extra_json
            .as_object()
            .and_then(|extra| extra.get("owner_id"))
            .and_then(|value| value.as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                node.file_path
                    .split('/')
                    .next()
                    .unwrap_or(&node.file_path)
                    .to_string()
            })
    };

    let repo_of = |node: &Node| -> String {
        node.extra_json
            .as_object()
            .and_then(|extra| extra.get("repo_id"))
            .and_then(|value| value.as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                node.file_path
                    .split('/')
                    .next()
                    .unwrap_or(&node.file_path)
                    .to_string()
            })
    };

    let changed_modules: HashSet<String> = base
        .changed_nodes
        .iter()
        .map(|n| module_of(&n.file_path))
        .collect();

    let changed_packages: HashSet<String> = base.changed_nodes.iter().map(package_of).collect();
    let changed_repos: HashSet<String> = base.changed_nodes.iter().map(repo_of).collect();

    // Collect impacted nodes outside the changed modules/packages/repos.
    let mut cross_module_qns: Vec<String> = Vec::new();
    let mut cross_package_qns: Vec<String> = Vec::new();
    let mut cross_repo_qns: Vec<String> = Vec::new();

    for n in &base.impacted_nodes {
        let m = module_of(&n.file_path);
        if !changed_modules.contains(&m) {
            cross_module_qns.push(n.qualified_name.clone());
        }
        let p = package_of(n);
        if !changed_packages.contains(&p) {
            cross_package_qns.push(n.qualified_name.clone());
        }
        let repo = repo_of(n);
        if !changed_repos.contains(&repo) {
            cross_repo_qns.push(n.qualified_name.clone());
        }
    }

    let mut violations = Vec::new();

    if !cross_module_qns.is_empty() {
        cross_module_qns.sort();
        cross_module_qns.dedup();
        let count = cross_module_qns.len();
        violations.push(BoundaryViolation {
            kind: BoundaryKind::CrossModule,
            description: format!("{count} node(s) in other modules are impacted by this change"),
            nodes: cross_module_qns,
        });
    }

    if !cross_package_qns.is_empty() {
        cross_package_qns.sort();
        cross_package_qns.dedup();
        let count = cross_package_qns.len();
        violations.push(BoundaryViolation {
            kind: BoundaryKind::CrossPackage,
            description: format!("{count} node(s) in other packages are impacted by this change"),
            nodes: cross_package_qns,
        });
    }

    if !cross_repo_qns.is_empty() {
        cross_repo_qns.sort();
        cross_repo_qns.dedup();
        let count = cross_repo_qns.len();
        violations.push(BoundaryViolation {
            kind: BoundaryKind::CrossRepo,
            description: format!("{count} node(s) in other repos are impacted by this change"),
            nodes: cross_repo_qns,
        });
    }

    violations
}

pub(crate) fn boundary_signal_sets(
    violations: &[BoundaryViolation],
) -> (HashSet<String>, HashSet<String>, HashSet<String>) {
    let mut cross_module_qns = HashSet::new();
    let mut cross_package_qns = HashSet::new();
    let mut cross_repo_qns = HashSet::new();

    for violation in violations {
        match violation.kind {
            BoundaryKind::CrossModule => {
                cross_module_qns.extend(violation.nodes.iter().cloned());
            }
            BoundaryKind::CrossPackage => {
                cross_package_qns.extend(violation.nodes.iter().cloned());
            }
            BoundaryKind::CrossRepo => {
                cross_repo_qns.extend(violation.nodes.iter().cloned());
            }
        }
    }

    (cross_module_qns, cross_package_qns, cross_repo_qns)
}

// ---------------------------------------------------------------------------
// Risk level
// ---------------------------------------------------------------------------
