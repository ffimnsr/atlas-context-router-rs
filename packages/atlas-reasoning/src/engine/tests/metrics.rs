//! Metrics and large-function analysis tests.

use super::super::{InsightsEngine, LargeFunctionMode, LargeFunctionRequest, MetricValue};
use super::{
    attach_owner, edge, find_distribution, find_file_metric, find_module_metric, find_node_metric,
    insights_engine, make_repo_root, make_store, node, seed_graph, write_repo_file,
};
use atlas_core::{EdgeKind, Node, NodeKind};

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
            LargeFunctionRequest {
                mode: LargeFunctionMode::Large,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(default.candidates.is_empty());

    let overridden = engine
        .find_large_functions(
            &repo_root,
            LargeFunctionRequest {
                threshold: Some(5),
                mode: LargeFunctionMode::Large,
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
            LargeFunctionRequest {
                files: Some(vec!["src/complex.rs".to_owned()]),
                threshold: Some(20),
                complexity_threshold: Some(3),
                cognitive_threshold: Some(3),
                nesting_threshold: Some(2),
                mode: LargeFunctionMode::Complex,
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
            LargeFunctionRequest {
                threshold: Some(5),
                complexity_threshold: Some(100),
                cognitive_threshold: Some(100),
                nesting_threshold: Some(100),
                mode: LargeFunctionMode::Large,
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
            LargeFunctionRequest {
                threshold: Some(5),
                mode: LargeFunctionMode::Large,
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
            LargeFunctionRequest {
                threshold: Some(5),
                mode: LargeFunctionMode::Large,
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
