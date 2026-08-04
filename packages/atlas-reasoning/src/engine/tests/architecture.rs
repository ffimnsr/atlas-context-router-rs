//! Architecture-analysis tests: SCC cycles, layer rules, high coupling,
//! and ignored-module exclusion.

use super::{
    attach_owner, edge, find_architecture_finding, insights_engine, insights_engine_with_config,
    make_repo_root, make_store, node, seed_graph, write_repo_file,
};
use atlas_core::{EdgeKind, InsightSeverity, NodeKind};
use serde_json::json;

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
