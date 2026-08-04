//! Unit tests for `ReasoningEngine`.
//!
//! Shared graph-fixture helpers live here; per-family tests are split into
//! `similarity`, `removal`, `architecture`, `insights`, `metrics`, `patterns`,
//! and `risk`.

mod architecture;
mod insights;
mod metrics;
mod patterns;
mod removal;
mod risk;
mod similarity;

use super::{
    ArchitectureAnalysis, FileMetric, InsightsEngine, MetricDistribution, MetricsAnalysis,
    ModuleMetric, NodeMetric, RiskAssessmentAnalysis, RiskAssessmentTarget, RiskFactorContribution,
};
use atlas_core::{
    Edge, EdgeKind, InsightEvidence, InsightFinding, InsightLineRange, InsightSeverity, Node,
    NodeId, NodeKind, PackageOwner, PackageOwnerKind,
};
use atlas_store_sqlite::Store;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

fn make_store() -> Store {
    let mut store = Store::open(":memory:").unwrap();
    store.migrate().unwrap();
    store
}

fn node(id: i64, name: &str, qname: &str, file: &str, kind: NodeKind) -> Node {
    Node {
        id: NodeId(id),
        kind,
        name: name.to_owned(),
        qualified_name: qname.to_owned(),
        file_path: file.to_owned(),
        line_start: 1,
        line_end: 10,
        language: "rust".to_owned(),
        parent_name: None,
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: String::new(),
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    }
}

fn edge(src: &str, tgt: &str, kind: EdgeKind, file: &str) -> Edge {
    Edge {
        id: 0,
        kind,
        source_qn: src.to_owned(),
        target_qn: tgt.to_owned(),
        file_path: file.to_owned(),
        line: None,
        confidence: 1.0,
        confidence_tier: Some("high".to_owned()),
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    }
}

fn seed_graph(store: &mut Store, nodes: Vec<Node>, edges: Vec<Edge>) {
    let mut files: std::collections::HashMap<String, (Vec<Node>, Vec<Edge>)> = Default::default();
    for node in nodes {
        files
            .entry(node.file_path.clone())
            .or_default()
            .0
            .push(node);
    }
    for edge in edges {
        files
            .entry(edge.file_path.clone())
            .or_default()
            .1
            .push(edge);
    }
    for (path, (nodes, edges)) in files {
        let language = nodes.first().map(|node| node.language.clone());
        store
            .replace_file_graph(&path, "hash", language.as_deref(), None, &nodes, &edges)
            .unwrap();
    }
}

fn attach_owner(store: &mut Store, path: &str, manifest_path: &str) {
    let root = manifest_path
        .rsplit_once('/')
        .map(|(prefix, _)| prefix)
        .unwrap_or("");
    let owner = PackageOwner {
        owner_id: format!("cargo:{manifest_path}"),
        kind: PackageOwnerKind::Cargo,
        root: root.to_owned(),
        manifest_path: manifest_path.to_owned(),
        package_name: manifest_path.split('/').rev().nth(1).map(str::to_owned),
    };
    store.upsert_file_owner(path, Some(&owner)).unwrap();
}

fn sample_insight(id: &str, file: &str, qname: &str, severity: InsightSeverity) -> InsightFinding {
    InsightFinding {
        id: id.to_owned(),
        title: format!("finding-{id}"),
        severity,
        category: "metrics".to_owned(),
        message: format!("message-{id}"),
        evidence: vec![InsightEvidence {
            file_path: Some(file.to_owned()),
            qualified_name: Some(qname.to_owned()),
            node_kind: Some("function".to_owned()),
            edge_kind: None,
            line_range: Some(InsightLineRange {
                start_line: 10,
                end_line: 20,
            }),
            confidence_tier: None,
        }],
        ranking_reason: format!("reason-{id}"),
        details: None,
        score: 10.0,
    }
}

fn make_repo_root() -> PathBuf {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "atlas-reasoning-metrics-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

fn write_repo_file(repo_root: &Path, rel_path: &str, content: &str) {
    let path = repo_root.join(rel_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn read_fingerprint_cache_json(repo_root: &Path) -> serde_json::Value {
    let path = repo_root
        .join(atlas_engine::paths::ATLAS_DIR)
        .join("insights-fingerprint-cache.v1.json");
    let raw = fs::read_to_string(path).unwrap();
    serde_json::from_str(&raw).unwrap()
}

fn insights_engine<'a>(store: &'a Store) -> InsightsEngine<'a> {
    InsightsEngine::new(store, atlas_engine::config::InsightsConfig::default())
        .unwrap()
        .with_generated_at("2026-05-11T12:00:00Z")
}

fn insights_engine_with_config<'a>(
    store: &'a Store,
    config: atlas_engine::config::InsightsConfig,
) -> InsightsEngine<'a> {
    InsightsEngine::new(store, config)
        .unwrap()
        .with_generated_at("2026-05-11T12:00:00Z")
}

fn find_node_metric<'a>(analysis: &'a MetricsAnalysis, qname: &str) -> &'a NodeMetric {
    analysis
        .metrics
        .node_metrics
        .iter()
        .find(|metric| metric.node.qualified_name == qname)
        .unwrap_or_else(|| panic!("missing node metric for {qname}"))
}

fn find_file_metric<'a>(analysis: &'a MetricsAnalysis, file_path: &str) -> &'a FileMetric {
    analysis
        .metrics
        .file_metrics
        .iter()
        .find(|metric| metric.file_path == file_path)
        .unwrap_or_else(|| panic!("missing file metric for {file_path}"))
}

fn find_module_metric<'a>(analysis: &'a MetricsAnalysis, module_id: &str) -> &'a ModuleMetric {
    analysis
        .metrics
        .module_metrics
        .iter()
        .find(|metric| metric.module_id == module_id)
        .unwrap_or_else(|| panic!("missing module metric for {module_id}"))
}

fn find_distribution<'a>(
    analysis: &'a MetricsAnalysis,
    metric_name: &str,
) -> &'a MetricDistribution {
    analysis
        .metrics
        .distributions
        .iter()
        .find(|distribution| distribution.metric_name == metric_name)
        .unwrap_or_else(|| panic!("missing distribution for {metric_name}"))
}

fn find_architecture_finding<'a>(
    analysis: &'a ArchitectureAnalysis,
    category: &str,
) -> &'a InsightFinding {
    analysis
        .report
        .findings
        .iter()
        .find(|finding| finding.category == category)
        .unwrap_or_else(|| panic!("missing architecture finding for {category}"))
}

fn pattern_findings<'a>(
    report: &'a atlas_core::PatternReport,
    category: &str,
) -> Vec<&'a InsightFinding> {
    report
        .findings
        .iter()
        .filter(|finding| finding.category == category)
        .collect()
}

fn assess_risk(
    engine: &InsightsEngine<'_>,
    repo_root: &Path,
    symbol: &str,
) -> RiskAssessmentAnalysis {
    engine
        .assess_risk(
            repo_root,
            RiskAssessmentTarget::Symbol {
                symbol: symbol.to_owned(),
            },
        )
        .unwrap_or_else(|err| panic!("risk assessment failed for {symbol}: {err}"))
}

fn factor<'a>(analysis: &'a RiskAssessmentAnalysis, name: &str) -> &'a RiskFactorContribution {
    analysis
        .factor_contributions
        .iter()
        .find(|factor| factor.factor == name)
        .unwrap_or_else(|| panic!("missing risk factor {name}"))
}
