//! Deterministic Louvain community detection over the code graph.
//!
//! Communities are a derived analytics surface: detection reads graph nodes and
//! edges, never source files, and persists memberships only when `persist` is
//! true. The implementation is a standard weighted Louvain local-moving loop
//! with deterministic node order and tie-breaking so repeated runs produce the
//! same partition.

use std::collections::{BTreeMap, HashMap};

use anyhow::Context;
use atlas_core::kinds::EdgeKind;
use atlas_core::model::{Edge, Node};
use atlas_store_sqlite::Store;
use serde::Serialize;

pub const COMMUNITY_ALGORITHM: &str = "louvain";

#[derive(Debug, Clone, Serialize)]
pub struct CommunityDetectionOptions {
    pub min_members: usize,
    pub resolution: f64,
    pub max_levels: usize,
    pub persist: bool,
}

impl Default for CommunityDetectionOptions {
    fn default() -> Self {
        Self {
            min_members: 2,
            resolution: 1.0,
            max_levels: 10,
            persist: true,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DetectedCommunity {
    pub name: String,
    pub member_count: usize,
    pub dominant_module: Option<String>,
    pub members: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommunityDetectionReport {
    pub algorithm: String,
    pub modularity: f64,
    pub node_count: usize,
    pub edge_count: usize,
    pub community_count: usize,
    pub min_members: usize,
    pub persisted: bool,
    pub communities: Vec<DetectedCommunity>,
    pub warnings: Vec<String>,
}

/// Detect communities over call/reference/import edges and optionally persist
/// them as `louvain` communities.
pub fn detect_communities(
    store: &Store,
    options: &CommunityDetectionOptions,
) -> anyhow::Result<CommunityDetectionReport> {
    let nodes = store.list_all_nodes().context("cannot list graph nodes")?;
    let edges = store.list_all_edges().context("cannot list graph edges")?;
    let mut warnings = Vec::new();
    let (qnames, adjacency) = build_graph(&nodes, &edges);
    let node_count = qnames.len();
    if node_count == 0 {
        return Ok(CommunityDetectionReport {
            algorithm: COMMUNITY_ALGORITHM.to_owned(),
            modularity: 0.0,
            node_count: 0,
            edge_count: 0,
            community_count: 0,
            min_members: options.min_members,
            persisted: false,
            communities: Vec::new(),
            warnings,
        });
    }

    let (membership, modularity) = louvain(&adjacency, options.resolution, options.max_levels);
    let edge_count = adjacency
        .iter()
        .map(|neighbours| neighbours.len())
        .sum::<usize>()
        / 2;

    let mut grouped: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for (index, community) in membership.iter().enumerate() {
        grouped
            .entry(*community)
            .or_default()
            .push(qnames[index].clone());
    }
    let node_by_qname = qnames
        .iter()
        .enumerate()
        .map(|(index, qname)| (qname.as_str(), index))
        .collect::<HashMap<_, _>>();
    let nodes_by_qname = nodes
        .iter()
        .filter(|node| node_by_qname.contains_key(node.qualified_name.as_str()))
        .map(|node| (node.qualified_name.as_str(), node))
        .collect::<HashMap<_, _>>();

    let mut communities = grouped
        .into_values()
        .filter(|members| members.len() >= options.min_members)
        .map(|members| {
            let dominant_module = dominant_module(&members, &nodes_by_qname);
            DetectedCommunity {
                name: dominant_module
                    .clone()
                    .unwrap_or_else(|| "community".to_owned()),
                member_count: members.len(),
                dominant_module,
                members,
            }
        })
        .collect::<Vec<_>>();
    communities.sort_by(|left, right| {
        right
            .member_count
            .cmp(&left.member_count)
            .then_with(|| left.name.cmp(&right.name))
    });
    uniquify_names(&mut communities);

    let mut persisted = false;
    if options.persist {
        persist_communities(store, &communities)?;
        persisted = true;
    } else if options.min_members > 1 {
        warnings.push(format!(
            "communities with fewer than {} members were omitted",
            options.min_members
        ));
    }

    Ok(CommunityDetectionReport {
        algorithm: COMMUNITY_ALGORITHM.to_owned(),
        modularity,
        node_count,
        edge_count,
        community_count: communities.len(),
        min_members: options.min_members,
        persisted,
        communities,
        warnings,
    })
}

fn build_graph(nodes: &[Node], edges: &[Edge]) -> (Vec<String>, Vec<BTreeMap<usize, f64>>) {
    let mut qnames = nodes
        .iter()
        .filter(|node| {
            !matches!(
                node.kind,
                atlas_core::NodeKind::File
                    | atlas_core::NodeKind::Package
                    | atlas_core::NodeKind::Import
            )
        })
        .filter(|node| !node.file_path.starts_with(".atlas/"))
        .map(|node| node.qualified_name.clone())
        .collect::<Vec<_>>();
    qnames.sort();
    qnames.dedup();
    let index = qnames
        .iter()
        .enumerate()
        .map(|(index, qname)| (qname.as_str(), index))
        .collect::<HashMap<_, _>>();
    let mut adjacency = vec![BTreeMap::new(); qnames.len()];
    for edge in edges {
        if !matches!(
            edge.kind,
            EdgeKind::Calls
                | EdgeKind::References
                | EdgeKind::Imports
                | EdgeKind::Implements
                | EdgeKind::Extends
        ) {
            continue;
        }
        let Some(&source) = index.get(edge.source_qn.as_str()) else {
            continue;
        };
        let Some(&target) = index.get(edge.target_qn.as_str()) else {
            continue;
        };
        if source == target {
            continue;
        }
        *adjacency[source].entry(target).or_insert(0.0) += 1.0;
        *adjacency[target].entry(source).or_insert(0.0) += 1.0;
    }
    (qnames, adjacency)
}

/// Standard Louvain: local moving followed by graph aggregation. Returns the
/// final partition plus modularity measured on the original graph.
fn louvain(
    adjacency: &[BTreeMap<usize, f64>],
    resolution: f64,
    max_levels: usize,
) -> (Vec<usize>, f64) {
    let node_count = adjacency.len();
    if node_count == 0 {
        return (Vec::new(), 0.0);
    }
    let mut graph = adjacency.to_vec();
    let mut membership = (0..node_count).collect::<Vec<_>>();
    let mut levels = 0;
    loop {
        let partition = local_moving(&graph, resolution);
        let (partition, community_count) = compact_partition(&partition);
        if community_count == graph.len() {
            break;
        }
        for community in membership.iter_mut() {
            *community = partition[*community];
        }
        graph = aggregate(&graph, &partition, community_count);
        levels += 1;
        if levels >= max_levels {
            break;
        }
    }
    let modularity = modularity(adjacency, &membership, resolution);
    (membership, modularity)
}

fn local_moving(adjacency: &[BTreeMap<usize, f64>], resolution: f64) -> Vec<usize> {
    let node_count = adjacency.len();
    let degree = adjacency
        .iter()
        .map(|neighbours| neighbours.values().sum::<f64>())
        .collect::<Vec<_>>();
    let total_weight = degree.iter().sum::<f64>();
    if total_weight == 0.0 {
        return (0..node_count).collect();
    }
    let two_m = total_weight;
    let mut community = (0..node_count).collect::<Vec<_>>();
    let mut total = degree.clone();

    loop {
        let mut improved = false;
        for node in 0..node_count {
            let current = community[node];
            let weight = degree[node];
            if weight == 0.0 {
                continue;
            }
            total[current] -= weight;

            let mut weights: BTreeMap<usize, f64> = BTreeMap::new();
            for (neighbour, edge_weight) in &adjacency[node] {
                *weights.entry(community[*neighbour]).or_insert(0.0) += edge_weight;
            }

            let mut best = current;
            let mut best_gain = weights.get(&current).copied().unwrap_or(0.0)
                - resolution * total[current] * weight / two_m;
            for (candidate, candidate_weight) in weights {
                let gain = candidate_weight - resolution * total[candidate] * weight / two_m;
                if gain > best_gain + 1e-12
                    || ((gain - best_gain).abs() <= 1e-12 && candidate < best)
                {
                    best = candidate;
                    best_gain = gain;
                }
            }

            total[best] += weight;
            if best != current {
                community[node] = best;
                improved = true;
            }
        }
        if !improved {
            break;
        }
    }
    community
}

/// Compact community labels to `0..k` in first-seen order so aggregation can
/// index by label without gaps.
fn compact_partition(partition: &[usize]) -> (Vec<usize>, usize) {
    let mut mapping: BTreeMap<usize, usize> = BTreeMap::new();
    let mut compacted = Vec::with_capacity(partition.len());
    for label in partition {
        let next = mapping.len();
        let id = *mapping.entry(*label).or_insert(next);
        compacted.push(id);
    }
    (compacted, mapping.len())
}

fn aggregate(
    adjacency: &[BTreeMap<usize, f64>],
    community: &[usize],
    community_count: usize,
) -> Vec<BTreeMap<usize, f64>> {
    let mut aggregated = vec![BTreeMap::new(); community_count];
    let mut self_loops = vec![0.0; community_count];
    for source in 0..adjacency.len() {
        let source_community = community[source];
        for (target, weight) in &adjacency[source] {
            let target_community = community[*target];
            if source_community == target_community {
                if source < *target {
                    self_loops[source_community] += weight;
                }
            } else if source < *target {
                *aggregated[source_community]
                    .entry(target_community)
                    .or_insert(0.0) += weight;
                *aggregated[target_community]
                    .entry(source_community)
                    .or_insert(0.0) += weight;
            }
        }
    }
    for (index, loop_weight) in self_loops.into_iter().enumerate() {
        if loop_weight > 0.0 {
            *aggregated[index].entry(index).or_insert(0.0) += loop_weight;
        }
    }
    aggregated
}

fn modularity(adjacency: &[BTreeMap<usize, f64>], community: &[usize], resolution: f64) -> f64 {
    let total_weight = adjacency
        .iter()
        .map(|neighbours| neighbours.values().sum::<f64>())
        .sum::<f64>();
    if total_weight == 0.0 {
        return 0.0;
    }
    let two_m = total_weight;
    let mut internal = vec![0.0; community.iter().copied().max().unwrap_or(0) + 1];
    let mut degree = vec![0.0; internal.len()];
    for source in 0..adjacency.len() {
        let source_community = community[source];
        degree[source_community] += adjacency[source].values().sum::<f64>();
        for (target, weight) in &adjacency[source] {
            if source <= *target && source_community == community[*target] {
                internal[source_community] += weight;
            }
        }
    }
    let m = two_m / 2.0;
    internal
        .iter()
        .zip(degree.iter())
        .map(|(internal_weight, degree)| {
            internal_weight / m - resolution * (degree / two_m).powi(2)
        })
        .sum()
}

fn dominant_module(members: &[String], nodes: &HashMap<&str, &Node>) -> Option<String> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for member in members {
        let Some(node) = nodes.get(member.as_str()) else {
            continue;
        };
        let module = module_prefix(&node.file_path);
        *counts.entry(module).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0)))
        .map(|(module, _)| module)
}

fn module_prefix(path: &str) -> String {
    let mut components = path.split('/');
    match components.next() {
        Some("packages") | Some("crates") => {
            let first = components.next().unwrap_or_default();
            if first.is_empty() {
                path.to_owned()
            } else {
                first.to_owned()
            }
        }
        Some(first) if !first.is_empty() => first.to_owned(),
        _ => path.to_owned(),
    }
}

fn uniquify_names(communities: &mut [DetectedCommunity]) {
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for community in communities {
        let count = seen.entry(community.name.clone()).or_insert(0);
        *count += 1;
        if *count > 1 {
            community.name = format!("{}-{}", community.name, *count);
        }
    }
}

fn persist_communities(store: &Store, communities: &[DetectedCommunity]) -> anyhow::Result<()> {
    for existing in store
        .list_communities()
        .context("cannot list communities")?
    {
        if existing.algorithm.as_deref() == Some(COMMUNITY_ALGORITHM) {
            store
                .delete_community(existing.id)
                .context("cannot replace detected community")?;
        }
    }
    for community in communities {
        let id = store
            .create_community(&community.name, Some(COMMUNITY_ALGORITHM), Some(0), None)
            .with_context(|| format!("cannot create community {}", community.name))?;
        for member in &community.members {
            store
                .add_community_node(id, member)
                .context("cannot add community member")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph_from_edges(edges: &[(usize, usize)]) -> Vec<BTreeMap<usize, f64>> {
        let node_count = edges
            .iter()
            .flat_map(|(source, target)| [*source, *target])
            .max()
            .unwrap_or(0)
            + 1;
        let mut adjacency = vec![BTreeMap::new(); node_count];
        for (source, target) in edges {
            *adjacency[*source].entry(*target).or_insert(0.0) += 1.0;
            *adjacency[*target].entry(*source).or_insert(0.0) += 1.0;
        }
        adjacency
    }

    #[test]
    fn louvain_separates_two_dense_cliques() {
        // Two triangles connected by one bridge edge.
        let adjacency = graph_from_edges(&[(0, 1), (1, 2), (0, 2), (3, 4), (4, 5), (3, 5), (2, 3)]);
        let (membership, modularity) = louvain(&adjacency, 1.0, 10);
        assert_eq!(membership[0], membership[1]);
        assert_eq!(membership[1], membership[2]);
        assert_eq!(membership[3], membership[4]);
        assert_eq!(membership[4], membership[5]);
        assert_ne!(membership[0], membership[3]);
        assert!(modularity > 0.3, "modularity was {modularity}");
    }

    #[test]
    fn module_prefix_groups_packages() {
        assert_eq!(
            module_prefix("packages/atlas-core/src/health.rs"),
            "atlas-core"
        );
        assert_eq!(module_prefix("src/lib.rs"), "src");
    }
}
