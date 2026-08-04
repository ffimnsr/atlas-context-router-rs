//! Risk-assessment tests: factor contributions, weighted signals, and
//! classification boundaries.

use super::super::RiskClassification;
use super::{
    assess_risk, attach_owner, edge, factor, insights_engine, insights_engine_with_config,
    make_repo_root, make_store, node, seed_graph, write_repo_file,
};
use atlas_core::{EdgeKind, Node, NodeKind};

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
