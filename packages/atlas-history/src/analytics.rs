use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use atlas_store_sqlite::{HistoricalEdge, HistoricalNode, Store};
use serde::Serialize;

use crate::diff::module_key;
use crate::query::{SnapshotState, load_snapshot_states, sorted_strings};
use crate::reports::{HistoryEvidence, build_evidence, edge_identifier_for, node_identifier_for};

#[derive(Debug, Clone, Serialize)]
pub struct ChurnSummary {
    pub commit_count: usize,
    pub snapshot_count: usize,
    pub symbol_count: usize,
    pub file_count: usize,
    pub module_count: usize,
    pub stable_symbol_count: usize,
    pub unstable_symbol_count: usize,
    pub frequently_changing_dependency_count: usize,
    pub hotspot_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolChurnRecord {
    pub qualified_name: String,
    pub first_commit_sha: String,
    pub last_commit_sha: String,
    pub kind: String,
    pub lifetime_snapshots: usize,
    pub lifetime_secs: Option<i64>,
    pub change_count: usize,
    pub introduction_count: usize,
    pub removal_count: usize,
    pub add_remove_frequency: f64,
    pub current_file_paths: Vec<String>,
    pub stability_score: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileChurnRecord {
    pub file_path: String,
    pub commits_touched: usize,
    pub graph_delta_size: usize,
    pub first_commit_sha: String,
    pub last_commit_sha: String,
    pub current_file_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleChurnRecord {
    pub module: String,
    pub dependency_churn: usize,
    pub symbol_churn: usize,
    pub file_paths: Vec<String>,
    pub hotspot_score: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct DependencyChurnRecord {
    pub source_qn: String,
    pub target_qn: String,
    pub kind: String,
    pub file_path: String,
    pub change_count: usize,
    pub introduction_count: usize,
    pub removal_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SymbolStabilityRecord {
    pub qualified_name: String,
    pub stability_score: f64,
    pub change_count: usize,
    pub lifetime_snapshots: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ArchitecturalHotspotRecord {
    pub module: String,
    pub hotspot_score: usize,
    pub symbol_churn: usize,
    pub dependency_churn: usize,
    pub max_coupling_count: usize,
    pub cycle_participation_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct StabilityIndicators {
    pub stable_symbols: Vec<SymbolStabilityRecord>,
    pub unstable_symbols: Vec<SymbolStabilityRecord>,
    pub frequently_changing_dependencies: Vec<DependencyChurnRecord>,
    pub architectural_hotspots: Vec<ArchitecturalHotspotRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrendPoint {
    pub snapshot_id: i64,
    pub commit_sha: String,
    pub file_count: usize,
    pub node_count: usize,
    pub edge_count: usize,
    pub cycle_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleCouplingPoint {
    pub module: String,
    pub coupling_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleCouplingTrendPoint {
    pub snapshot_id: i64,
    pub commit_sha: String,
    pub modules: Vec<ModuleCouplingPoint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TrendMetrics {
    pub file_count_growth: i64,
    pub node_count_growth: i64,
    pub edge_count_growth: i64,
    pub cycle_count_growth: i64,
    pub timeline: Vec<TrendPoint>,
    pub module_coupling_trend: Vec<ModuleCouplingTrendPoint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StorageDiagnostics {
    pub commits_stored: usize,
    pub snapshots_stored: usize,
    pub unique_file_hashes: usize,
    pub snapshot_file_memberships: usize,
    pub deduplication_ratio: f64,
    pub db_size_bytes: u64,
    pub snapshot_density: f64,
    pub storage_growth_without_dedup_bytes: u64,
    pub storage_growth_with_dedup_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChurnReport {
    pub summary: ChurnSummary,
    pub symbol_churn: Vec<SymbolChurnRecord>,
    pub file_churn: Vec<FileChurnRecord>,
    pub module_churn: Vec<ModuleChurnRecord>,
    pub stability: StabilityIndicators,
    pub trends: TrendMetrics,
    pub storage_diagnostics: StorageDiagnostics,
    pub evidence: HistoryEvidence,
}

#[derive(Debug, Default)]
struct SymbolAccumulator {
    kind: String,
    first_commit_sha: String,
    last_commit_sha: String,
    first_author_time: i64,
    last_author_time: i64,
    first_index: usize,
    last_index: usize,
    change_count: usize,
    introduction_count: usize,
    removal_count: usize,
    current_file_paths: Vec<String>,
}

#[derive(Debug, Default)]
struct FileAccumulator {
    first_commit_sha: String,
    last_commit_sha: String,
    commits_touched: usize,
    graph_delta_size: usize,
    current_file_hash: Option<String>,
}

#[derive(Debug, Default)]
struct ModuleAccumulator {
    file_paths: BTreeSet<String>,
    symbol_churn: usize,
    dependency_churn: usize,
    max_coupling_count: usize,
    cycle_participation_count: usize,
}

pub fn compute_churn_report(
    store: &Store,
    canonical_root: &str,
    db_path: &str,
) -> Result<ChurnReport> {
    let states = load_snapshot_states(store, canonical_root)?;
    let repo_id = store
        .find_repo_id(canonical_root)?
        .ok_or_else(|| anyhow::anyhow!("repo not yet registered for history: {canonical_root}"))?;
    let commits = store.list_commits(repo_id)?;
    let state_maps = states.iter().map(build_state_maps).collect::<Vec<_>>();

    let symbol_churn = build_symbol_churn(&states, &state_maps);
    let file_churn = build_file_churn(&states, &state_maps);
    let (module_churn, hotspot_records, module_trend, trend_points) =
        build_module_and_trend_metrics(&states, &state_maps);
    let dependency_churn = build_dependency_churn(&states, &state_maps);
    let stability = build_stability_indicators(&symbol_churn, &dependency_churn, hotspot_records);
    let storage_diagnostics = build_storage_diagnostics(store, db_path, &states)?;
    let evidence = build_report_evidence(&states);

    Ok(ChurnReport {
        summary: ChurnSummary {
            commit_count: commits.len(),
            snapshot_count: states.len(),
            symbol_count: symbol_churn.len(),
            file_count: file_churn.len(),
            module_count: module_churn.len(),
            stable_symbol_count: stability.stable_symbols.len(),
            unstable_symbol_count: stability.unstable_symbols.len(),
            frequently_changing_dependency_count: stability.frequently_changing_dependencies.len(),
            hotspot_count: stability.architectural_hotspots.len(),
        },
        symbol_churn,
        file_churn,
        module_churn,
        stability,
        trends: TrendMetrics {
            file_count_growth: growth_for(&trend_points, |point| point.file_count),
            node_count_growth: growth_for(&trend_points, |point| point.node_count),
            edge_count_growth: growth_for(&trend_points, |point| point.edge_count),
            cycle_count_growth: growth_for(&trend_points, |point| point.cycle_count),
            timeline: trend_points,
            module_coupling_trend: module_trend,
        },
        storage_diagnostics,
        evidence,
    })
}

fn build_symbol_churn(
    states: &[SnapshotState],
    state_maps: &[StateMaps],
) -> Vec<SymbolChurnRecord> {
    let all_symbols = state_maps
        .iter()
        .flat_map(|maps| maps.symbols.keys().cloned())
        .collect::<BTreeSet<_>>();
    let mut records = Vec::new();

    for qualified_name in all_symbols {
        let mut acc = SymbolAccumulator::default();
        let mut prev_signature = BTreeSet::new();
        let mut prev_paths = BTreeSet::new();
        let mut prev_present = false;

        for (index, (state, maps)) in states.iter().zip(state_maps.iter()).enumerate() {
            let current = maps
                .symbols
                .get(&qualified_name)
                .cloned()
                .unwrap_or_default();
            let present = !current.is_empty();
            let current_paths = current
                .iter()
                .map(|node| node.file_path.clone())
                .collect::<BTreeSet<_>>();
            let current_signature = current
                .iter()
                .map(symbol_signature_key)
                .collect::<BTreeSet<_>>();

            if present {
                if acc.first_commit_sha.is_empty() {
                    acc.first_commit_sha = state.commit_sha.clone();
                    acc.first_author_time = state.author_time;
                    acc.first_index = index;
                }
                acc.last_commit_sha = state.commit_sha.clone();
                acc.last_author_time = state.author_time;
                acc.last_index = index;
                acc.kind = current[0].kind.clone();
                acc.current_file_paths = sorted_strings(current_paths.iter().cloned());
            }

            if present && !prev_present {
                acc.introduction_count += 1;
                if index != acc.first_index {
                    acc.change_count += 1;
                }
            } else if !present && prev_present {
                acc.removal_count += 1;
                acc.change_count += 1;
            } else if present
                && prev_present
                && (current_signature != prev_signature || current_paths != prev_paths)
            {
                acc.change_count += 1;
            }

            prev_present = present;
            prev_signature = current_signature;
            prev_paths = current_paths;
        }

        if acc.first_commit_sha.is_empty() {
            continue;
        }

        let lifetime_snapshots = acc.last_index.saturating_sub(acc.first_index) + 1;
        let add_remove_frequency =
            (acc.introduction_count + acc.removal_count) as f64 / lifetime_snapshots.max(1) as f64;
        let change_density = acc.change_count as f64 / lifetime_snapshots.max(1) as f64;
        let stability_score =
            (1.0 - change_density - (add_remove_frequency * 0.25)).clamp(0.0, 1.0);

        records.push(SymbolChurnRecord {
            qualified_name,
            first_commit_sha: acc.first_commit_sha,
            last_commit_sha: acc.last_commit_sha,
            kind: acc.kind,
            lifetime_snapshots,
            lifetime_secs: Some(acc.last_author_time.saturating_sub(acc.first_author_time)),
            change_count: acc.change_count,
            introduction_count: acc.introduction_count,
            removal_count: acc.removal_count,
            add_remove_frequency,
            current_file_paths: acc.current_file_paths,
            stability_score,
        });
    }

    records.sort_by(|left, right| {
        right
            .change_count
            .cmp(&left.change_count)
            .then_with(|| left.qualified_name.cmp(&right.qualified_name))
    });
    records
}

fn build_file_churn(states: &[SnapshotState], state_maps: &[StateMaps]) -> Vec<FileChurnRecord> {
    let all_files = state_maps
        .iter()
        .flat_map(|maps| maps.files.keys().cloned())
        .collect::<BTreeSet<_>>();
    let mut records = Vec::new();

    for file_path in all_files {
        let mut acc = FileAccumulator::default();
        let mut prev_exists = false;
        let mut prev_hash = None::<String>;
        let mut prev_node_count = 0usize;
        let mut prev_edge_count = 0usize;

        for (state, maps) in states.iter().zip(state_maps.iter()) {
            let file = maps.files.get(&file_path);
            let exists = file.is_some();
            let current_hash = file.map(|file| file.file_hash.clone());
            let current_node_count = maps.nodes_by_file.get(&file_path).map_or(0, Vec::len);
            let current_edge_count = maps.edges_by_file.get(&file_path).map_or(0, Vec::len);

            if exists {
                if acc.first_commit_sha.is_empty() {
                    acc.first_commit_sha = state.commit_sha.clone();
                }
                acc.last_commit_sha = state.commit_sha.clone();
                acc.current_file_hash = current_hash.clone();
            }

            let touched = (exists != prev_exists) || current_hash != prev_hash;
            if touched && (exists || prev_exists) {
                acc.commits_touched += 1;
                acc.graph_delta_size += current_node_count.abs_diff(prev_node_count)
                    + current_edge_count.abs_diff(prev_edge_count);
            }

            prev_exists = exists;
            prev_hash = current_hash;
            prev_node_count = current_node_count;
            prev_edge_count = current_edge_count;
        }

        if acc.first_commit_sha.is_empty() {
            continue;
        }

        records.push(FileChurnRecord {
            file_path,
            commits_touched: acc.commits_touched,
            graph_delta_size: acc.graph_delta_size,
            first_commit_sha: acc.first_commit_sha,
            last_commit_sha: acc.last_commit_sha,
            current_file_hash: acc.current_file_hash,
        });
    }

    records.sort_by(|left, right| {
        right
            .commits_touched
            .cmp(&left.commits_touched)
            .then_with(|| right.graph_delta_size.cmp(&left.graph_delta_size))
            .then_with(|| left.file_path.cmp(&right.file_path))
    });
    records
}

fn build_dependency_churn(
    states: &[SnapshotState],
    state_maps: &[StateMaps],
) -> Vec<DependencyChurnRecord> {
    let all_edges = state_maps
        .iter()
        .flat_map(|maps| maps.dependencies.keys().cloned())
        .collect::<BTreeSet<_>>();
    let mut records = Vec::new();

    for edge_key in all_edges {
        let mut prev_present = false;
        let mut change_count = 0usize;
        let mut introduction_count = 0usize;
        let mut removal_count = 0usize;

        for maps in state_maps {
            let present = maps.dependencies.contains_key(&edge_key);
            if present && !prev_present {
                introduction_count += 1;
                if prev_present || introduction_count > 1 {
                    change_count += 1;
                }
            } else if !present && prev_present {
                removal_count += 1;
                change_count += 1;
            }
            prev_present = present;
        }

        if introduction_count == 0 {
            continue;
        }

        records.push(DependencyChurnRecord {
            source_qn: edge_key.0,
            target_qn: edge_key.1,
            kind: edge_key.2,
            file_path: edge_key.3,
            change_count,
            introduction_count,
            removal_count,
        });
    }

    records.sort_by(|left, right| {
        right
            .change_count
            .cmp(&left.change_count)
            .then_with(|| left.source_qn.cmp(&right.source_qn))
            .then_with(|| left.target_qn.cmp(&right.target_qn))
    });
    let _ = states;
    records
}

fn build_module_and_trend_metrics(
    states: &[SnapshotState],
    state_maps: &[StateMaps],
) -> (
    Vec<ModuleChurnRecord>,
    Vec<ArchitecturalHotspotRecord>,
    Vec<ModuleCouplingTrendPoint>,
    Vec<TrendPoint>,
) {
    let all_modules = state_maps
        .iter()
        .flat_map(|maps| maps.module_nodes.keys().cloned())
        .chain(
            state_maps
                .iter()
                .flat_map(|maps| maps.module_coupling.keys().cloned()),
        )
        .collect::<BTreeSet<_>>();
    let mut accumulators = BTreeMap::<String, ModuleAccumulator>::new();
    let mut prev_module_nodes = BTreeMap::<String, BTreeSet<String>>::new();
    let mut prev_module_dependencies = BTreeMap::<String, BTreeSet<(String, String)>>::new();
    let mut module_trend = Vec::new();
    let mut timeline = Vec::new();

    for (state, maps) in states.iter().zip(state_maps.iter()) {
        let cycles = strongly_connected_components(&maps.module_graph);
        let cycle_modules = cycles
            .iter()
            .flat_map(|cycle| cycle.iter().cloned())
            .collect::<BTreeSet<_>>();

        timeline.push(TrendPoint {
            snapshot_id: state.snapshot_id,
            commit_sha: state.commit_sha.clone(),
            file_count: state.files.len(),
            node_count: state.nodes.len(),
            edge_count: state.edges.len(),
            cycle_count: cycles.len(),
        });

        let modules = all_modules
            .iter()
            .map(|module| ModuleCouplingPoint {
                module: module.clone(),
                coupling_count: *maps.module_coupling.get(module).unwrap_or(&0),
            })
            .collect::<Vec<_>>();
        module_trend.push(ModuleCouplingTrendPoint {
            snapshot_id: state.snapshot_id,
            commit_sha: state.commit_sha.clone(),
            modules,
        });

        for module in &all_modules {
            let nodes = maps.module_nodes.get(module).cloned().unwrap_or_default();
            let deps = maps
                .module_dependencies
                .get(module)
                .cloned()
                .unwrap_or_default();
            let acc = accumulators.entry(module.clone()).or_default();
            acc.file_paths
                .extend(maps.module_files.get(module).into_iter().flatten().cloned());
            acc.max_coupling_count = acc
                .max_coupling_count
                .max(*maps.module_coupling.get(module).unwrap_or(&0));
            if cycle_modules.contains(module) {
                acc.cycle_participation_count += 1;
            }

            let prev_nodes = prev_module_nodes.get(module).cloned().unwrap_or_default();
            let prev_deps = prev_module_dependencies
                .get(module)
                .cloned()
                .unwrap_or_default();
            if !prev_nodes.is_empty() || !nodes.is_empty() {
                acc.symbol_churn += prev_nodes.symmetric_difference(&nodes).count();
            }
            if !prev_deps.is_empty() || !deps.is_empty() {
                acc.dependency_churn += prev_deps.symmetric_difference(&deps).count();
            }
            prev_module_nodes.insert(module.clone(), nodes);
            prev_module_dependencies.insert(module.clone(), deps);
        }
    }

    let mut module_churn = Vec::new();
    let mut hotspots = Vec::new();
    for (module, acc) in accumulators {
        let hotspot_score = acc.symbol_churn
            + acc.dependency_churn
            + acc.max_coupling_count
            + acc.cycle_participation_count;
        module_churn.push(ModuleChurnRecord {
            module: module.clone(),
            dependency_churn: acc.dependency_churn,
            symbol_churn: acc.symbol_churn,
            file_paths: sorted_strings(acc.file_paths),
            hotspot_score,
        });
        hotspots.push(ArchitecturalHotspotRecord {
            module,
            hotspot_score,
            symbol_churn: acc.symbol_churn,
            dependency_churn: acc.dependency_churn,
            max_coupling_count: acc.max_coupling_count,
            cycle_participation_count: acc.cycle_participation_count,
        });
    }

    module_churn.sort_by(|left, right| {
        right
            .hotspot_score
            .cmp(&left.hotspot_score)
            .then_with(|| left.module.cmp(&right.module))
    });
    hotspots.sort_by(|left, right| {
        right
            .hotspot_score
            .cmp(&left.hotspot_score)
            .then_with(|| left.module.cmp(&right.module))
    });

    (module_churn, hotspots, module_trend, timeline)
}

fn build_stability_indicators(
    symbol_churn: &[SymbolChurnRecord],
    dependency_churn: &[DependencyChurnRecord],
    hotspot_records: Vec<ArchitecturalHotspotRecord>,
) -> StabilityIndicators {
    let mut stable_symbols = symbol_churn
        .iter()
        .map(|record| SymbolStabilityRecord {
            qualified_name: record.qualified_name.clone(),
            stability_score: record.stability_score,
            change_count: record.change_count,
            lifetime_snapshots: record.lifetime_snapshots,
        })
        .collect::<Vec<_>>();
    stable_symbols.sort_by(|left, right| {
        right
            .stability_score
            .total_cmp(&left.stability_score)
            .then_with(|| left.change_count.cmp(&right.change_count))
            .then_with(|| left.qualified_name.cmp(&right.qualified_name))
    });

    let mut unstable_symbols = stable_symbols.clone();
    unstable_symbols.sort_by(|left, right| {
        left.stability_score
            .total_cmp(&right.stability_score)
            .then_with(|| right.change_count.cmp(&left.change_count))
            .then_with(|| left.qualified_name.cmp(&right.qualified_name))
    });

    let mut frequently_changing_dependencies = dependency_churn
        .iter()
        .filter(|record| record.change_count > 0 || record.removal_count > 0)
        .cloned()
        .collect::<Vec<_>>();
    frequently_changing_dependencies.sort_by(|left, right| {
        right
            .change_count
            .cmp(&left.change_count)
            .then_with(|| left.source_qn.cmp(&right.source_qn))
            .then_with(|| left.target_qn.cmp(&right.target_qn))
    });

    StabilityIndicators {
        stable_symbols: stable_symbols.into_iter().take(10).collect(),
        unstable_symbols: unstable_symbols.into_iter().take(10).collect(),
        frequently_changing_dependencies: frequently_changing_dependencies
            .into_iter()
            .take(10)
            .collect(),
        architectural_hotspots: hotspot_records.into_iter().take(10).collect(),
    }
}

fn build_storage_diagnostics(
    store: &Store,
    db_path: &str,
    states: &[SnapshotState],
) -> Result<StorageDiagnostics> {
    let mut unique_hashes = BTreeSet::new();
    let mut without_dedup = 0u64;
    let mut size_by_hash = BTreeMap::<String, u64>::new();
    let mut snapshot_file_memberships = 0usize;

    for state in states {
        snapshot_file_memberships += state.files.len();
        for file in &state.files {
            unique_hashes.insert(file.file_hash.clone());
            let size = file.size.unwrap_or_default().max(0) as u64;
            without_dedup = without_dedup.saturating_add(size);
            size_by_hash.entry(file.file_hash.clone()).or_insert(size);
        }
    }

    let with_dedup = size_by_hash.values().copied().sum::<u64>();
    let deduplication_ratio = if unique_hashes.is_empty() {
        1.0
    } else {
        snapshot_file_memberships as f64 / unique_hashes.len() as f64
    };
    let snapshot_density = if states.is_empty() {
        0.0
    } else {
        snapshot_file_memberships as f64 / states.len() as f64
    };

    let db_size_bytes = match std::path::Path::new(db_path).exists() {
        true => std::fs::metadata(db_path)
            .map(|metadata| metadata.len())
            .unwrap_or(store.database_size_bytes()?),
        false => store.database_size_bytes()?,
    };

    Ok(StorageDiagnostics {
        commits_stored: states.len(),
        snapshots_stored: states.len(),
        unique_file_hashes: unique_hashes.len(),
        snapshot_file_memberships,
        deduplication_ratio,
        db_size_bytes,
        snapshot_density,
        storage_growth_without_dedup_bytes: without_dedup,
        storage_growth_with_dedup_bytes: with_dedup,
    })
}

fn build_report_evidence(states: &[SnapshotState]) -> HistoryEvidence {
    build_evidence(
        states.iter().map(|state| state.snapshot_id),
        states.iter().map(|state| state.commit_sha.clone()),
        states
            .iter()
            .flat_map(|state| state.nodes.iter().map(node_identifier_for)),
        states
            .iter()
            .flat_map(|state| state.edges.iter().map(edge_identifier_for)),
        states
            .iter()
            .flat_map(|state| state.files.iter().map(|file| file.file_path.clone())),
    )
}

#[derive(Debug, Default)]
struct StateMaps {
    symbols: BTreeMap<String, Vec<HistoricalNode>>,
    files: BTreeMap<String, atlas_store_sqlite::StoredSnapshotFile>,
    nodes_by_file: BTreeMap<String, Vec<HistoricalNode>>,
    edges_by_file: BTreeMap<String, Vec<HistoricalEdge>>,
    dependencies: BTreeMap<(String, String, String, String), HistoricalEdge>,
    module_nodes: BTreeMap<String, BTreeSet<String>>,
    module_files: BTreeMap<String, BTreeSet<String>>,
    module_dependencies: BTreeMap<String, BTreeSet<(String, String)>>,
    module_coupling: BTreeMap<String, usize>,
    module_graph: BTreeMap<String, BTreeSet<String>>,
}

fn build_state_maps(state: &SnapshotState) -> StateMaps {
    let mut maps = StateMaps::default();
    let node_lookup = state
        .nodes
        .iter()
        .map(|node| (node.qualified_name.clone(), node))
        .collect::<BTreeMap<_, _>>();

    for file in &state.files {
        maps.files.insert(file.file_path.clone(), file.clone());
    }
    for node in &state.nodes {
        maps.symbols
            .entry(node.qualified_name.clone())
            .or_default()
            .push(node.clone());
        maps.nodes_by_file
            .entry(node.file_path.clone())
            .or_default()
            .push(node.clone());
        let module = module_key(&node.file_path);
        maps.module_nodes
            .entry(module.clone())
            .or_default()
            .insert(node.qualified_name.clone());
        maps.module_files
            .entry(module)
            .or_default()
            .insert(node.file_path.clone());
    }
    for edge in &state.edges {
        maps.edges_by_file
            .entry(edge.file_path.clone())
            .or_default()
            .push(edge.clone());
        maps.dependencies.insert(
            (
                edge.source_qn.clone(),
                edge.target_qn.clone(),
                edge.kind.clone(),
                edge.file_path.clone(),
            ),
            edge.clone(),
        );

        let source_module = node_lookup
            .get(&edge.source_qn)
            .map(|node| module_key(&node.file_path))
            .unwrap_or_else(|| module_key(&edge.file_path));
        let target_module = node_lookup
            .get(&edge.target_qn)
            .map(|node| module_key(&node.file_path))
            .unwrap_or_else(|| module_key(&edge.file_path));
        if source_module != target_module {
            maps.module_dependencies
                .entry(source_module.clone())
                .or_default()
                .insert((source_module.clone(), target_module.clone()));
            *maps
                .module_coupling
                .entry(source_module.clone())
                .or_default() += 1;
            *maps
                .module_coupling
                .entry(target_module.clone())
                .or_default() += 1;
            maps.module_graph
                .entry(source_module)
                .or_default()
                .insert(target_module);
        }
    }

    maps
}

fn symbol_signature_key(node: &HistoricalNode) -> String {
    format!(
        "{}|{}|{}|{}",
        node.file_path,
        node.params.as_deref().unwrap_or(""),
        node.return_type.as_deref().unwrap_or(""),
        node.modifiers.as_deref().unwrap_or(""),
    )
}

fn growth_for(points: &[TrendPoint], selector: impl Fn(&TrendPoint) -> usize) -> i64 {
    let Some(first) = points.first() else {
        return 0;
    };
    let Some(last) = points.last() else {
        return 0;
    };
    selector(last) as i64 - selector(first) as i64
}

fn strongly_connected_components(graph: &BTreeMap<String, BTreeSet<String>>) -> Vec<Vec<String>> {
    let mut adjacency = graph.clone();
    for targets in graph.values() {
        for target in targets {
            adjacency.entry(target.clone()).or_default();
        }
    }

    let mut index = 0usize;
    let mut index_map = BTreeMap::<String, usize>::new();
    let mut lowlink = BTreeMap::<String, usize>::new();
    let mut stack = Vec::<String>::new();
    let mut on_stack = BTreeSet::<String>::new();
    let mut components = Vec::<Vec<String>>::new();

    for node in adjacency.keys() {
        if !index_map.contains_key(node) {
            strong_connect(
                node,
                &adjacency,
                &mut index,
                &mut index_map,
                &mut lowlink,
                &mut stack,
                &mut on_stack,
                &mut components,
            );
        }
    }

    components
        .into_iter()
        .filter(|component| component.len() > 1)
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn strong_connect(
    node: &str,
    graph: &BTreeMap<String, BTreeSet<String>>,
    index: &mut usize,
    index_map: &mut BTreeMap<String, usize>,
    lowlink: &mut BTreeMap<String, usize>,
    stack: &mut Vec<String>,
    on_stack: &mut BTreeSet<String>,
    components: &mut Vec<Vec<String>>,
) {
    index_map.insert(node.to_owned(), *index);
    lowlink.insert(node.to_owned(), *index);
    *index += 1;
    stack.push(node.to_owned());
    on_stack.insert(node.to_owned());

    for neighbor in graph.get(node).into_iter().flatten() {
        if !index_map.contains_key(neighbor) {
            strong_connect(
                neighbor, graph, index, index_map, lowlink, stack, on_stack, components,
            );
            let neighbor_lowlink = lowlink[neighbor];
            let current = lowlink[node];
            lowlink.insert(node.to_owned(), current.min(neighbor_lowlink));
        } else if on_stack.contains(neighbor) {
            let neighbor_index = index_map[neighbor];
            let current = lowlink[node];
            lowlink.insert(node.to_owned(), current.min(neighbor_index));
        }
    }

    if lowlink[node] == index_map[node] {
        let mut component = Vec::new();
        while let Some(entry) = stack.pop() {
            on_stack.remove(&entry);
            component.push(entry.clone());
            if entry == node {
                break;
            }
        }
        component.sort();
        components.push(component);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_store_sqlite::{HistoricalEdge, HistoricalNode, StoredCommit, StoredSnapshotFile};

    fn open_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite");
        let store = Store::open(path.to_str().unwrap()).unwrap();
        (dir, store)
    }

    fn history_fixture_store() -> Store {
        let (_dir, store) = open_store();
        let repo_id = store.upsert_repo("/repo").unwrap();

        insert_snapshot(
            &store,
            repo_id,
            1,
            "a",
            1_700_000_000,
            vec!["src/a.rs::fn::stable"],
            vec![],
        );
        insert_snapshot(
            &store,
            repo_id,
            2,
            "b",
            1_700_000_100,
            vec!["src/a.rs::fn::stable", "src/b.rs::fn::flappy"],
            vec![("src/b.rs::fn::flappy", "src/a.rs::fn::stable")],
        );
        insert_snapshot(
            &store,
            repo_id,
            3,
            "c",
            1_700_000_200,
            vec!["src/a.rs::fn::stable"],
            vec![],
        );
        store
    }

    fn insert_snapshot(
        store: &Store,
        repo_id: i64,
        ordinal: usize,
        sha_prefix: &str,
        author_time: i64,
        qnames: Vec<&str>,
        edges: Vec<(&str, &str)>,
    ) {
        let sha = format!("{sha_prefix:0<40}");
        store
            .upsert_commit(&StoredCommit {
                commit_sha: sha.clone(),
                repo_id,
                parent_sha: None,
                indexed_ref: None,
                author_name: None,
                author_email: None,
                author_time,
                committer_time: author_time,
                subject: format!("commit {ordinal}"),
                message: None,
                indexed_at: String::new(),
            })
            .unwrap();
        let snapshot_id = store
            .insert_snapshot(
                repo_id,
                &sha,
                None,
                qnames.len() as i64,
                edges.len() as i64,
                1,
                1.0,
                0,
            )
            .unwrap();
        let file_hash = format!("hash-{ordinal}");
        let file_path = if ordinal == 2 { "src/b.rs" } else { "src/a.rs" };
        store
            .insert_snapshot_files(&[StoredSnapshotFile {
                snapshot_id,
                file_path: file_path.to_owned(),
                file_hash: file_hash.clone(),
                language: Some("rust".to_owned()),
                size: Some((10 * ordinal) as i64),
            }])
            .unwrap();
        let nodes = qnames
            .iter()
            .map(|qn| HistoricalNode {
                file_hash: file_hash.clone(),
                qualified_name: (*qn).to_owned(),
                kind: "function".to_owned(),
                name: qn.rsplit("::").next().unwrap_or("node").to_owned(),
                file_path: if qn.contains("src/b.rs") {
                    "src/b.rs".to_owned()
                } else {
                    "src/a.rs".to_owned()
                },
                line_start: Some(1),
                line_end: Some(5),
                language: Some("rust".to_owned()),
                parent_name: None,
                params: None,
                return_type: None,
                modifiers: None,
                is_test: false,
                extra_json: None,
            })
            .collect::<Vec<_>>();
        let historical_edges = edges
            .iter()
            .map(|(source, target)| HistoricalEdge {
                file_hash: file_hash.clone(),
                source_qn: (*source).to_owned(),
                target_qn: (*target).to_owned(),
                kind: "calls".to_owned(),
                file_path: "src/b.rs".to_owned(),
                line: Some(1),
                confidence: 1.0,
                confidence_tier: None,
                extra_json: None,
            })
            .collect::<Vec<_>>();
        store.insert_historical_nodes(&nodes).unwrap();
        store.insert_historical_edges(&historical_edges).unwrap();
        store
            .attach_snapshot_nodes(
                snapshot_id,
                &file_hash,
                &nodes
                    .iter()
                    .map(|node| node.qualified_name.clone())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        store
            .attach_snapshot_edges(
                snapshot_id,
                &file_hash,
                &historical_edges
                    .iter()
                    .map(|edge| {
                        (
                            edge.source_qn.clone(),
                            edge.target_qn.clone(),
                            edge.kind.clone(),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
            .unwrap();
    }

    #[test]
    fn churn_report_computes_dedup_ratio_and_trends() {
        let store = history_fixture_store();
        let report = compute_churn_report(&store, "/repo", ":memory:").unwrap();

        assert_eq!(report.summary.snapshot_count, 3);
        assert!(report.storage_diagnostics.deduplication_ratio >= 1.0);
        assert_eq!(report.trends.timeline.len(), 3);
        assert!(
            report
                .stability
                .unstable_symbols
                .iter()
                .any(|record| record.qualified_name == "src/b.rs::fn::flappy")
        );
    }
}
