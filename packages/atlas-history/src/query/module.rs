use std::collections::BTreeMap;

use anyhow::Result;
use atlas_store_sqlite::Store;

use crate::diff::module_key;
use crate::reports::{build_evidence, edge_identifier_for, node_identifier_for};

use super::{
    ModuleHistoryFindings, ModuleHistoryPoint, ModuleHistoryReport, ModuleHistorySummary,
    load_snapshot_states, sorted_strings,
};

pub fn query_module_history(
    store: &Store,
    canonical_root: &str,
    module: &str,
) -> Result<ModuleHistoryReport> {
    let states = load_snapshot_states(store, canonical_root)?;
    let mut timeline = Vec::new();
    let mut commits_where_changed = Vec::new();
    let mut evidence_snapshot_ids = std::collections::BTreeSet::new();
    let mut evidence_commit_shas = std::collections::BTreeSet::new();
    let mut evidence_node_ids = std::collections::BTreeSet::new();
    let mut evidence_edge_ids = std::collections::BTreeSet::new();
    let mut evidence_file_paths = std::collections::BTreeSet::new();
    let mut first_seen = None;
    let mut last_seen = None;
    let mut removal_commit_sha = None;
    let mut prev_point = None::<(usize, usize, usize, usize)>;
    let mut prev_present = false;

    for state in &states {
        let node_lookup = state
            .nodes
            .iter()
            .map(|node| (node.qualified_name.clone(), node))
            .collect::<BTreeMap<_, _>>();
        let node_modules = state
            .nodes
            .iter()
            .map(|node| (node.qualified_name.clone(), module_key(&node.file_path)))
            .collect::<BTreeMap<_, _>>();
        let nodes = state
            .nodes
            .iter()
            .filter(|node| module_key(&node.file_path) == module)
            .cloned()
            .collect::<Vec<_>>();
        let node_count = nodes.len();
        let file_paths = sorted_strings(nodes.iter().map(|node| node.file_path.clone()));
        let mut dependency_paths = std::collections::BTreeSet::new();
        let mut coupling_count = 0usize;
        let mut test_adjacent = std::collections::BTreeSet::new();
        let mut edge_ids = std::collections::BTreeSet::new();

        for node in &nodes {
            if node.is_test {
                test_adjacent.insert(node.qualified_name.clone());
            }
        }

        for edge in &state.edges {
            let source_module = node_modules
                .get(&edge.source_qn)
                .cloned()
                .unwrap_or_else(|| module_key(&edge.file_path));
            let target_module = node_modules
                .get(&edge.target_qn)
                .cloned()
                .unwrap_or_else(|| module_key(&edge.file_path));

            if source_module != target_module
                && (source_module == module || target_module == module)
            {
                dependency_paths.insert((source_module.clone(), target_module.clone()));
                coupling_count += 1;
                edge_ids.insert(edge_identifier_for(edge));
            }

            if source_module == module || target_module == module {
                if let Some(node) = node_lookup.get(&edge.source_qn)
                    && node.is_test
                {
                    test_adjacent.insert(node.qualified_name.clone());
                }
                if let Some(node) = node_lookup.get(&edge.target_qn)
                    && node.is_test
                {
                    test_adjacent.insert(node.qualified_name.clone());
                }
            }
        }

        let active = node_count > 0 || !dependency_paths.is_empty() || coupling_count > 0;
        if active || prev_present {
            timeline.push(ModuleHistoryPoint {
                snapshot_id: state.snapshot_id,
                commit_sha: state.commit_sha.clone(),
                node_count,
                dependency_count: dependency_paths.len(),
                coupling_count,
                test_adjacency_count: test_adjacent.len(),
                file_paths: file_paths.clone(),
            });
        }

        let point_tuple = (
            node_count,
            dependency_paths.len(),
            coupling_count,
            test_adjacent.len(),
        );
        if active && (!prev_present || prev_point != Some(point_tuple)) {
            commits_where_changed.push(ModuleHistoryPoint {
                snapshot_id: state.snapshot_id,
                commit_sha: state.commit_sha.clone(),
                node_count,
                dependency_count: dependency_paths.len(),
                coupling_count,
                test_adjacency_count: test_adjacent.len(),
                file_paths: file_paths.clone(),
            });
        } else if !active && prev_present {
            removal_commit_sha = Some(state.commit_sha.clone());
            commits_where_changed.push(ModuleHistoryPoint {
                snapshot_id: state.snapshot_id,
                commit_sha: state.commit_sha.clone(),
                node_count,
                dependency_count: dependency_paths.len(),
                coupling_count,
                test_adjacency_count: test_adjacent.len(),
                file_paths: file_paths.clone(),
            });
        }

        if active {
            evidence_snapshot_ids.insert(state.snapshot_id);
            evidence_commit_shas.insert(state.commit_sha.clone());
            evidence_node_ids.extend(nodes.iter().map(node_identifier_for));
            evidence_edge_ids.extend(edge_ids);
            evidence_file_paths.extend(file_paths.iter().cloned());
            if first_seen.is_none() {
                first_seen = Some((state.snapshot_id, state.commit_sha.clone()));
            }
            last_seen = Some((
                state.snapshot_id,
                state.commit_sha.clone(),
                point_tuple.0,
                point_tuple.1,
                point_tuple.2,
                point_tuple.3,
            ));
        }

        prev_point = Some(point_tuple);
        prev_present = active;
    }

    let Some((first_snapshot_id, first_commit_sha)) = first_seen else {
        anyhow::bail!("no historical module matches found for {module}");
    };
    let (last_snapshot_id, last_commit_sha, _, _, _, _) = last_seen
        .ok_or_else(|| anyhow::anyhow!("module history missing last appearance for {module}"))?;
    let max_node_count = timeline
        .iter()
        .map(|point| point.node_count)
        .max()
        .unwrap_or(0);
    let max_dependency_count = timeline
        .iter()
        .map(|point| point.dependency_count)
        .max()
        .unwrap_or(0);
    let max_coupling_count = timeline
        .iter()
        .map(|point| point.coupling_count)
        .max()
        .unwrap_or(0);
    let max_test_adjacency_count = timeline
        .iter()
        .map(|point| point.test_adjacency_count)
        .max()
        .unwrap_or(0);

    Ok(ModuleHistoryReport {
        module: module.to_owned(),
        summary: ModuleHistorySummary {
            module: module.to_owned(),
            first_appearance_snapshot_id: Some(first_snapshot_id),
            last_appearance_snapshot_id: Some(last_snapshot_id),
            first_appearance_commit_sha: Some(first_commit_sha),
            last_appearance_commit_sha: Some(last_commit_sha),
            removal_commit_sha,
            max_node_count,
            max_dependency_count,
            max_coupling_count,
            max_test_adjacency_count,
            timeline_points: timeline.len(),
        },
        findings: ModuleHistoryFindings {
            timeline,
            commits_where_changed,
        },
        evidence: build_evidence(
            evidence_snapshot_ids,
            evidence_commit_shas,
            evidence_node_ids,
            evidence_edge_ids,
            evidence_file_paths,
        ),
    })
}
