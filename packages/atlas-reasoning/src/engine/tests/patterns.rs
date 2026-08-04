//! Pattern-detection tests: repeated chains, unused modules, isolated
//! components, hubs/bottlenecks, and deep chains with cycle guard.

use super::{
    edge, insights_engine, insights_engine_with_config, make_store, node, pattern_findings,
    seed_graph,
};
use atlas_core::{EdgeKind, NodeKind};
use serde_json::json;

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

    let report = insights_engine(&store).analyze_patterns("/repo").unwrap();
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

    let report = insights_engine(&store).analyze_patterns("/repo").unwrap();
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

    let report = insights_engine(&store).analyze_patterns("/repo").unwrap();
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
        .analyze_patterns("/repo")
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
        .analyze_patterns("/repo")
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
