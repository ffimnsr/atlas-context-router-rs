use super::*;

impl<'s> InsightsEngine<'s> {
    pub fn infer_modules(&self, repo_root: impl AsRef<Path>) -> Result<ModuleInferenceAnalysis> {
        let store = self.store().ok_or_else(|| {
            AtlasError::Other("module inference requires a store-backed insights engine".to_owned())
        })?;
        let snapshot = self.load_graph_snapshot(store, repo_root.as_ref())?;
        let file_assignments = infer_file_modules(store, &snapshot)?;
        let modules = build_inferred_modules(&snapshot, &file_assignments);
        let findings = modules
            .iter()
            .filter_map(module_to_finding)
            .collect::<Vec<_>>();
        let report = self.architecture_report(findings);
        Ok(ModuleInferenceAnalysis { report, modules })
    }
}

#[derive(Debug, Clone)]
pub(super) struct FileModuleAssignment {
    pub(super) module_id: String,
    pub(super) display_name: String,
    pub(super) explicit: bool,
    pub(super) confidence: f64,
    pub(super) evidence: Vec<String>,
}

pub(super) fn infer_file_modules(
    store: &atlas_store_sqlite::Store,
    snapshot: &GraphSnapshot,
) -> Result<BTreeMap<String, FileModuleAssignment>> {
    let community_by_qname = community_assignments(store)?;
    let nodes_by_file =
        snapshot
            .nodes
            .iter()
            .fold(BTreeMap::<String, Vec<&Node>>::new(), |mut acc, node| {
                acc.entry(node.file_path.clone()).or_default().push(node);
                acc
            });
    let mut assignments = BTreeMap::new();
    for file_path in snapshot.owner_by_file.keys() {
        let owner_id = store.file_owner_id(file_path)?;
        let assignment = if let Some(owner_id) = owner_id {
            FileModuleAssignment {
                module_id: owner_id.clone(),
                display_name: owner_id.clone(),
                explicit: true,
                confidence: 1.0,
                evidence: vec![format!("package owner `{owner_id}`")],
            }
        } else if let Some(assignment) = infer_community_module(
            nodes_by_file
                .get(file_path)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            &community_by_qname,
        ) {
            assignment
        } else {
            infer_path_module(file_path)
        };
        assignments.insert(file_path.clone(), assignment);
    }
    Ok(assignments)
}

pub(super) fn community_assignments(
    store: &atlas_store_sqlite::Store,
) -> Result<HashMap<String, (String, String)>> {
    let mut by_qname = HashMap::new();
    for community in store.list_communities()? {
        let display = community.name.clone();
        let module_id = format!("community:{}", community.id);
        for member in store.get_community_nodes(community.id)? {
            by_qname
                .entry(member.node_qualified_name)
                .or_insert_with(|| (module_id.clone(), display.clone()));
        }
    }
    Ok(by_qname)
}

pub(super) fn infer_community_module(
    nodes: &[&Node],
    community_by_qname: &HashMap<String, (String, String)>,
) -> Option<FileModuleAssignment> {
    let mut counts = BTreeMap::<(String, String), usize>::new();
    for node in nodes {
        if let Some((module_id, display_name)) = community_by_qname.get(&node.qualified_name) {
            *counts
                .entry((module_id.clone(), display_name.clone()))
                .or_default() += 1;
        }
    }
    let ((module_id, display_name), count) = counts
        .into_iter()
        .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0)))?;
    if count == 0 {
        return None;
    }
    Some(FileModuleAssignment {
        module_id: module_id.clone(),
        display_name: display_name.clone(),
        explicit: false,
        confidence: 0.88,
        evidence: vec![format!(
            "graph community `{display_name}` matched {count} file symbol(s)"
        )],
    })
}

pub(super) fn infer_path_module(file_path: &str) -> FileModuleAssignment {
    if let Some(rest) = file_path.strip_prefix("packages/")
        && let Some((package, _)) = rest.split_once('/')
    {
        return FileModuleAssignment {
            module_id: format!("infer:packages/{package}"),
            display_name: format!("packages/{package}"),
            explicit: false,
            confidence: 0.95,
            evidence: vec!["package directory prefix".to_owned()],
        };
    }
    if let Some(rest) = file_path.strip_prefix("src/") {
        if let Some((segment, _)) = rest.split_once('/') {
            return FileModuleAssignment {
                module_id: format!("infer:src/{segment}"),
                display_name: format!("src/{segment}"),
                explicit: false,
                confidence: 0.82,
                evidence: vec!["top-level src segment".to_owned()],
            };
        }
        return FileModuleAssignment {
            module_id: "infer:src".to_owned(),
            display_name: "src".to_owned(),
            explicit: false,
            confidence: 0.78,
            evidence: vec!["src root file".to_owned()],
        };
    }
    if file_path.starts_with("tests/") {
        return FileModuleAssignment {
            module_id: "infer:tests".to_owned(),
            display_name: "tests".to_owned(),
            explicit: false,
            confidence: 0.80,
            evidence: vec!["test directory prefix".to_owned()],
        };
    }
    if file_path.starts_with("docs/")
        || file_path.starts_with("wiki/")
        || file_path.ends_with(".md")
    {
        return FileModuleAssignment {
            module_id: "infer:docs".to_owned(),
            display_name: "docs".to_owned(),
            explicit: false,
            confidence: 0.76,
            evidence: vec!["docs path pattern".to_owned()],
        };
    }
    let display_name = file_path
        .rsplit_once('/')
        .map(|(prefix, _)| prefix.to_owned())
        .unwrap_or_else(|| "<root>".to_owned());
    FileModuleAssignment {
        module_id: format!("infer:{display_name}"),
        display_name,
        explicit: false,
        confidence: 0.62,
        evidence: vec!["parent directory fallback".to_owned()],
    }
}

pub(super) fn build_inferred_modules(
    snapshot: &GraphSnapshot,
    assignments: &BTreeMap<String, FileModuleAssignment>,
) -> Vec<InferredModule> {
    let node_to_module = snapshot
        .nodes
        .iter()
        .map(|node| {
            let assignment = assignments
                .get(&node.file_path)
                .cloned()
                .unwrap_or_else(|| infer_path_module(&node.file_path));
            (node.qualified_name.clone(), assignment.module_id)
        })
        .collect::<HashMap<_, _>>();
    let mut files_by_module = BTreeMap::<String, BTreeSet<String>>::new();
    let mut qnames_by_module = BTreeMap::<String, BTreeSet<String>>::new();
    let mut evidence_by_module = BTreeMap::<String, BTreeSet<String>>::new();
    let mut confidence_by_module = BTreeMap::<String, f64>::new();
    let mut display_by_module = BTreeMap::<String, String>::new();
    let mut explicit_by_module = BTreeMap::<String, bool>::new();
    for (file_path, assignment) in assignments {
        files_by_module
            .entry(assignment.module_id.clone())
            .or_default()
            .insert(file_path.clone());
        evidence_by_module
            .entry(assignment.module_id.clone())
            .or_default()
            .extend(assignment.evidence.iter().cloned());
        confidence_by_module
            .entry(assignment.module_id.clone())
            .and_modify(|score| *score = score.max(assignment.confidence))
            .or_insert(assignment.confidence);
        display_by_module
            .entry(assignment.module_id.clone())
            .or_insert_with(|| assignment.display_name.clone());
        explicit_by_module
            .entry(assignment.module_id.clone())
            .and_modify(|value| *value |= assignment.explicit)
            .or_insert(assignment.explicit);
    }
    for node in &snapshot.nodes {
        let module_id = node_to_module
            .get(&node.qualified_name)
            .cloned()
            .unwrap_or_else(|| module_id_for_file(&node.file_path, None));
        qnames_by_module
            .entry(module_id)
            .or_default()
            .insert(node.qualified_name.clone());
    }

    let mut outbound_by_module = BTreeMap::<String, BTreeSet<String>>::new();
    let mut inbound_by_module = BTreeMap::<String, BTreeSet<String>>::new();
    for edge in &snapshot.edges {
        let Some(source_module) = node_to_module.get(&edge.source_qn) else {
            continue;
        };
        let Some(target_module) = node_to_module.get(&edge.target_qn) else {
            continue;
        };
        if source_module == target_module {
            continue;
        }
        outbound_by_module
            .entry(source_module.clone())
            .or_default()
            .insert(target_module.clone());
        inbound_by_module
            .entry(target_module.clone())
            .or_default()
            .insert(source_module.clone());
    }

    let mut modules = files_by_module
        .into_iter()
        .map(|(module_id, file_paths)| {
            let owned_symbols = qnames_by_module
                .remove(&module_id)
                .unwrap_or_default()
                .into_iter()
                .collect::<Vec<_>>();
            InferredModule {
                display_name: display_by_module
                    .get(&module_id)
                    .cloned()
                    .unwrap_or_else(|| module_id.clone()),
                root_paths: file_paths.iter().cloned().collect(),
                node_count: owned_symbols.len(),
                owned_symbols,
                file_count: file_paths.len(),
                outbound_dependencies: outbound_by_module
                    .remove(&module_id)
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
                inbound_dependencies: inbound_by_module
                    .remove(&module_id)
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
                confidence: confidence_by_module.get(&module_id).copied().unwrap_or(0.5),
                evidence: evidence_by_module
                    .remove(&module_id)
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
                explicit: explicit_by_module.get(&module_id).copied().unwrap_or(false),
                module_id,
            }
        })
        .collect::<Vec<_>>();

    modules.sort_by(|left, right| left.module_id.cmp(&right.module_id));
    modules
}
