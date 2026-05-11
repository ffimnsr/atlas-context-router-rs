use super::*;
use atlas_core::{
    Edge, EdgeKind, FreshnessWarning, GraphStats, ImpactClass, InsightEvidence, InsightFinding,
    InsightLineRange, InsightSeverity, Node, NodeId, NodeKind, PackageOwner, PackageOwnerKind,
    ProvenanceMeta, ReferenceScope, SafetyBand,
};
use atlas_store_sqlite::Store;
use serde_json::json;
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

#[test]
fn removal_simple_call_graph() {
    let mut store = make_store();
    let nodes = vec![
        node(0, "fn_a", "src/a.rs::fn_a", "src/a.rs", NodeKind::Function),
        node(0, "fn_b", "src/b.rs::fn_b", "src/b.rs", NodeKind::Function),
    ];
    let edges = vec![edge(
        "src/b.rs::fn_b",
        "src/a.rs::fn_a",
        EdgeKind::Calls,
        "src/b.rs",
    )];
    seed_graph(&mut store, nodes, edges);

    let engine = ReasoningEngine::new(&store);
    let result = engine
        .analyze_removal(&["src/a.rs::fn_a"], None, None)
        .unwrap();

    assert!(!result.seed.is_empty(), "seed should resolve");
    assert!(
        result
            .impacted_symbols
            .iter()
            .any(|im| im.node.qualified_name == "src/b.rs::fn_b"),
        "fn_b should be in impacted symbols"
    );
}

#[test]
fn removal_cyclic_graph() {
    let mut store = make_store();
    let nodes = vec![
        node(0, "fn_a", "src/a.rs::fn_a", "src/a.rs", NodeKind::Function),
        node(0, "fn_b", "src/b.rs::fn_b", "src/b.rs", NodeKind::Function),
    ];
    let edges = vec![
        edge(
            "src/a.rs::fn_a",
            "src/b.rs::fn_b",
            EdgeKind::Calls,
            "src/a.rs",
        ),
        edge(
            "src/b.rs::fn_b",
            "src/a.rs::fn_a",
            EdgeKind::Calls,
            "src/b.rs",
        ),
    ];
    seed_graph(&mut store, nodes, edges);

    let engine = ReasoningEngine::new(&store);
    let result = engine
        .analyze_removal(&["src/a.rs::fn_a"], Some(5), Some(100))
        .unwrap();

    assert!(
        result
            .impacted_symbols
            .iter()
            .any(|im| im.node.qualified_name == "src/b.rs::fn_b")
    );
}

#[test]
fn removal_normalizes_function_alias_qname() {
    let mut store = make_store();
    let nodes = vec![
        node(
            0,
            "fn_a",
            "src/a.rs::fn::fn_a",
            "src/a.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "fn_b",
            "src/b.rs::fn::fn_b",
            "src/b.rs",
            NodeKind::Function,
        ),
    ];
    let edges = vec![edge(
        "src/b.rs::fn::fn_b",
        "src/a.rs::fn::fn_a",
        EdgeKind::Calls,
        "src/b.rs",
    )];
    seed_graph(&mut store, nodes, edges);

    let engine = ReasoningEngine::new(&store);
    let result = engine
        .analyze_removal(&["src/a.rs::function::fn_a"], None, None)
        .unwrap();

    assert_eq!(result.seed[0].qualified_name, "src/a.rs::fn::fn_a");
    assert!(
        result
            .impacted_symbols
            .iter()
            .any(|im| im.node.qualified_name == "src/b.rs::fn::fn_b")
    );
}

#[test]
fn removal_containment_only_edges_are_weak_not_probable() {
    let mut store = make_store();
    let nodes = vec![
        node(0, "fn_a", "src/a.rs::fn_a", "src/a.rs", NodeKind::Function),
        node(0, "mod_a", "src/a.rs", "src/a.rs", NodeKind::File),
    ];
    let edges = vec![edge(
        "src/a.rs",
        "src/a.rs::fn_a",
        EdgeKind::Contains,
        "src/a.rs",
    )];
    seed_graph(&mut store, nodes, edges);

    let engine = ReasoningEngine::new(&store);
    let result = engine
        .analyze_removal(&["src/a.rs::fn_a"], None, None)
        .unwrap();

    for impacted in &result.impacted_symbols {
        if impacted.node.qualified_name == "src/a.rs" {
            assert_ne!(
                impacted.impact_class,
                ImpactClass::Definite,
                "containment parent must not be Definite impact"
            );
            assert_ne!(
                impacted.impact_class,
                ImpactClass::Probable,
                "containment parent must not be Probable impact — got inflated result"
            );
        }
    }
}

#[test]
fn dead_code_private_function_flagged() {
    let mut store = make_store();
    let mut priv_node = node(
        0,
        "unused_fn",
        "src/a.rs::unused_fn",
        "src/a.rs",
        NodeKind::Function,
    );
    priv_node.modifiers = None;
    seed_graph(&mut store, vec![priv_node], vec![]);

    let engine = ReasoningEngine::new(&store);
    let candidates = engine.detect_dead_code(&[], None, None, &[]).unwrap();
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.node.qualified_name == "src/a.rs::unused_fn"),
        "private unused_fn should be dead-code candidate"
    );
}

#[test]
fn insights_engine_filters_ignored_findings_and_keeps_metadata() {
    let summary = InsightsGraphSummary {
        graph_stats: GraphStats {
            file_count: 1,
            node_count: 2,
            edge_count: 3,
            nodes_by_kind: vec![("function".to_owned(), 2)],
            languages: vec!["rust".to_owned()],
            last_indexed_at: Some("2026-05-11T00:00:00Z".to_owned()),
        },
        atlas_provenance: ProvenanceMeta {
            indexed_file_count: 1,
            last_indexed_at: Some("2026-05-11T00:00:00Z".to_owned()),
        },
        atlas_freshness: Some(FreshnessWarning {
            stale: true,
            changed_files: vec!["src/lib.rs".to_owned()],
            stale_result_files: vec!["src/lib.rs".to_owned()],
            warning: "stale".to_owned(),
            suggested_recovery: vec!["refresh".to_owned()],
        }),
    };
    let config = atlas_engine::config::InsightsConfig {
        ignore_files: vec!["tests".to_owned()],
        ignore_modules: vec!["crate::ignored".to_owned()],
        ..Default::default()
    };

    let engine =
        InsightsEngine::from_summary(summary, config).with_generated_at("2026-05-11T12:00:00Z");
    let report = engine.metrics_report(vec![
        sample_insight(
            "keep",
            "src/lib.rs",
            "crate::kept::compute",
            InsightSeverity::High,
        ),
        sample_insight(
            "drop-file",
            "tests/lib.rs",
            "crate::tests::helper",
            InsightSeverity::High,
        ),
        sample_insight(
            "drop-module",
            "src/ignored.rs",
            "crate::ignored::helper",
            InsightSeverity::High,
        ),
    ]);

    assert_eq!(report.summary.total_findings, 1);
    assert_eq!(report.findings[0].id, "keep");
    assert_eq!(report.summary.generated_at, "2026-05-11T12:00:00Z");
    assert!(report.atlas_freshness.is_some());
    assert_eq!(report.atlas_provenance.indexed_file_count, 1);
}

#[test]
fn insights_engine_sorts_and_limits_findings() {
    let summary = InsightsGraphSummary {
        graph_stats: GraphStats {
            file_count: 1,
            node_count: 1,
            edge_count: 0,
            nodes_by_kind: vec![],
            languages: vec!["rust".to_owned()],
            last_indexed_at: None,
        },
        atlas_provenance: ProvenanceMeta {
            indexed_file_count: 1,
            last_indexed_at: None,
        },
        atlas_freshness: None,
    };
    let config = atlas_engine::config::InsightsConfig {
        max_findings: 2,
        ..Default::default()
    };

    let mut high = sample_insight("high", "src/a.rs", "crate::alpha", InsightSeverity::High);
    high.score = 50.0;
    let mut low = sample_insight("low", "src/c.rs", "crate::gamma", InsightSeverity::Low);
    low.score = 99.0;
    let mut high_later = sample_insight(
        "high-later",
        "src/b.rs",
        "crate::beta",
        InsightSeverity::High,
    );
    high_later.score = 40.0;

    let engine = InsightsEngine::from_summary(summary, config);
    let report = engine.pattern_report(vec![low, high_later, high]);

    assert_eq!(report.findings.len(), 2);
    assert_eq!(report.findings[0].id, "high");
    assert_eq!(report.findings[1].id, "high-later");
}

#[test]
fn architecture_detects_scc_cycles_with_deterministic_path() {
    let repo_root = make_repo_root();
    write_repo_file(&repo_root, "src/a/mod.rs", "pub fn alpha() {}\n");
    write_repo_file(&repo_root, "src/b/mod.rs", "pub fn beta() {}\n");

    let alpha = node(
        0,
        "alpha",
        "src/a/mod.rs::fn::alpha",
        "src/a/mod.rs",
        NodeKind::Function,
    );
    let beta = node(
        0,
        "beta",
        "src/b/mod.rs::fn::beta",
        "src/b/mod.rs",
        NodeKind::Function,
    );
    let edges = vec![
        edge(
            "src/a/mod.rs::fn::alpha",
            "src/b/mod.rs::fn::beta",
            EdgeKind::Calls,
            "src/a/mod.rs",
        ),
        edge(
            "src/b/mod.rs::fn::beta",
            "src/a/mod.rs::fn::alpha",
            EdgeKind::Calls,
            "src/b/mod.rs",
        ),
    ];

    let mut store = make_store();
    seed_graph(&mut store, vec![alpha, beta], edges);

    let analysis = insights_engine(&store)
        .analyze_architecture(&repo_root)
        .unwrap();
    let finding = find_architecture_finding(&analysis, "architecture_cycle");

    assert_eq!(finding.severity, InsightSeverity::Medium);
    assert_eq!(
        finding.details.as_ref().unwrap()["classification"],
        json!("local")
    );
    assert_eq!(
        finding.details.as_ref().unwrap()["cycle_path"],
        json!(["module:src/a", "module:src/b", "module:src/a"])
    );
}

#[test]
fn architecture_classifies_cross_module_cycles() {
    let repo_root = make_repo_root();
    write_repo_file(&repo_root, "packages/foo/src/lib.rs", "pub fn alpha() {}\n");
    write_repo_file(&repo_root, "packages/bar/src/lib.rs", "pub fn beta() {}\n");

    let alpha = node(
        0,
        "alpha",
        "packages/foo/src/lib.rs::fn::alpha",
        "packages/foo/src/lib.rs",
        NodeKind::Function,
    );
    let beta = node(
        0,
        "beta",
        "packages/bar/src/lib.rs::fn::beta",
        "packages/bar/src/lib.rs",
        NodeKind::Function,
    );
    let edges = vec![
        edge(
            "packages/foo/src/lib.rs::fn::alpha",
            "packages/bar/src/lib.rs::fn::beta",
            EdgeKind::Calls,
            "packages/foo/src/lib.rs",
        ),
        edge(
            "packages/bar/src/lib.rs::fn::beta",
            "packages/foo/src/lib.rs::fn::alpha",
            EdgeKind::Calls,
            "packages/bar/src/lib.rs",
        ),
    ];

    let mut store = make_store();
    seed_graph(&mut store, vec![alpha.clone(), beta.clone()], edges);
    attach_owner(&mut store, &alpha.file_path, "packages/foo/Cargo.toml");
    attach_owner(&mut store, &beta.file_path, "packages/bar/Cargo.toml");

    let analysis = insights_engine(&store)
        .analyze_architecture(&repo_root)
        .unwrap();
    let finding = find_architecture_finding(&analysis, "architecture_cycle");

    assert_eq!(finding.severity, InsightSeverity::High);
    assert_eq!(
        finding.details.as_ref().unwrap()["classification"],
        json!("cross-module")
    );
}

#[test]
fn architecture_valid_layer_rule_allows_dependency() {
    let repo_root = make_repo_root();
    write_repo_file(&repo_root, "src/api/mod.rs", "pub fn handler() {}\n");
    write_repo_file(&repo_root, "src/domain/mod.rs", "pub fn service() {}\n");

    let api = node(
        0,
        "handler",
        "src/api/mod.rs::fn::handler",
        "src/api/mod.rs",
        NodeKind::Function,
    );
    let domain = node(
        0,
        "service",
        "src/domain/mod.rs::fn::service",
        "src/domain/mod.rs",
        NodeKind::Function,
    );
    let mut store = make_store();
    seed_graph(
        &mut store,
        vec![api, domain],
        vec![edge(
            "src/api/mod.rs::fn::handler",
            "src/domain/mod.rs::fn::service",
            EdgeKind::Calls,
            "src/api/mod.rs",
        )],
    );

    let config = atlas_engine::config::InsightsConfig {
        layer_rules: vec![
            atlas_engine::config::InsightsLayerRule {
                name: "api".to_owned(),
                path_prefixes: vec!["src/api".to_owned()],
                module_prefixes: vec![],
            },
            atlas_engine::config::InsightsLayerRule {
                name: "domain".to_owned(),
                path_prefixes: vec!["src/domain".to_owned()],
                module_prefixes: vec![],
            },
        ],
        ..Default::default()
    };

    let analysis = insights_engine_with_config(&store, config)
        .analyze_architecture(&repo_root)
        .unwrap();

    assert!(
        !analysis
            .report
            .findings
            .iter()
            .any(|finding| finding.category == "layer_violation")
    );
}

#[test]
fn architecture_invalid_layer_rule_reports_violation() {
    let repo_root = make_repo_root();
    write_repo_file(&repo_root, "src/api/mod.rs", "pub fn dto() {}\n");
    write_repo_file(&repo_root, "src/domain/mod.rs", "pub fn service() {}\n");

    let api = node(
        0,
        "dto",
        "src/api/mod.rs::fn::dto",
        "src/api/mod.rs",
        NodeKind::Function,
    );
    let domain = node(
        0,
        "service",
        "src/domain/mod.rs::fn::service",
        "src/domain/mod.rs",
        NodeKind::Function,
    );
    let mut store = make_store();
    seed_graph(
        &mut store,
        vec![api, domain],
        vec![edge(
            "src/domain/mod.rs::fn::service",
            "src/api/mod.rs::fn::dto",
            EdgeKind::Calls,
            "src/domain/mod.rs",
        )],
    );

    let config = atlas_engine::config::InsightsConfig {
        layer_rules: vec![
            atlas_engine::config::InsightsLayerRule {
                name: "api".to_owned(),
                path_prefixes: vec!["src/api".to_owned()],
                module_prefixes: vec![],
            },
            atlas_engine::config::InsightsLayerRule {
                name: "domain".to_owned(),
                path_prefixes: vec!["src/domain".to_owned()],
                module_prefixes: vec![],
            },
        ],
        ..Default::default()
    };

    let analysis = insights_engine_with_config(&store, config)
        .analyze_architecture(&repo_root)
        .unwrap();
    let finding = find_architecture_finding(&analysis, "layer_violation");

    assert_eq!(finding.severity, InsightSeverity::High);
    assert_eq!(
        finding.details.as_ref().unwrap()["source_layer"],
        json!("domain")
    );
    assert_eq!(
        finding.details.as_ref().unwrap()["target_layer"],
        json!("api")
    );
}

#[test]
fn architecture_detects_high_coupling_modules() {
    let repo_root = make_repo_root();
    write_repo_file(&repo_root, "src/core/a.rs", "pub fn a() {}\n");
    write_repo_file(&repo_root, "src/core/b.rs", "pub fn b() {}\n");
    write_repo_file(&repo_root, "src/ext/c.rs", "pub fn c() {}\n");
    write_repo_file(&repo_root, "src/ext/d.rs", "pub fn d() {}\n");

    let a = node(
        0,
        "a",
        "src/core/a.rs::fn::a",
        "src/core/a.rs",
        NodeKind::Function,
    );
    let b = node(
        0,
        "b",
        "src/core/b.rs::fn::b",
        "src/core/b.rs",
        NodeKind::Function,
    );
    let c = node(
        0,
        "c",
        "src/ext/c.rs::fn::c",
        "src/ext/c.rs",
        NodeKind::Function,
    );
    let d = node(
        0,
        "d",
        "src/ext/d.rs::fn::d",
        "src/ext/d.rs",
        NodeKind::Function,
    );
    let edges = vec![
        edge(
            "src/core/a.rs::fn::a",
            "src/ext/c.rs::fn::c",
            EdgeKind::Calls,
            "src/core/a.rs",
        ),
        edge(
            "src/core/b.rs::fn::b",
            "src/ext/d.rs::fn::d",
            EdgeKind::Calls,
            "src/core/b.rs",
        ),
    ];

    let mut store = make_store();
    seed_graph(&mut store, vec![a, b, c, d], edges);

    let config = atlas_engine::config::InsightsConfig {
        high_coupling: 1,
        ..Default::default()
    };
    let analysis = insights_engine_with_config(&store, config)
        .analyze_architecture(&repo_root)
        .unwrap();

    assert!(analysis.report.findings.iter().any(|finding| {
        finding.category == "architecture_module_health" && finding.id.contains("module:src/core")
    }));
}

#[test]
fn architecture_ignored_module_is_excluded() {
    let repo_root = make_repo_root();
    write_repo_file(&repo_root, "src/ignored/a.rs", "pub fn a() {}\n");
    write_repo_file(&repo_root, "src/ignored/b.rs", "pub fn b() {}\n");

    let a = node(
        0,
        "a",
        "src/ignored/a.rs::fn::a",
        "src/ignored/a.rs",
        NodeKind::Function,
    );
    let b = node(
        0,
        "b",
        "src/ignored/b.rs::fn::b",
        "src/ignored/b.rs",
        NodeKind::Function,
    );
    let mut store = make_store();
    seed_graph(
        &mut store,
        vec![a, b],
        vec![
            edge(
                "src/ignored/a.rs::fn::a",
                "src/ignored/b.rs::fn::b",
                EdgeKind::Calls,
                "src/ignored/a.rs",
            ),
            edge(
                "src/ignored/b.rs::fn::b",
                "src/ignored/a.rs::fn::a",
                EdgeKind::Calls,
                "src/ignored/b.rs",
            ),
        ],
    );

    let config = atlas_engine::config::InsightsConfig {
        ignore_modules: vec!["module:src/ignored".to_owned()],
        ..Default::default()
    };
    let analysis = insights_engine_with_config(&store, config)
        .analyze_architecture(&repo_root)
        .unwrap();

    assert!(analysis.report.findings.is_empty());
    assert!(analysis.modules.is_empty());
    assert!(analysis.edges.is_empty());
}

#[test]
fn dead_code_exclude_kinds_removes_matching_candidates() {
    let mut store = make_store();
    let mut fn_node = node(
        0,
        "unused_fn",
        "src/a.rs::unused_fn",
        "src/a.rs",
        NodeKind::Function,
    );
    fn_node.modifiers = None;
    let mut const_node = node(
        1,
        "UNUSED_CONST",
        "src/a.rs::UNUSED_CONST",
        "src/a.rs",
        NodeKind::Constant,
    );
    const_node.modifiers = None;
    seed_graph(&mut store, vec![fn_node, const_node], vec![]);

    let engine = ReasoningEngine::new(&store);
    let candidates = engine
        .detect_dead_code(&[], None, None, &[NodeKind::Constant])
        .unwrap();
    assert!(
        !candidates
            .iter()
            .any(|candidate| candidate.node.kind == NodeKind::Constant),
        "constants should be filtered out by exclude_kinds"
    );
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.node.kind == NodeKind::Function),
        "functions should still appear when only constants are excluded"
    );
}

#[test]
fn dead_code_exported_function_not_flagged() {
    let mut store = make_store();
    let mut pub_node = node(
        0,
        "pub_fn",
        "src/a.rs::pub_fn",
        "src/a.rs",
        NodeKind::Function,
    );
    pub_node.modifiers = Some("pub".to_owned());
    seed_graph(&mut store, vec![pub_node], vec![]);

    let engine = ReasoningEngine::new(&store);
    let candidates = engine.detect_dead_code(&[], None, None, &[]).unwrap();
    assert!(
        !candidates
            .iter()
            .any(|candidate| candidate.node.qualified_name == "src/a.rs::pub_fn"),
        "pub function should not be flagged"
    );
}

#[test]
fn dead_code_entrypoint_suppressed() {
    let mut store = make_store();
    let main_node = node(
        0,
        "main",
        "src/main.rs::main",
        "src/main.rs",
        NodeKind::Function,
    );
    seed_graph(&mut store, vec![main_node], vec![]);

    let engine = ReasoningEngine::new(&store);
    let candidates = engine.detect_dead_code(&[], None, None, &[]).unwrap();
    assert!(
        !candidates
            .iter()
            .any(|candidate| candidate.node.name == "main"),
        "main entrypoint should be suppressed"
    );
}

#[test]
fn rename_same_file_radius() {
    let mut store = make_store();
    let nodes = vec![
        node(0, "fn_a", "src/a.rs::fn_a", "src/a.rs", NodeKind::Function),
        node(
            0,
            "fn_caller",
            "src/a.rs::fn_caller",
            "src/a.rs",
            NodeKind::Function,
        ),
    ];
    let edges = vec![edge(
        "src/a.rs::fn_caller",
        "src/a.rs::fn_a",
        EdgeKind::Calls,
        "src/a.rs",
    )];
    seed_graph(&mut store, nodes, edges);

    let engine = ReasoningEngine::new(&store);
    let result = engine
        .preview_rename_radius("src/a.rs::fn_a", "fn_a_renamed")
        .unwrap();
    assert!(
        result
            .affected_references
            .iter()
            .any(|reference| reference.scope == ReferenceScope::SameFile),
        "caller in same file should appear as SameFile reference"
    );
}

#[test]
fn rename_cross_module_radius() {
    let mut store = make_store();
    let nodes = vec![
        node(
            0,
            "fn_a",
            "module_a/lib.rs::fn_a",
            "module_a/lib.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "fn_b",
            "module_b/lib.rs::fn_b",
            "module_b/lib.rs",
            NodeKind::Function,
        ),
    ];
    let edges = vec![edge(
        "module_b/lib.rs::fn_b",
        "module_a/lib.rs::fn_a",
        EdgeKind::Calls,
        "module_b/lib.rs",
    )];
    seed_graph(&mut store, nodes, edges);

    let engine = ReasoningEngine::new(&store);
    let result = engine
        .preview_rename_radius("module_a/lib.rs::fn_a", "fn_a_v2")
        .unwrap();
    assert!(
        result
            .affected_references
            .iter()
            .any(|reference| reference.scope == ReferenceScope::CrossModule),
        "caller in different module dir should be CrossModule"
    );
}

#[test]
fn rename_radius_normalizes_function_alias_qname() {
    let mut store = make_store();
    let nodes = vec![
        node(
            0,
            "fn_a",
            "src/a.rs::fn::fn_a",
            "src/a.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "fn_caller",
            "src/a.rs::fn::fn_caller",
            "src/a.rs",
            NodeKind::Function,
        ),
    ];
    let edges = vec![edge(
        "src/a.rs::fn::fn_caller",
        "src/a.rs::fn::fn_a",
        EdgeKind::Calls,
        "src/a.rs",
    )];
    seed_graph(&mut store, nodes, edges);

    let engine = ReasoningEngine::new(&store);
    let result = engine
        .preview_rename_radius("src/a.rs::function::fn_a", "fn_a_renamed")
        .unwrap();

    assert_eq!(result.target.qualified_name, "src/a.rs::fn::fn_a");
    assert!(
        result
            .affected_references
            .iter()
            .any(|reference| reference.node.qualified_name == "src/a.rs::fn::fn_caller")
    );
}

#[test]
fn dependency_removal_blocked_by_reference() {
    let mut store = make_store();
    let nodes = vec![
        node(
            0,
            "dep_a",
            "src/a.rs::dep_a",
            "src/a.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "consumer",
            "src/b.rs::consumer",
            "src/b.rs",
            NodeKind::Function,
        ),
    ];
    let edges = vec![edge(
        "src/b.rs::consumer",
        "src/a.rs::dep_a",
        EdgeKind::Calls,
        "src/b.rs",
    )];
    seed_graph(&mut store, nodes, edges);

    let engine = ReasoningEngine::new(&store);
    let result = engine.check_dependency_removal("src/a.rs::dep_a").unwrap();
    assert!(
        !result.removable,
        "dep_a is still referenced — not removable"
    );
    assert!(!result.blocking_references.is_empty());
}

#[test]
fn dependency_removal_normalizes_function_alias_qname() {
    let mut store = make_store();
    let nodes = vec![
        node(
            0,
            "dep_a",
            "src/a.rs::fn::dep_a",
            "src/a.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "consumer",
            "src/b.rs::fn::consumer",
            "src/b.rs",
            NodeKind::Function,
        ),
    ];
    let edges = vec![edge(
        "src/b.rs::fn::consumer",
        "src/a.rs::fn::dep_a",
        EdgeKind::Calls,
        "src/b.rs",
    )];
    seed_graph(&mut store, nodes, edges);

    let engine = ReasoningEngine::new(&store);
    let result = engine
        .check_dependency_removal("src/a.rs::function::dep_a")
        .unwrap();

    assert_eq!(result.target_qname, "src/a.rs::fn::dep_a");
    assert!(!result.blocking_references.is_empty());
}

#[test]
fn test_adjacency_missing_for_changed_symbol() {
    let mut store = make_store();
    let no_test_node = node(
        0,
        "fn_x",
        "src/lib.rs::fn_x",
        "src/lib.rs",
        NodeKind::Function,
    );
    seed_graph(&mut store, vec![no_test_node], vec![]);

    let engine = ReasoningEngine::new(&store);
    let result = engine.find_test_adjacency("src/lib.rs::fn_x").unwrap();
    assert_eq!(result.coverage_strength, atlas_core::CoverageStrength::None);
    assert!(result.recommendation.is_some());
}

#[test]
fn test_adjacency_normalizes_function_alias_qname() {
    let mut store = make_store();
    let target = node(
        0,
        "fn_x",
        "src/lib.rs::fn::fn_x",
        "src/lib.rs",
        NodeKind::Function,
    );
    let test = node(
        0,
        "fn_x_test",
        "tests/lib.rs::test::fn_x_test",
        "tests/lib.rs",
        NodeKind::Test,
    );
    let edges = vec![edge(
        "tests/lib.rs::test::fn_x_test",
        "src/lib.rs::fn::fn_x",
        EdgeKind::Tests,
        "tests/lib.rs",
    )];
    seed_graph(&mut store, vec![target, test], edges);

    let engine = ReasoningEngine::new(&store);
    let result = engine
        .find_test_adjacency("src/lib.rs::function::fn_x")
        .unwrap();

    assert_eq!(result.symbol.qualified_name, "src/lib.rs::fn::fn_x");
    assert_eq!(
        result.coverage_strength,
        atlas_core::CoverageStrength::Direct
    );
}

#[test]
fn test_adjacency_indirect_through_caller_tests() {
    let mut store = make_store();
    let target = node(
        0,
        "inner",
        "src/lib.rs::fn::inner",
        "src/lib.rs",
        NodeKind::Function,
    );
    let caller = node(
        0,
        "outer",
        "src/lib.rs::fn::outer",
        "src/lib.rs",
        NodeKind::Function,
    );
    let test_fn = node(
        0,
        "test_outer",
        "tests/lib.rs::test::test_outer",
        "tests/lib.rs",
        NodeKind::Test,
    );
    let edges = vec![
        edge(
            "src/lib.rs::fn::outer",
            "src/lib.rs::fn::inner",
            EdgeKind::Calls,
            "src/lib.rs",
        ),
        edge(
            "tests/lib.rs::test::test_outer",
            "src/lib.rs::fn::outer",
            EdgeKind::Tests,
            "tests/lib.rs",
        ),
    ];
    seed_graph(&mut store, vec![target, caller, test_fn], edges);

    let engine = ReasoningEngine::new(&store);
    let result = engine.find_test_adjacency("src/lib.rs::fn::inner").unwrap();
    assert_eq!(
        result.coverage_strength,
        atlas_core::CoverageStrength::IndirectThroughCallers
    );
    assert!(result.recommendation.is_some());
}

#[test]
fn refactor_safety_sanity_checks() {
    let mut store = make_store();
    let solo = node(
        0,
        "solo_fn",
        "src/a.rs::solo_fn",
        "src/a.rs",
        NodeKind::Function,
    );
    seed_graph(&mut store, vec![solo], vec![]);

    let engine = ReasoningEngine::new(&store);
    let result = engine.score_refactor_safety("src/a.rs::solo_fn").unwrap();
    assert_eq!(result.safety.band, SafetyBand::Safe);
    assert_eq!(result.coverage_strength, atlas_core::CoverageStrength::None);
}

#[test]
fn refactor_safety_normalizes_function_alias_qname() {
    let mut store = make_store();
    let solo = node(
        0,
        "solo_fn",
        "src/a.rs::fn::solo_fn",
        "src/a.rs",
        NodeKind::Function,
    );
    seed_graph(&mut store, vec![solo], vec![]);

    let engine = ReasoningEngine::new(&store);
    let result = engine
        .score_refactor_safety("src/a.rs::function::solo_fn")
        .unwrap();

    assert_eq!(result.node.qualified_name, "src/a.rs::fn::solo_fn");
}

#[test]
fn classify_change_risk_uses_owner_identity_for_cross_package() {
    let mut store = make_store();
    let target = node(
        0,
        "helper",
        "packages/foo/src/lib.rs::fn::helper",
        "packages/foo/src/lib.rs",
        NodeKind::Function,
    );
    let caller = node(
        0,
        "caller",
        "packages/bar/src/lib.rs::fn::caller",
        "packages/bar/src/lib.rs",
        NodeKind::Function,
    );
    let edges = vec![edge(
        "packages/bar/src/lib.rs::fn::caller",
        "packages/foo/src/lib.rs::fn::helper",
        EdgeKind::Calls,
        "packages/bar/src/lib.rs",
    )];
    seed_graph(&mut store, vec![target.clone(), caller], edges);
    attach_owner(&mut store, &target.file_path, "packages/foo/Cargo.toml");
    attach_owner(
        &mut store,
        "packages/bar/src/lib.rs",
        "packages/bar/Cargo.toml",
    );

    let engine = ReasoningEngine::new(&store);
    let result = engine.classify_change_risk(&target.qualified_name).unwrap();

    assert!(
        result
            .contributing_factors
            .iter()
            .any(|factor| factor.contains("cross-package")),
        "expected cross-package factor, got {:?}",
        result.contributing_factors
    );
}

#[test]
fn classify_change_risk_normalizes_function_alias_qname() {
    let mut store = make_store();
    let target = node(
        0,
        "helper",
        "src/lib.rs::fn::helper",
        "src/lib.rs",
        NodeKind::Function,
    );
    let caller = node(
        0,
        "caller",
        "src/other.rs::fn::caller",
        "src/other.rs",
        NodeKind::Function,
    );
    let edges = vec![edge(
        "src/other.rs::fn::caller",
        "src/lib.rs::fn::helper",
        EdgeKind::Calls,
        "src/other.rs",
    )];
    seed_graph(&mut store, vec![target, caller], edges);

    let engine = ReasoningEngine::new(&store);
    let result = engine
        .classify_change_risk("src/lib.rs::function::helper")
        .unwrap();

    assert!(!result.contributing_factors.is_empty());
}

#[test]
fn metrics_compute_fan_in_fan_out_and_dependency_depth() {
    let repo_root = make_repo_root();
    write_repo_file(
        &repo_root,
        "src/lib.rs",
        "pub fn target() {}\npub fn dep() {}\n",
    );
    write_repo_file(&repo_root, "src/a.rs", "pub fn caller_a() {}\n");
    write_repo_file(&repo_root, "src/b.rs", "pub fn caller_b() {}\n");

    let mut store = make_store();
    let nodes = vec![
        node(
            0,
            "target",
            "src/lib.rs::fn::target",
            "src/lib.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "dep",
            "src/lib.rs::fn::dep",
            "src/lib.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "caller_a",
            "src/a.rs::fn::caller_a",
            "src/a.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "caller_b",
            "src/b.rs::fn::caller_b",
            "src/b.rs",
            NodeKind::Function,
        ),
    ];
    let edges = vec![
        edge(
            "src/a.rs::fn::caller_a",
            "src/lib.rs::fn::target",
            EdgeKind::Calls,
            "src/a.rs",
        ),
        edge(
            "src/b.rs::fn::caller_b",
            "src/lib.rs::fn::target",
            EdgeKind::Calls,
            "src/b.rs",
        ),
        edge(
            "src/lib.rs::fn::target",
            "src/lib.rs::fn::dep",
            EdgeKind::Calls,
            "src/lib.rs",
        ),
    ];
    seed_graph(&mut store, nodes, edges);

    let analysis = insights_engine(&store).analyze_metrics(&repo_root).unwrap();
    let target_metric = find_node_metric(&analysis, "src/lib.rs::fn::target");

    assert_eq!(target_metric.fan_in, 2);
    assert_eq!(target_metric.fan_out, 1);
    assert_eq!(target_metric.dependency_depth, 1);
}

#[test]
fn metrics_dependency_depth_has_cycle_guard() {
    let repo_root = make_repo_root();
    write_repo_file(
        &repo_root,
        "src/lib.rs",
        "pub fn a() {}\npub fn b() {}\npub fn c() {}\n",
    );

    let mut store = make_store();
    let nodes = vec![
        node(
            0,
            "a",
            "src/lib.rs::fn::a",
            "src/lib.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "b",
            "src/lib.rs::fn::b",
            "src/lib.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "c",
            "src/lib.rs::fn::c",
            "src/lib.rs",
            NodeKind::Function,
        ),
    ];
    let edges = vec![
        edge(
            "src/lib.rs::fn::a",
            "src/lib.rs::fn::b",
            EdgeKind::Calls,
            "src/lib.rs",
        ),
        edge(
            "src/lib.rs::fn::b",
            "src/lib.rs::fn::c",
            EdgeKind::Calls,
            "src/lib.rs",
        ),
        edge(
            "src/lib.rs::fn::c",
            "src/lib.rs::fn::a",
            EdgeKind::Calls,
            "src/lib.rs",
        ),
    ];
    seed_graph(&mut store, nodes, edges);

    let analysis = insights_engine(&store).analyze_metrics(&repo_root).unwrap();
    let metric = find_node_metric(&analysis, "src/lib.rs::fn::a");

    assert_eq!(metric.dependency_depth, 2);
}

#[test]
fn metrics_compute_loc_and_rust_complexity() {
    let repo_root = make_repo_root();
    write_repo_file(
        &repo_root,
        "src/complex.rs",
        "pub fn complex(value: i32) -> i32 {\n    if value > 0 && value < 10 {\n        return value;\n    }\n    while value > 1 {\n        break;\n    }\n    for step in 0..value {\n        if step % 2 == 0 {\n            break;\n        }\n    }\n    match value {\n        0 => 0,\n        _ => value,\n    }\n}\n",
    );

    let mut complex = node(
        0,
        "complex",
        "src/complex.rs::fn::complex",
        "src/complex.rs",
        NodeKind::Function,
    );
    complex.line_start = 1;
    complex.line_end = 15;

    let mut store = make_store();
    seed_graph(&mut store, vec![complex], vec![]);

    let analysis = insights_engine(&store).analyze_metrics(&repo_root).unwrap();
    let metric = find_node_metric(&analysis, "src/complex.rs::fn::complex");

    assert_eq!(metric.loc, Some(15));
    assert_eq!(metric.cyclomatic_complexity, MetricValue::Available(7));
    assert_eq!(metric.branch_count, MetricValue::Available(7));
}

#[test]
fn metrics_cognitive_complexity_and_nesting_increase_with_nested_control_flow() {
    let repo_root = make_repo_root();
    write_repo_file(
        &repo_root,
        "src/nested.rs",
        "pub fn flat(flag: bool) {\n    if flag {\n        return;\n    }\n}\n\npub fn nested(flag: bool) {\n    if flag {\n        while flag {\n            match 1 {\n                _ => {}\n            }\n            return;\n        }\n    }\n}\n",
    );

    let mut flat = node(
        0,
        "flat",
        "src/nested.rs::fn::flat",
        "src/nested.rs",
        NodeKind::Function,
    );
    flat.line_start = 1;
    flat.line_end = 5;
    let mut nested = node(
        0,
        "nested",
        "src/nested.rs::fn::nested",
        "src/nested.rs",
        NodeKind::Function,
    );
    nested.line_start = 7;
    nested.line_end = 15;

    let mut store = make_store();
    seed_graph(&mut store, vec![flat, nested], vec![]);

    let analysis = insights_engine(&store).analyze_metrics(&repo_root).unwrap();
    let flat_metric = find_node_metric(&analysis, "src/nested.rs::fn::flat");
    let nested_metric = find_node_metric(&analysis, "src/nested.rs::fn::nested");

    assert!(
        nested_metric.cognitive_complexity.copied().unwrap()
            > flat_metric.cognitive_complexity.copied().unwrap()
    );
    assert_eq!(nested_metric.max_nesting_depth, MetricValue::Available(3));
}

#[test]
fn metrics_report_not_available_for_unsupported_language_complexity() {
    let repo_root = make_repo_root();
    write_repo_file(&repo_root, "script.py", "def helper():\n    return 1\n");

    let mut py_node = node(
        0,
        "helper",
        "script.py::fn::helper",
        "script.py",
        NodeKind::Function,
    );
    py_node.language = "python".to_owned();
    py_node.line_start = 1;
    py_node.line_end = 2;

    let mut store = make_store();
    seed_graph(&mut store, vec![py_node], vec![]);

    let analysis = insights_engine(&store).analyze_metrics(&repo_root).unwrap();
    let metric = find_node_metric(&analysis, "script.py::fn::helper");

    assert_eq!(metric.cyclomatic_complexity, MetricValue::NotAvailable);
    assert_eq!(metric.cognitive_complexity, MetricValue::NotAvailable);
    assert_eq!(metric.max_nesting_depth, MetricValue::NotAvailable);
}

#[test]
fn metrics_compute_file_import_count() {
    let repo_root = make_repo_root();
    write_repo_file(&repo_root, "src/lib.rs", "pub fn imported() {}\n");
    write_repo_file(&repo_root, "src/use.rs", "pub fn user() {}\n");

    let nodes = vec![
        node(
            0,
            "imported",
            "src/lib.rs::fn::imported",
            "src/lib.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "user",
            "src/use.rs::fn::user",
            "src/use.rs",
            NodeKind::Function,
        ),
    ];
    let edges = vec![edge(
        "src/use.rs::fn::user",
        "src/lib.rs::fn::imported",
        EdgeKind::Imports,
        "src/use.rs",
    )];

    let mut store = make_store();
    seed_graph(&mut store, nodes, edges);

    let analysis = insights_engine(&store).analyze_metrics(&repo_root).unwrap();
    let metric = find_file_metric(&analysis, "src/use.rs");

    assert_eq!(metric.import_count, 1);
}

#[test]
fn metrics_compute_module_coupling_score() {
    let repo_root = make_repo_root();
    write_repo_file(&repo_root, "packages/foo/src/lib.rs", "pub fn api() {}\n");
    write_repo_file(
        &repo_root,
        "packages/bar/src/lib.rs",
        "pub fn caller() {}\n",
    );

    let foo = node(
        0,
        "api",
        "packages/foo/src/lib.rs::fn::api",
        "packages/foo/src/lib.rs",
        NodeKind::Function,
    );
    let bar = node(
        0,
        "caller",
        "packages/bar/src/lib.rs::fn::caller",
        "packages/bar/src/lib.rs",
        NodeKind::Function,
    );
    let edges = vec![edge(
        "packages/bar/src/lib.rs::fn::caller",
        "packages/foo/src/lib.rs::fn::api",
        EdgeKind::Calls,
        "packages/bar/src/lib.rs",
    )];

    let mut store = make_store();
    seed_graph(&mut store, vec![foo.clone(), bar.clone()], edges);
    attach_owner(&mut store, &foo.file_path, "packages/foo/Cargo.toml");
    attach_owner(&mut store, &bar.file_path, "packages/bar/Cargo.toml");

    let analysis = insights_engine(&store).analyze_metrics(&repo_root).unwrap();
    let foo_metric = find_module_metric(&analysis, "cargo:packages/foo/Cargo.toml");
    let bar_metric = find_module_metric(&analysis, "cargo:packages/bar/Cargo.toml");

    assert_eq!(foo_metric.inbound_dependency_edge_count, 1);
    assert_eq!(foo_metric.coupling_score, 1.0);
    assert_eq!(bar_metric.external_dependency_edge_count, 1);
    assert_eq!(bar_metric.coupling_score, 1.0);
}

#[test]
fn metrics_distribution_detects_outliers() {
    let repo_root = make_repo_root();
    write_repo_file(&repo_root, "src/a.rs", "pub fn a() {}\n");
    write_repo_file(&repo_root, "src/b.rs", "pub fn b() {}\n");
    write_repo_file(&repo_root, "src/c.rs", "pub fn c() {}\n");
    write_repo_file(&repo_root, "src/outlier.rs", "pub fn seed() {}\n");

    let mut nodes = vec![
        node(0, "a", "src/a.rs::fn::a", "src/a.rs", NodeKind::Function),
        node(0, "b", "src/b.rs::fn::b", "src/b.rs", NodeKind::Function),
        node(0, "c", "src/c.rs::fn::c", "src/c.rs", NodeKind::Function),
    ];
    for index in 0..100 {
        nodes.push(node(
            0,
            &format!("outlier_{index}"),
            &format!("src/outlier.rs::fn::outlier_{index}"),
            "src/outlier.rs",
            NodeKind::Function,
        ));
    }

    let mut store = make_store();
    seed_graph(&mut store, nodes, vec![]);

    let analysis = insights_engine(&store).analyze_metrics(&repo_root).unwrap();
    let distribution = find_distribution(&analysis, "file_node_count");

    assert!(
        distribution
            .outliers
            .iter()
            .any(|outlier| outlier.subject_id == "src/outlier.rs")
    );
}

#[test]
fn metrics_distribution_uses_configured_outlier_percentile() {
    let repo_root = make_repo_root();
    let mut nodes = Vec::new();
    for file_index in 1..=5 {
        let file_path = format!("src/file_{file_index}.rs");
        write_repo_file(&repo_root, &file_path, "pub fn seed() {}\n");
        for node_index in 0..file_index {
            nodes.push(node(
                0,
                &format!("fn_{file_index}_{node_index}"),
                &format!("{file_path}::fn::fn_{file_index}_{node_index}"),
                &file_path,
                NodeKind::Function,
            ));
        }
    }

    let mut store = make_store();
    seed_graph(&mut store, nodes, vec![]);

    let config = atlas_engine::config::InsightsConfig {
        outlier_percentile_cutoff: 50,
        ..Default::default()
    };
    let analysis = InsightsEngine::new(&store, config)
        .unwrap()
        .analyze_metrics(&repo_root)
        .unwrap();
    let distribution = find_distribution(&analysis, "file_node_count");

    assert_eq!(distribution.outlier_cutoff, 3.0);
    assert_eq!(distribution.outliers.len(), 3);
    assert!(
        distribution
            .outliers
            .iter()
            .any(|outlier| outlier.subject_id == "src/file_3.rs")
    );
    assert!(
        distribution
            .outliers
            .iter()
            .any(|outlier| outlier.subject_id == "src/file_4.rs")
    );
    assert!(
        distribution
            .outliers
            .iter()
            .any(|outlier| outlier.subject_id == "src/file_5.rs")
    );
}

#[test]
fn large_function_threshold_override_changes_result_set() {
    let repo_root = make_repo_root();
    write_repo_file(
        &repo_root,
        "src/large.rs",
        "pub fn giant() {\n    let mut total = 0;\n    total += 1;\n    total += 2;\n    total += 3;\n    total += 4;\n    total += 5;\n    total += 6;\n    total += 7;\n    total += 8;\n    total += 9;\n    total += 10;\n}\n",
    );

    let mut store = make_store();
    let giant = Node {
        line_end: 12,
        ..node(
            0,
            "giant",
            "src/large.rs::fn::giant",
            "src/large.rs",
            NodeKind::Function,
        )
    };
    seed_graph(&mut store, vec![giant], vec![]);

    let engine = insights_engine(&store);
    let default = engine
        .find_large_functions(
            &repo_root,
            super::LargeFunctionRequest {
                mode: super::LargeFunctionMode::Large,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(default.candidates.is_empty());

    let overridden = engine
        .find_large_functions(
            &repo_root,
            super::LargeFunctionRequest {
                threshold: Some(5),
                mode: super::LargeFunctionMode::Large,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(overridden.candidates.len(), 1);
    assert_eq!(
        overridden.candidates[0].qualified_name,
        "src/large.rs::fn::giant"
    );
}

#[test]
fn large_function_file_scope_and_mode_filtering_work() {
    let repo_root = make_repo_root();
    write_repo_file(
        &repo_root,
        "src/large.rs",
        "pub fn giant() {\n    let mut total = 0;\n    total += 1;\n    total += 2;\n    total += 3;\n    total += 4;\n    total += 5;\n    total += 6;\n    total += 7;\n    total += 8;\n    total += 9;\n    total += 10;\n}\n",
    );
    write_repo_file(
        &repo_root,
        "src/complex.rs",
        "pub fn knot(x: i32) -> i32 {\n    if x > 0 {\n        if x % 2 == 0 {\n            for value in 0..x {\n                if value == 3 || value == 4 {\n                    return value;\n                }\n            }\n        }\n    }\n    0\n}\n",
    );

    let mut store = make_store();
    let giant = Node {
        line_end: 12,
        ..node(
            0,
            "giant",
            "src/large.rs::fn::giant",
            "src/large.rs",
            NodeKind::Function,
        )
    };
    let knot = Node {
        line_end: 11,
        ..node(
            1,
            "knot",
            "src/complex.rs::fn::knot",
            "src/complex.rs",
            NodeKind::Function,
        )
    };
    seed_graph(&mut store, vec![giant, knot], vec![]);

    let engine = insights_engine(&store);
    let complex_only = engine
        .find_large_functions(
            &repo_root,
            super::LargeFunctionRequest {
                files: Some(vec!["src/complex.rs".to_owned()]),
                threshold: Some(20),
                complexity_threshold: Some(3),
                cognitive_threshold: Some(3),
                nesting_threshold: Some(2),
                mode: super::LargeFunctionMode::Complex,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(complex_only.candidates.len(), 1);
    assert_eq!(
        complex_only.candidates[0].qualified_name,
        "src/complex.rs::fn::knot"
    );

    let large_only = engine
        .find_large_functions(
            &repo_root,
            super::LargeFunctionRequest {
                threshold: Some(5),
                complexity_threshold: Some(100),
                cognitive_threshold: Some(100),
                nesting_threshold: Some(100),
                mode: super::LargeFunctionMode::Large,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(large_only.candidates.len(), 2);
    assert!(
        large_only
            .candidates
            .iter()
            .all(|candidate| candidate.large_match && !candidate.complex_match)
    );
}

#[test]
fn large_function_include_tests_and_limit_preserve_sorted_ties() {
    let repo_root = make_repo_root();
    write_repo_file(
        &repo_root,
        "src/a.rs",
        "pub fn alpha() {\n    let a = 1;\n    let b = 2;\n    let c = 3;\n    let d = 4;\n    let e = 5;\n}\n",
    );
    write_repo_file(
        &repo_root,
        "src/z.rs",
        "pub fn zeta() {\n    let a = 1;\n    let b = 2;\n    let c = 3;\n    let d = 4;\n    let e = 5;\n}\n",
    );
    write_repo_file(
        &repo_root,
        "tests/large_test.rs",
        "#[test]\nfn giant_test() {\n    let a = 1;\n    let b = 2;\n    let c = 3;\n    let d = 4;\n    let e = 5;\n}\n",
    );

    let mut store = make_store();
    let alpha = Node {
        line_end: 7,
        ..node(
            0,
            "alpha",
            "src/a.rs::fn::alpha",
            "src/a.rs",
            NodeKind::Function,
        )
    };
    let zeta = Node {
        line_end: 7,
        ..node(
            1,
            "zeta",
            "src/z.rs::fn::zeta",
            "src/z.rs",
            NodeKind::Function,
        )
    };
    let giant_test = Node {
        line_start: 2,
        line_end: 8,
        is_test: true,
        ..node(
            2,
            "giant_test",
            "tests/large_test.rs::test::giant_test",
            "tests/large_test.rs",
            NodeKind::Test,
        )
    };
    seed_graph(&mut store, vec![alpha, zeta, giant_test], vec![]);

    let engine = insights_engine(&store);
    let no_tests = engine
        .find_large_functions(
            &repo_root,
            super::LargeFunctionRequest {
                threshold: Some(5),
                mode: super::LargeFunctionMode::Large,
                limit: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(no_tests.candidates.len(), 1);
    assert_eq!(no_tests.candidates[0].qualified_name, "src/a.rs::fn::alpha");

    let with_tests = engine
        .find_large_functions(
            &repo_root,
            super::LargeFunctionRequest {
                threshold: Some(5),
                mode: super::LargeFunctionMode::Large,
                include_tests: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(
        with_tests
            .candidates
            .iter()
            .any(|candidate| candidate.qualified_name == "tests/large_test.rs::test::giant_test")
    );
}

#[test]
fn pattern_detection_groups_repeated_call_chains() {
    let mut store = make_store();
    let nodes = vec![
        node(
            0,
            "entry_a",
            "src/a.rs::fn::entry_a",
            "src/a.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "parse",
            "src/a.rs::fn::parse",
            "src/a.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "save",
            "src/a.rs::fn::save",
            "src/a.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "entry_b",
            "src/b.rs::fn::entry_b",
            "src/b.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "parse",
            "src/b.rs::fn::parse",
            "src/b.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "save",
            "src/b.rs::fn::save",
            "src/b.rs",
            NodeKind::Function,
        ),
    ];
    let edges = vec![
        edge(
            "src/a.rs::fn::entry_a",
            "src/a.rs::fn::parse",
            EdgeKind::Calls,
            "src/a.rs",
        ),
        edge(
            "src/a.rs::fn::parse",
            "src/a.rs::fn::save",
            EdgeKind::Calls,
            "src/a.rs",
        ),
        edge(
            "src/b.rs::fn::entry_b",
            "src/b.rs::fn::parse",
            EdgeKind::Calls,
            "src/b.rs",
        ),
        edge(
            "src/b.rs::fn::parse",
            "src/b.rs::fn::save",
            EdgeKind::Calls,
            "src/b.rs",
        ),
    ];
    seed_graph(&mut store, nodes, edges);

    let report = insights_engine(&store).analyze_patterns().unwrap();
    let findings = pattern_findings(&report, "pattern_repeated_chain");

    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].details.as_ref().unwrap()["sequence"],
        json!(["parse", "save"])
    );
    assert_eq!(
        findings[0].details.as_ref().unwrap()["occurrence_count"],
        json!(2)
    );
}

#[test]
fn pattern_detection_reports_unused_module_with_blockers() {
    let mut store = make_store();
    let mut public_api = node(
        0,
        "public_api",
        "src/unused/api.rs::fn::public_api",
        "src/unused/api.rs",
        NodeKind::Function,
    );
    public_api.modifiers = Some("pub".to_owned());
    let helper = node(
        0,
        "helper",
        "src/unused/helper.rs::fn::helper",
        "src/unused/helper.rs",
        NodeKind::Function,
    );
    seed_graph(&mut store, vec![public_api, helper], vec![]);

    let report = insights_engine(&store).analyze_patterns().unwrap();
    let findings = pattern_findings(&report, "pattern_unused_module");

    assert_eq!(findings.len(), 1);
    let blockers = findings[0].details.as_ref().unwrap()["blockers"]
        .as_array()
        .expect("blockers array");
    assert!(
        blockers
            .iter()
            .any(|value| value == "contains public API symbols")
    );
}

#[test]
fn pattern_detection_reports_isolated_components() {
    let mut store = make_store();
    let nodes = vec![
        node(
            0,
            "a1",
            "src/a1.rs::fn::a1",
            "src/a1.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "a2",
            "src/a2.rs::fn::a2",
            "src/a2.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "b1",
            "src/b1.rs::fn::b1",
            "src/b1.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "b2",
            "src/b2.rs::fn::b2",
            "src/b2.rs",
            NodeKind::Function,
        ),
    ];
    let edges = vec![
        edge(
            "src/a1.rs::fn::a1",
            "src/a2.rs::fn::a2",
            EdgeKind::Calls,
            "src/a1.rs",
        ),
        edge(
            "src/b1.rs::fn::b1",
            "src/b2.rs::fn::b2",
            EdgeKind::Calls,
            "src/b1.rs",
        ),
    ];
    seed_graph(&mut store, nodes, edges);

    let report = insights_engine(&store).analyze_patterns().unwrap();
    let findings = pattern_findings(&report, "pattern_isolated_component");

    assert_eq!(findings.len(), 2);
    assert!(
        findings
            .iter()
            .all(|finding| { finding.details.as_ref().unwrap()["node_count"] == json!(2) })
    );
}

#[test]
fn pattern_detection_reports_hubs_and_bottlenecks() {
    let mut store = make_store();
    let nodes = vec![
        node(
            0,
            "left_a",
            "src/l1.rs::fn::left_a",
            "src/l1.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "left_b",
            "src/l2.rs::fn::left_b",
            "src/l2.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "hub",
            "src/hub.rs::fn::hub",
            "src/hub.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "right_a",
            "src/r1.rs::fn::right_a",
            "src/r1.rs",
            NodeKind::Function,
        ),
        node(
            0,
            "right_b",
            "src/r2.rs::fn::right_b",
            "src/r2.rs",
            NodeKind::Function,
        ),
    ];
    let edges = vec![
        edge(
            "src/l1.rs::fn::left_a",
            "src/hub.rs::fn::hub",
            EdgeKind::Calls,
            "src/l1.rs",
        ),
        edge(
            "src/l2.rs::fn::left_b",
            "src/hub.rs::fn::hub",
            EdgeKind::Calls,
            "src/l2.rs",
        ),
        edge(
            "src/hub.rs::fn::hub",
            "src/r1.rs::fn::right_a",
            EdgeKind::Calls,
            "src/hub.rs",
        ),
        edge(
            "src/hub.rs::fn::hub",
            "src/r2.rs::fn::right_b",
            EdgeKind::Calls,
            "src/hub.rs",
        ),
    ];
    seed_graph(&mut store, nodes, edges);

    let config = atlas_engine::config::InsightsConfig {
        high_fan_in: 2,
        high_fan_out: 2,
        outlier_percentile_cutoff: 90,
        ..Default::default()
    };
    let report = insights_engine_with_config(&store, config)
        .analyze_patterns()
        .unwrap();
    let findings = pattern_findings(&report, "pattern_centrality");

    assert_eq!(findings.len(), 1);
    assert_eq!(
        findings[0].details.as_ref().unwrap()["qualified_name"],
        json!("src/hub.rs::fn::hub")
    );
    assert_eq!(
        findings[0].details.as_ref().unwrap()["bottleneck"],
        json!(true)
    );
    assert_eq!(findings[0].details.as_ref().unwrap()["hub"], json!(true));
}

#[test]
fn pattern_detection_reports_deep_chains_with_cycle_guard() {
    let mut store = make_store();
    let nodes = vec![
        node(0, "a", "src/a.rs::fn::a", "src/a.rs", NodeKind::Function),
        node(0, "b", "src/b.rs::fn::b", "src/b.rs", NodeKind::Function),
        node(0, "c", "src/c.rs::fn::c", "src/c.rs", NodeKind::Function),
        node(0, "d", "src/d.rs::fn::d", "src/d.rs", NodeKind::Function),
        node(0, "e", "src/e.rs::fn::e", "src/e.rs", NodeKind::Function),
    ];
    let edges = vec![
        edge(
            "src/a.rs::fn::a",
            "src/b.rs::fn::b",
            EdgeKind::Calls,
            "src/a.rs",
        ),
        edge(
            "src/b.rs::fn::b",
            "src/c.rs::fn::c",
            EdgeKind::Calls,
            "src/b.rs",
        ),
        edge(
            "src/c.rs::fn::c",
            "src/a.rs::fn::a",
            EdgeKind::Calls,
            "src/c.rs",
        ),
        edge(
            "src/c.rs::fn::c",
            "src/d.rs::fn::d",
            EdgeKind::Calls,
            "src/c.rs",
        ),
        edge(
            "src/d.rs::fn::d",
            "src/e.rs::fn::e",
            EdgeKind::Calls,
            "src/d.rs",
        ),
    ];
    seed_graph(&mut store, nodes, edges);

    let config = atlas_engine::config::InsightsConfig {
        deep_chain_length: 2,
        ..Default::default()
    };
    let report = insights_engine_with_config(&store, config)
        .analyze_patterns()
        .unwrap();
    let findings = pattern_findings(&report, "pattern_deep_chain");

    assert!(!findings.is_empty());
    let chain = findings[0].details.as_ref().unwrap()["chain"]
        .as_array()
        .expect("chain array")
        .iter()
        .map(|value| value.as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    let unique = chain.iter().collect::<std::collections::BTreeSet<_>>();
    assert_eq!(chain.len(), unique.len());
    assert!(chain.len() >= 4);
}

#[test]
fn risk_assessment_high_fan_in_increases_score() {
    let repo_root = make_repo_root();
    write_repo_file(&repo_root, "src/core.rs", "pub fn target() {}\n");
    write_repo_file(
        &repo_root,
        "src/callers.rs",
        "pub fn c1() {}\npub fn c2() {}\npub fn c3() {}\n",
    );

    let mut store = make_store();
    let low = Node {
        line_end: 1,
        ..node(
            0,
            "target",
            "src/core.rs::fn::target",
            "src/core.rs",
            NodeKind::Function,
        )
    };
    let high = Node {
        line_start: 2,
        line_end: 2,
        ..node(
            1,
            "target_hot",
            "src/core.rs::fn::target_hot",
            "src/core.rs",
            NodeKind::Function,
        )
    };
    let callers = vec![
        node(
            2,
            "c1",
            "src/callers.rs::fn::c1",
            "src/callers.rs",
            NodeKind::Function,
        ),
        node(
            3,
            "c2",
            "src/callers.rs::fn::c2",
            "src/callers.rs",
            NodeKind::Function,
        ),
        node(
            4,
            "c3",
            "src/callers.rs::fn::c3",
            "src/callers.rs",
            NodeKind::Function,
        ),
    ];
    let edges = vec![
        edge(
            "src/callers.rs::fn::c1",
            "src/core.rs::fn::target_hot",
            EdgeKind::Calls,
            "src/callers.rs",
        ),
        edge(
            "src/callers.rs::fn::c2",
            "src/core.rs::fn::target_hot",
            EdgeKind::Calls,
            "src/callers.rs",
        ),
        edge(
            "src/callers.rs::fn::c3",
            "src/core.rs::fn::target_hot",
            EdgeKind::Calls,
            "src/callers.rs",
        ),
    ];
    let mut nodes = vec![low, high];
    nodes.extend(callers);
    seed_graph(&mut store, nodes, edges);

    let config = atlas_engine::config::InsightsConfig {
        high_fan_in: 2,
        ..Default::default()
    };
    let engine = insights_engine_with_config(&store, config);
    let low_risk = assess_risk(&engine, &repo_root, "src/core.rs::fn::target");
    let high_risk = assess_risk(&engine, &repo_root, "src/core.rs::fn::target_hot");

    assert!(high_risk.score > low_risk.score);
    assert!(factor(&high_risk, "fan_in").contribution > 0.0);
}

#[test]
fn risk_assessment_test_adjacency_mitigates_score() {
    let repo_root = make_repo_root();
    write_repo_file(
        &repo_root,
        "src/lib.rs",
        "pub fn subject() {}\npub fn covered() {}\n",
    );
    write_repo_file(
        &repo_root,
        "tests/lib_test.rs",
        "#[test]\nfn covered_test() {}\n",
    );

    let mut store = make_store();
    let subject = node(
        0,
        "subject",
        "src/lib.rs::fn::subject",
        "src/lib.rs",
        NodeKind::Function,
    );
    let covered = Node {
        line_start: 2,
        line_end: 2,
        ..node(
            1,
            "covered",
            "src/lib.rs::fn::covered",
            "src/lib.rs",
            NodeKind::Function,
        )
    };
    let covered_test = Node {
        is_test: true,
        line_start: 2,
        line_end: 2,
        ..node(
            2,
            "covered_test",
            "tests/lib_test.rs::test::covered_test",
            "tests/lib_test.rs",
            NodeKind::Test,
        )
    };
    let mut test_edge = edge(
        "tests/lib_test.rs::test::covered_test",
        "src/lib.rs::fn::covered",
        EdgeKind::Tests,
        "tests/lib_test.rs",
    );
    test_edge.confidence_tier = Some("high".to_owned());
    seed_graph(
        &mut store,
        vec![subject, covered, covered_test],
        vec![test_edge],
    );

    let engine = insights_engine(&store);
    let no_tests = assess_risk(&engine, &repo_root, "src/lib.rs::fn::subject");
    let with_tests = assess_risk(&engine, &repo_root, "src/lib.rs::fn::covered");

    assert!(with_tests.score < no_tests.score);
    assert!(factor(&with_tests, "test_adjacency").mitigates_risk);
}

#[test]
fn risk_assessment_public_api_increases_score() {
    let repo_root = make_repo_root();
    write_repo_file(
        &repo_root,
        "src/lib.rs",
        "fn internal() {}\npub fn exported() {}\n",
    );

    let mut store = make_store();
    let internal = node(
        0,
        "internal",
        "src/lib.rs::fn::internal",
        "src/lib.rs",
        NodeKind::Function,
    );
    let exported = Node {
        line_start: 2,
        line_end: 2,
        modifiers: Some("pub".to_owned()),
        ..node(
            1,
            "exported",
            "src/lib.rs::fn::exported",
            "src/lib.rs",
            NodeKind::Function,
        )
    };
    seed_graph(&mut store, vec![internal, exported], vec![]);

    let engine = insights_engine(&store);
    let internal_risk = assess_risk(&engine, &repo_root, "src/lib.rs::fn::internal");
    let exported_risk = assess_risk(&engine, &repo_root, "src/lib.rs::fn::exported");

    assert!(exported_risk.score > internal_risk.score);
    assert!(factor(&exported_risk, "public_api_exposure").contribution > 0.0);
}

#[test]
fn risk_assessment_unresolved_edges_increase_score() {
    let repo_root = make_repo_root();
    write_repo_file(
        &repo_root,
        "src/lib.rs",
        "pub fn plain() {}\npub fn dynamic() {}\npub fn caller() {}\n",
    );

    let mut store = make_store();
    let plain = Node {
        line_end: 1,
        ..node(
            0,
            "plain",
            "src/lib.rs::fn::plain",
            "src/lib.rs",
            NodeKind::Function,
        )
    };
    let dynamic = Node {
        line_start: 2,
        line_end: 2,
        ..node(
            1,
            "dynamic",
            "src/lib.rs::fn::dynamic",
            "src/lib.rs",
            NodeKind::Function,
        )
    };
    let caller = Node {
        line_start: 3,
        line_end: 3,
        ..node(
            2,
            "caller",
            "src/lib.rs::fn::caller",
            "src/lib.rs",
            NodeKind::Function,
        )
    };
    let mut unresolved = edge(
        "src/lib.rs::fn::caller",
        "src/lib.rs::fn::dynamic",
        EdgeKind::Calls,
        "src/lib.rs",
    );
    unresolved.confidence = 0.2;
    unresolved.confidence_tier = Some("low".to_owned());
    seed_graph(&mut store, vec![plain, dynamic, caller], vec![unresolved]);

    let engine = insights_engine(&store);
    let plain_risk = assess_risk(&engine, &repo_root, "src/lib.rs::fn::plain");
    let dynamic_risk = assess_risk(&engine, &repo_root, "src/lib.rs::fn::dynamic");

    assert!(dynamic_risk.score > plain_risk.score);
    assert!(factor(&dynamic_risk, "unresolved_edge_count").contribution > 0.0);
}

#[test]
fn risk_assessment_large_function_increases_callable_risk() {
    let repo_root = make_repo_root();
    write_repo_file(
        &repo_root,
        "src/lib.rs",
        "pub fn small() {}\n\npub fn large() {\n    let a = 1;\n    let b = 2;\n    let c = 3;\n    let d = 4;\n    let e = 5;\n    let f = 6;\n}\n",
    );

    let mut store = make_store();
    let small = Node {
        line_end: 1,
        ..node(
            0,
            "small",
            "src/lib.rs::fn::small",
            "src/lib.rs",
            NodeKind::Function,
        )
    };
    let large = Node {
        line_start: 3,
        line_end: 9,
        ..node(
            1,
            "large",
            "src/lib.rs::fn::large",
            "src/lib.rs",
            NodeKind::Function,
        )
    };
    seed_graph(&mut store, vec![small, large], vec![]);

    let config = atlas_engine::config::InsightsConfig {
        large_function_loc: 5,
        ..Default::default()
    };
    let engine = insights_engine_with_config(&store, config);
    let small_risk = assess_risk(&engine, &repo_root, "src/lib.rs::fn::small");
    let large_risk = assess_risk(&engine, &repo_root, "src/lib.rs::fn::large");

    assert!(large_risk.score > small_risk.score);
    assert!(factor(&large_risk, "large_function_flag").contribution > 0.0);
}

#[test]
fn risk_assessment_high_cyclomatic_complexity_increases_callable_risk() {
    let repo_root = make_repo_root();
    write_repo_file(
        &repo_root,
        "src/lib.rs",
        "pub fn plain(x: i32) -> i32 { x }\n\npub fn branchy(x: i32) -> i32 {\n    if x > 0 || x < -10 {\n        return 1;\n    }\n    if x % 2 == 0 {\n        return 2;\n    }\n    0\n}\n",
    );

    let mut store = make_store();
    let plain = Node {
        line_end: 1,
        ..node(
            0,
            "plain",
            "src/lib.rs::fn::plain",
            "src/lib.rs",
            NodeKind::Function,
        )
    };
    let branchy = Node {
        line_start: 3,
        line_end: 10,
        ..node(
            1,
            "branchy",
            "src/lib.rs::fn::branchy",
            "src/lib.rs",
            NodeKind::Function,
        )
    };
    seed_graph(&mut store, vec![plain, branchy], vec![]);

    let config = atlas_engine::config::InsightsConfig {
        high_cyclomatic_complexity: 2,
        high_cognitive_complexity: 999,
        max_nesting_depth: 999,
        risk_public_api_weight: 0.0001,
        risk_fan_in_weight: 0.0001,
        risk_fan_out_weight: 0.0001,
        risk_cross_module_dependency_weight: 0.0001,
        risk_test_adjacency_mitigation_weight: 0.0001,
        risk_dependency_depth_weight: 0.0001,
        risk_unresolved_edge_weight: 0.0001,
        risk_large_function_weight: 0.0001,
        risk_loc_weight: 0.0001,
        risk_cyclomatic_complexity_weight: 3.0,
        risk_cognitive_complexity_weight: 0.0001,
        risk_nesting_depth_weight: 0.0001,
        risk_cycle_participation_weight: 0.0001,
        ..Default::default()
    };
    let engine = insights_engine_with_config(&store, config);
    let plain_risk = assess_risk(&engine, &repo_root, "src/lib.rs::fn::plain");
    let branchy_risk = assess_risk(&engine, &repo_root, "src/lib.rs::fn::branchy");

    assert!(branchy_risk.score > plain_risk.score);
    assert!(factor(&branchy_risk, "cyclomatic_complexity").contribution > 0.0);
}

#[test]
fn risk_assessment_high_cognitive_complexity_increases_callable_risk() {
    let repo_root = make_repo_root();
    write_repo_file(
        &repo_root,
        "src/lib.rs",
        "pub fn flat(x: i32) -> i32 { if x > 0 { return 1; } 0 }\n\npub fn nested(x: i32) -> i32 {\n    if x > 0 {\n        if x % 2 == 0 {\n            if x > 10 {\n                return 1;\n            }\n        }\n    }\n    0\n}\n",
    );

    let mut store = make_store();
    let flat = Node {
        line_end: 1,
        ..node(
            0,
            "flat",
            "src/lib.rs::fn::flat",
            "src/lib.rs",
            NodeKind::Function,
        )
    };
    let nested = Node {
        line_start: 3,
        line_end: 12,
        ..node(
            1,
            "nested",
            "src/lib.rs::fn::nested",
            "src/lib.rs",
            NodeKind::Function,
        )
    };
    seed_graph(&mut store, vec![flat, nested], vec![]);

    let config = atlas_engine::config::InsightsConfig {
        high_cyclomatic_complexity: 999,
        high_cognitive_complexity: 3,
        max_nesting_depth: 999,
        risk_public_api_weight: 0.0001,
        risk_fan_in_weight: 0.0001,
        risk_fan_out_weight: 0.0001,
        risk_cross_module_dependency_weight: 0.0001,
        risk_test_adjacency_mitigation_weight: 0.0001,
        risk_dependency_depth_weight: 0.0001,
        risk_unresolved_edge_weight: 0.0001,
        risk_large_function_weight: 0.0001,
        risk_loc_weight: 0.0001,
        risk_cyclomatic_complexity_weight: 0.0001,
        risk_cognitive_complexity_weight: 3.0,
        risk_nesting_depth_weight: 0.0001,
        risk_cycle_participation_weight: 0.0001,
        ..Default::default()
    };
    let engine = insights_engine_with_config(&store, config);
    let flat_risk = assess_risk(&engine, &repo_root, "src/lib.rs::fn::flat");
    let nested_risk = assess_risk(&engine, &repo_root, "src/lib.rs::fn::nested");

    assert!(nested_risk.score > flat_risk.score);
    assert!(factor(&nested_risk, "cognitive_complexity").contribution > 0.0);
}

#[test]
fn risk_assessment_high_nesting_depth_increases_callable_risk() {
    let repo_root = make_repo_root();
    write_repo_file(
        &repo_root,
        "src/lib.rs",
        "pub fn shallow(x: i32) -> i32 { if x > 0 { return 1; } 0 }\n\npub fn deep(x: i32) -> i32 {\n    if x > 0 {\n        if x % 2 == 0 {\n            if x > 10 {\n                if x < 100 {\n                    return 1;\n                }\n            }\n        }\n    }\n    0\n}\n",
    );

    let mut store = make_store();
    let shallow = Node {
        line_end: 1,
        ..node(
            0,
            "shallow",
            "src/lib.rs::fn::shallow",
            "src/lib.rs",
            NodeKind::Function,
        )
    };
    let deep = Node {
        line_start: 3,
        line_end: 14,
        ..node(
            1,
            "deep",
            "src/lib.rs::fn::deep",
            "src/lib.rs",
            NodeKind::Function,
        )
    };
    seed_graph(&mut store, vec![shallow, deep], vec![]);

    let config = atlas_engine::config::InsightsConfig {
        high_cyclomatic_complexity: 999,
        high_cognitive_complexity: 999,
        max_nesting_depth: 2,
        risk_public_api_weight: 0.0001,
        risk_fan_in_weight: 0.0001,
        risk_fan_out_weight: 0.0001,
        risk_cross_module_dependency_weight: 0.0001,
        risk_test_adjacency_mitigation_weight: 0.0001,
        risk_dependency_depth_weight: 0.0001,
        risk_unresolved_edge_weight: 0.0001,
        risk_large_function_weight: 0.0001,
        risk_loc_weight: 0.0001,
        risk_cyclomatic_complexity_weight: 0.0001,
        risk_cognitive_complexity_weight: 0.0001,
        risk_nesting_depth_weight: 3.0,
        risk_cycle_participation_weight: 0.0001,
        ..Default::default()
    };
    let engine = insights_engine_with_config(&store, config);
    let shallow_risk = assess_risk(&engine, &repo_root, "src/lib.rs::fn::shallow");
    let deep_risk = assess_risk(&engine, &repo_root, "src/lib.rs::fn::deep");

    assert!(deep_risk.score > shallow_risk.score);
    assert!(factor(&deep_risk, "max_nesting_depth").contribution > 0.0);
}

#[test]
fn risk_assessment_cycle_participation_increases_score() {
    let repo_root = make_repo_root();
    write_repo_file(&repo_root, "packages/foo/src/lib.rs", "pub fn foo() {}\n");
    write_repo_file(&repo_root, "packages/bar/src/lib.rs", "pub fn bar() {}\n");

    let mut store = make_store();
    let foo = node(
        0,
        "foo",
        "packages/foo/src/lib.rs::fn::foo",
        "packages/foo/src/lib.rs",
        NodeKind::Function,
    );
    let bar = node(
        1,
        "bar",
        "packages/bar/src/lib.rs::fn::bar",
        "packages/bar/src/lib.rs",
        NodeKind::Function,
    );
    let forward = edge(
        "packages/foo/src/lib.rs::fn::foo",
        "packages/bar/src/lib.rs::fn::bar",
        EdgeKind::Calls,
        "packages/foo/src/lib.rs",
    );
    let backward = edge(
        "packages/bar/src/lib.rs::fn::bar",
        "packages/foo/src/lib.rs::fn::foo",
        EdgeKind::Calls,
        "packages/bar/src/lib.rs",
    );
    seed_graph(
        &mut store,
        vec![foo.clone(), bar.clone()],
        vec![forward.clone()],
    );
    attach_owner(&mut store, &foo.file_path, "packages/foo/Cargo.toml");
    attach_owner(&mut store, &bar.file_path, "packages/bar/Cargo.toml");

    let engine = insights_engine(&store);
    let acyclic = assess_risk(&engine, &repo_root, "packages/foo/src/lib.rs::fn::foo");

    let mut cycle_store = make_store();
    seed_graph(&mut cycle_store, vec![foo, bar], vec![forward, backward]);
    attach_owner(
        &mut cycle_store,
        "packages/foo/src/lib.rs",
        "packages/foo/Cargo.toml",
    );
    attach_owner(
        &mut cycle_store,
        "packages/bar/src/lib.rs",
        "packages/bar/Cargo.toml",
    );
    let cycle_engine = insights_engine(&cycle_store);
    let cyclic = assess_risk(
        &cycle_engine,
        &repo_root,
        "packages/foo/src/lib.rs::fn::foo",
    );

    assert!(cyclic.score > acyclic.score);
    assert!(factor(&cyclic, "cycle_participation").contribution > 0.0);
}

#[test]
fn risk_assessment_score_stays_within_zero_to_one_hundred() {
    let repo_root = make_repo_root();
    write_repo_file(
        &repo_root,
        "packages/foo/src/lib.rs",
        "pub fn hotspot(x: i32) -> i32 {\n    if x > 0 {\n        if x % 2 == 0 {\n            return 1;\n        }\n    }\n    0\n}\n",
    );
    write_repo_file(
        &repo_root,
        "packages/bar/src/lib.rs",
        "pub fn caller() {}\n",
    );
    write_repo_file(
        &repo_root,
        "tests/hotspot_test.rs",
        "#[test]\nfn hotspot_test() {}\n",
    );

    let mut store = make_store();
    let hotspot = Node {
        modifiers: Some("pub".to_owned()),
        line_end: 8,
        ..node(
            0,
            "hotspot",
            "packages/foo/src/lib.rs::fn::hotspot",
            "packages/foo/src/lib.rs",
            NodeKind::Function,
        )
    };
    let caller = node(
        1,
        "caller",
        "packages/bar/src/lib.rs::fn::caller",
        "packages/bar/src/lib.rs",
        NodeKind::Function,
    );
    let test_node = Node {
        is_test: true,
        line_start: 2,
        line_end: 2,
        ..node(
            2,
            "hotspot_test",
            "tests/hotspot_test.rs::test::hotspot_test",
            "tests/hotspot_test.rs",
            NodeKind::Test,
        )
    };
    let mut low_confidence = edge(
        "packages/bar/src/lib.rs::fn::caller",
        "packages/foo/src/lib.rs::fn::hotspot",
        EdgeKind::Calls,
        "packages/bar/src/lib.rs",
    );
    low_confidence.confidence = 0.2;
    low_confidence.confidence_tier = Some("low".to_owned());
    let back_edge = edge(
        "packages/foo/src/lib.rs::fn::hotspot",
        "packages/bar/src/lib.rs::fn::caller",
        EdgeKind::Calls,
        "packages/foo/src/lib.rs",
    );
    let test_edge = edge(
        "tests/hotspot_test.rs::test::hotspot_test",
        "packages/foo/src/lib.rs::fn::hotspot",
        EdgeKind::Tests,
        "tests/hotspot_test.rs",
    );
    seed_graph(
        &mut store,
        vec![hotspot, caller, test_node],
        vec![low_confidence, back_edge, test_edge],
    );
    attach_owner(
        &mut store,
        "packages/foo/src/lib.rs",
        "packages/foo/Cargo.toml",
    );
    attach_owner(
        &mut store,
        "packages/bar/src/lib.rs",
        "packages/bar/Cargo.toml",
    );

    let config = atlas_engine::config::InsightsConfig {
        large_function_loc: 4,
        high_cyclomatic_complexity: 2,
        high_cognitive_complexity: 2,
        max_nesting_depth: 1,
        ..Default::default()
    };
    let engine = insights_engine_with_config(&store, config);
    let analysis = assess_risk(&engine, &repo_root, "packages/foo/src/lib.rs::fn::hotspot");

    assert!((0.0..=100.0).contains(&analysis.score));
}

#[test]
fn risk_assessment_low_medium_high_boundaries_are_stable() {
    let repo_root = make_repo_root();
    write_repo_file(
        &repo_root,
        "src/lib.rs",
        "fn low() {}\npub fn medium() {}\npub fn high(x: i32) -> i32 {\n    if x > 0 {\n        if x % 2 == 0 {\n            return 1;\n        }\n    }\n    0\n}\npub fn caller() {}\n",
    );

    let mut store = make_store();
    let low = Node {
        line_end: 1,
        ..node(
            0,
            "low",
            "src/lib.rs::fn::low",
            "src/lib.rs",
            NodeKind::Function,
        )
    };
    let medium = Node {
        line_start: 2,
        line_end: 2,
        modifiers: Some("pub".to_owned()),
        ..node(
            1,
            "medium",
            "src/lib.rs::fn::medium",
            "src/lib.rs",
            NodeKind::Function,
        )
    };
    let high = Node {
        line_start: 3,
        line_end: 10,
        modifiers: Some("pub".to_owned()),
        ..node(
            2,
            "high",
            "src/lib.rs::fn::high",
            "src/lib.rs",
            NodeKind::Function,
        )
    };
    let caller = Node {
        line_start: 11,
        line_end: 11,
        ..node(
            3,
            "caller",
            "src/lib.rs::fn::caller",
            "src/lib.rs",
            NodeKind::Function,
        )
    };
    let mut unresolved = edge(
        "src/lib.rs::fn::caller",
        "src/lib.rs::fn::high",
        EdgeKind::Calls,
        "src/lib.rs",
    );
    unresolved.confidence = 0.2;
    unresolved.confidence_tier = Some("low".to_owned());
    seed_graph(
        &mut store,
        vec![low, medium, high, caller],
        vec![
            unresolved.clone(),
            edge(
                "src/lib.rs::fn::caller",
                "src/lib.rs::fn::high",
                EdgeKind::References,
                "src/lib.rs",
            ),
        ],
    );

    let config = atlas_engine::config::InsightsConfig {
        high_fan_in: 1,
        large_function_loc: 4,
        high_cyclomatic_complexity: 2,
        high_cognitive_complexity: 2,
        max_nesting_depth: 1,
        risk_medium_threshold: 10.0,
        risk_high_threshold: 30.0,
        ..Default::default()
    };
    let engine = insights_engine_with_config(&store, config);
    let low_analysis = assess_risk(&engine, &repo_root, "src/lib.rs::fn::low");
    let medium_analysis = assess_risk(&engine, &repo_root, "src/lib.rs::fn::medium");
    let high_analysis = assess_risk(&engine, &repo_root, "src/lib.rs::fn::high");

    assert_eq!(low_analysis.classification, RiskClassification::Low);
    assert_eq!(medium_analysis.classification, RiskClassification::Medium);
    assert_eq!(high_analysis.classification, RiskClassification::High);
}
