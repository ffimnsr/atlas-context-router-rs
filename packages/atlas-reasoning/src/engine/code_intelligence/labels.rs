use super::*;

impl<'s> InsightsEngine<'s> {
    pub fn label_components(
        &self,
        repo_root: impl AsRef<Path>,
        request: ComponentLabelRequest,
    ) -> Result<ComponentLabelAnalysis> {
        let store = self.store().ok_or_else(|| {
            AtlasError::Other(
                "component labeling requires a store-backed insights engine".to_owned(),
            )
        })?;
        let snapshot = self.load_graph_snapshot(store, repo_root.as_ref())?;
        let explicit_files = normalize_paths(request.files.as_deref())?
            .into_iter()
            .collect::<BTreeSet<_>>();
        let explicit_symbols = request
            .symbols
            .clone()
            .unwrap_or_default()
            .into_iter()
            .collect::<BTreeSet<_>>();
        let limit = request.limit.unwrap_or(self.config().max_findings);

        let mut assignments = Vec::new();
        let mut file_paths = snapshot
            .owner_by_file
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        file_paths.extend(explicit_files.iter().cloned());
        for file_path in file_paths {
            if !explicit_files.is_empty() && !explicit_files.contains(&file_path) {
                continue;
            }
            let labels = labels_for_file(&file_path);
            if labels.is_empty() {
                continue;
            }
            assignments.push(ComponentLabelAssignment {
                file_path: file_path.clone(),
                qualified_name: None,
                labels,
            });
        }

        for node in &snapshot.nodes {
            if !explicit_symbols.is_empty() && !explicit_symbols.contains(&node.qualified_name) {
                continue;
            }
            if !explicit_files.is_empty() && !explicit_files.contains(&node.file_path) {
                continue;
            }
            let labels = labels_for_symbol(node);
            if labels.is_empty() {
                continue;
            }
            assignments.push(ComponentLabelAssignment {
                file_path: node.file_path.clone(),
                qualified_name: Some(node.qualified_name.clone()),
                labels,
            });
        }

        assignments.sort_by(|left, right| {
            left.file_path
                .cmp(&right.file_path)
                .then_with(|| left.qualified_name.cmp(&right.qualified_name))
        });
        if assignments.len() > limit {
            assignments.truncate(limit);
        }

        let findings = assignments
            .iter()
            .filter_map(component_assignment_to_finding)
            .collect::<Vec<_>>();
        let report = self.pattern_report(findings);

        Ok(ComponentLabelAnalysis {
            request,
            report,
            assignments,
        })
    }
}

pub(super) fn labels_for_file(file_path: &str) -> Vec<ComponentLabelMatch> {
    let mut labels = Vec::new();
    add_component_label(
        &mut labels,
        file_path.starts_with("packages/atlas-cli/"),
        "cli",
        1.0,
        "file path under packages/atlas-cli",
    );
    add_component_label(
        &mut labels,
        file_path.starts_with("packages/atlas-mcp/") || file_path == "MCP_TOOLS.md",
        "mcp",
        1.0,
        "file path under packages/atlas-mcp or MCP docs",
    );
    add_component_label(
        &mut labels,
        file_path.starts_with("packages/atlas-repo/"),
        "repo_scan",
        0.96,
        "file path under packages/atlas-repo",
    );
    add_component_label(
        &mut labels,
        file_path.starts_with("packages/atlas-parser/"),
        "parse",
        0.98,
        "file path under packages/atlas-parser",
    );
    add_component_label(
        &mut labels,
        file_path.starts_with("packages/atlas-store-sqlite/")
            || file_path.starts_with("packages/atlas-db-utils/"),
        "persist_graph",
        0.94,
        "store/db package path",
    );
    add_component_label(
        &mut labels,
        file_path.starts_with("packages/atlas-engine/")
            || file_path.contains("/update.rs")
            || file_path.contains("/watch.rs")
            || file_path.contains("postprocess"),
        "incremental_update",
        0.86,
        "engine/update/watch path",
    );
    add_component_label(
        &mut labels,
        file_path.starts_with("packages/atlas-search/")
            || file_path.contains("query")
            || file_path.contains("traverse"),
        "search_traverse",
        0.84,
        "search/query/traverse path",
    );
    add_component_label(
        &mut labels,
        file_path.starts_with("packages/atlas-review/")
            || file_path.contains("review")
            || file_path.ends_with("changes.rs")
            || file_path.ends_with("context_cmd.rs"),
        "review_context",
        0.88,
        "review/context path",
    );
    add_component_label(
        &mut labels,
        file_path.starts_with("packages/atlas-contextsave/")
            || file_path.starts_with("packages/atlas-contentstore/"),
        "context_memory",
        0.94,
        "content/context storage path",
    );
    add_component_label(
        &mut labels,
        file_path.starts_with("packages/atlas-session/")
            || file_path.starts_with("packages/atlas-agent-events/")
            || file_path.contains("session")
            || file_path.contains("wake_up"),
        "session_continuity",
        0.90,
        "session memory path",
    );
    add_component_label(
        &mut labels,
        file_path.starts_with("schemas/")
            || file_path.starts_with(".atlas/")
            || file_path.ends_with(".toml")
            || file_path.ends_with(".yaml")
            || file_path.ends_with(".yml")
            || file_path.ends_with(".json"),
        "config",
        0.82,
        "config/schema extension or directory",
    );
    add_component_label(
        &mut labels,
        file_path.contains("health")
            || file_path.contains("doctor")
            || file_path.contains("db_check")
            || file_path.contains("debug_graph")
            || file_path.contains("status"),
        "diagnostics",
        0.85,
        "health/doctor/status path",
    );
    add_component_label(
        &mut labels,
        file_path.starts_with("tests/")
            || file_path.contains("/tests/")
            || file_path.ends_with("_test.rs")
            || file_path.ends_with("tests.rs"),
        "tests",
        0.97,
        "test path pattern",
    );
    add_component_label(
        &mut labels,
        file_path.starts_with("docs/")
            || file_path.starts_with("wiki/")
            || file_path.ends_with(".md"),
        "docs",
        0.95,
        "docs/wiki/markdown path",
    );
    labels.sort_by(|left, right| left.label.cmp(&right.label));
    labels
}

pub(super) fn labels_for_symbol(node: &Node) -> Vec<ComponentLabelMatch> {
    let mut labels = labels_for_file(&node.file_path);
    if node.name.contains("doctor") || node.name.contains("status") {
        add_component_label(
            &mut labels,
            true,
            "diagnostics",
            0.78,
            "symbol name suggests diagnostics/health surface",
        );
    }
    labels.sort_by(|left, right| left.label.cmp(&right.label));
    labels.dedup_by(|left, right| left.label == right.label);
    labels
}

pub(super) fn add_component_label(
    labels: &mut Vec<ComponentLabelMatch>,
    predicate: bool,
    label: &str,
    confidence: f64,
    evidence: &str,
) {
    if !predicate {
        return;
    }
    if let Some(existing) = labels.iter_mut().find(|item| item.label == label) {
        existing.confidence = existing.confidence.max(confidence);
        if !existing.evidence.iter().any(|item| item == evidence) {
            existing.evidence.push(evidence.to_owned());
        }
        return;
    }
    labels.push(ComponentLabelMatch {
        label: label.to_owned(),
        confidence,
        evidence: vec![evidence.to_owned()],
    });
}

pub(super) fn component_assignment_to_finding(
    assignment: &ComponentLabelAssignment,
) -> Option<InsightFinding> {
    let best = assignment
        .labels
        .iter()
        .min_by(|left, right| left.confidence.total_cmp(&right.confidence))?;
    if best.confidence >= 0.75 {
        return None;
    }
    Some(InsightFinding {
        id: format!(
            "component_label:{}:{}",
            assignment.file_path,
            assignment
                .qualified_name
                .clone()
                .unwrap_or_else(|| "<file>".to_owned())
        ),
        title: format!(
            "low-confidence component labels for {}",
            assignment.file_path
        ),
        severity: InsightSeverity::Low,
        category: "component_labels".to_owned(),
        message: format!(
            "{} label confidence {:.2} stays below 0.75",
            best.label, best.confidence
        ),
        evidence: vec![InsightEvidence {
            file_path: Some(assignment.file_path.clone()),
            qualified_name: assignment.qualified_name.clone(),
            node_kind: None,
            edge_kind: None,
            line_range: None,
            confidence_tier: None,
        }],
        ranking_reason: "label confidence below deterministic threshold".to_owned(),
        details: Some(json!({
            "labels": assignment.labels,
        })),
        score: (1.0 - best.confidence) * 100.0,
    })
}

pub(super) fn module_to_finding(module: &InferredModule) -> Option<InsightFinding> {
    if module.confidence >= 0.75 {
        return None;
    }
    Some(InsightFinding {
        id: format!("infer_module:{}", module.module_id),
        title: format!("low-confidence inferred module {}", module.display_name),
        severity: InsightSeverity::Low,
        category: "inferred_modules".to_owned(),
        message: format!(
            "module {} inferred with confidence {:.2}",
            module.display_name, module.confidence
        ),
        evidence: module
            .root_paths
            .iter()
            .take(3)
            .map(|file_path| InsightEvidence {
                file_path: Some(file_path.clone()),
                qualified_name: None,
                node_kind: None,
                edge_kind: None,
                line_range: None,
                confidence_tier: None,
            })
            .collect(),
        ranking_reason: "heuristic-only module inference without explicit package owner".to_owned(),
        details: Some(json!({
            "module_id": module.module_id,
            "evidence": module.evidence,
            "outbound_dependencies": module.outbound_dependencies,
            "inbound_dependencies": module.inbound_dependencies,
        })),
        score: (1.0 - module.confidence) * 100.0,
    })
}
