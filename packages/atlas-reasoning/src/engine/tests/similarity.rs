//! Similar-function, duplicate-detection, module-inference, and
//! component-label tests.

use super::super::{ComponentLabelRequest, DuplicateDetectionRequest, SimilarFunctionRequest};
use super::{
    attach_owner, insights_engine, insights_engine_with_config, make_repo_root, make_store, node,
    read_fingerprint_cache_json, seed_graph, write_repo_file,
};
use atlas_core::NodeKind;
use serde_json::json;

#[test]
fn similar_functions_rank_semantically_close_callables() {
    let repo_root = make_repo_root();
    write_repo_file(
        &repo_root,
        "src/lib.rs",
        "pub fn alpha(value: i32) -> i32 {\n    let next = value + 1;\n    next * 2\n}\n\npub fn beta(input: i32) -> i32 {\n    let next = input + 1;\n    next * 2\n}\n\npub fn gamma(flag: bool) -> i32 {\n    if flag { 1 } else { 0 }\n}\n",
    );

    let mut alpha = node(
        0,
        "alpha",
        "src/lib.rs::fn::alpha",
        "src/lib.rs",
        NodeKind::Function,
    );
    alpha.params = Some("(value: i32)".to_owned());
    alpha.return_type = Some("i32".to_owned());
    alpha.line_start = 1;
    alpha.line_end = 4;
    let mut beta = node(
        0,
        "beta",
        "src/lib.rs::fn::beta",
        "src/lib.rs",
        NodeKind::Function,
    );
    beta.params = Some("(input: i32)".to_owned());
    beta.return_type = Some("i32".to_owned());
    beta.line_start = 6;
    beta.line_end = 9;
    let mut gamma = node(
        0,
        "gamma",
        "src/lib.rs::fn::gamma",
        "src/lib.rs",
        NodeKind::Function,
    );
    gamma.params = Some("(flag: bool)".to_owned());
    gamma.return_type = Some("i32".to_owned());
    gamma.line_start = 11;
    gamma.line_end = 13;

    let mut store = make_store();
    seed_graph(&mut store, vec![alpha, beta, gamma], vec![]);
    let engine = insights_engine(&store);
    let analysis = engine
        .find_similar_functions(
            &repo_root,
            SimilarFunctionRequest {
                symbol: "src/lib.rs::fn::alpha".to_owned(),
                limit: Some(5),
                min_score: Some(0.3),
                include_same_file: true,
            },
        )
        .expect("similar function analysis");

    assert_eq!(analysis.source.qualified_name, "src/lib.rs::fn::alpha");
    assert_eq!(
        analysis.matches[0].candidate.qualified_name,
        "src/lib.rs::fn::beta"
    );
    assert!(
        analysis.matches[0].score >= 0.4,
        "matches={:?}",
        analysis.matches
    );
}

#[test]
fn duplicate_detection_groups_near_duplicate_callables() {
    let repo_root = make_repo_root();
    write_repo_file(
        &repo_root,
        "src/a.rs",
        "pub fn first(input: i32) -> i32 {\n    let local = input + 1;\n    local * 2\n}\n",
    );
    write_repo_file(
        &repo_root,
        "src/b.rs",
        "pub fn second(value: i32) -> i32 {\n    let result = value + 1;\n    result * 2\n}\n",
    );

    let mut first = node(
        0,
        "first",
        "src/a.rs::fn::first",
        "src/a.rs",
        NodeKind::Function,
    );
    first.params = Some("(input: i32)".to_owned());
    first.return_type = Some("i32".to_owned());
    first.line_start = 1;
    first.line_end = 4;
    let mut second = node(
        0,
        "second",
        "src/b.rs::fn::second",
        "src/b.rs",
        NodeKind::Function,
    );
    second.params = Some("(value: i32)".to_owned());
    second.return_type = Some("i32".to_owned());
    second.line_start = 1;
    second.line_end = 4;

    let mut store = make_store();
    seed_graph(&mut store, vec![first, second], vec![]);
    let engine = insights_engine(&store);
    let analysis = engine
        .find_duplicates(
            &repo_root,
            DuplicateDetectionRequest {
                files: None,
                limit: Some(10),
                min_score: Some(0.6),
                include_tests: false,
                suppressions: Vec::new(),
            },
        )
        .expect("duplicate detection");

    let group = analysis
        .groups
        .iter()
        .find(|group| group.member_count == 2 && group.files == vec!["src/a.rs", "src/b.rs"])
        .unwrap_or_else(|| panic!("groups={:?}", analysis.groups));
    assert!(
        group
            .members
            .iter()
            .any(|member| member.source_span.file_path == "src/a.rs"
                && member.source_span.line_start == 1
                && member.source_span.line_end == 4),
        "group={group:?}"
    );
}

#[test]
fn duplicate_detection_honors_suppressions() {
    let repo_root = make_repo_root();
    write_repo_file(
        &repo_root,
        "src/a.rs",
        "pub fn first(input: i32) -> i32 {\n    let local = input + 1;\n    local * 2\n}\n",
    );
    write_repo_file(
        &repo_root,
        "src/b.rs",
        "pub fn second(value: i32) -> i32 {\n    let result = value + 1;\n    result * 2\n}\n",
    );

    let mut first = node(
        0,
        "first",
        "src/a.rs::fn::first",
        "src/a.rs",
        NodeKind::Function,
    );
    first.line_start = 1;
    first.line_end = 4;
    let mut second = node(
        0,
        "second",
        "src/b.rs::fn::second",
        "src/b.rs",
        NodeKind::Function,
    );
    second.line_start = 1;
    second.line_end = 4;

    let mut store = make_store();
    seed_graph(&mut store, vec![first, second], vec![]);
    let engine = insights_engine(&store);
    let analysis = engine
        .find_duplicates(
            &repo_root,
            DuplicateDetectionRequest {
                files: None,
                limit: Some(10),
                min_score: Some(0.6),
                include_tests: false,
                suppressions: vec!["src/b.rs".to_owned()],
            },
        )
        .expect("duplicate detection");

    assert!(analysis.groups.is_empty(), "groups={:?}", analysis.groups);
}

#[test]
fn configurable_similarity_thresholds_drive_score_band() {
    let repo_root = make_repo_root();
    write_repo_file(
        &repo_root,
        "src/lib.rs",
        "pub fn alpha(value: i32) -> i32 {\n    let next = value + 1;\n    next * 2\n}\n\npub fn beta(input: i32) -> i32 {\n    let next = input + 1;\n    next * 2\n}\n",
    );

    let mut alpha = node(
        0,
        "alpha",
        "src/lib.rs::fn::alpha",
        "src/lib.rs",
        NodeKind::Function,
    );
    alpha.params = Some("(value: i32)".to_owned());
    alpha.return_type = Some("i32".to_owned());
    alpha.line_start = 1;
    alpha.line_end = 4;
    let mut beta = node(
        0,
        "beta",
        "src/lib.rs::fn::beta",
        "src/lib.rs",
        NodeKind::Function,
    );
    beta.params = Some("(input: i32)".to_owned());
    beta.return_type = Some("i32".to_owned());
    beta.line_start = 6;
    beta.line_end = 9;

    let mut store = make_store();
    seed_graph(&mut store, vec![alpha, beta], vec![]);
    let config = atlas_engine::config::InsightsConfig {
        similarity_high_threshold: 0.4,
        similarity_medium_threshold: 0.3,
        similarity_low_threshold: 0.2,
        ..Default::default()
    };
    let engine = insights_engine_with_config(&store, config);
    let analysis = engine
        .find_similar_functions(
            &repo_root,
            SimilarFunctionRequest {
                symbol: "src/lib.rs::fn::alpha".to_owned(),
                limit: Some(5),
                min_score: Some(0.2),
                include_same_file: true,
            },
        )
        .expect("similar function analysis");

    assert_eq!(analysis.thresholds.high, 0.4);
    assert_eq!(analysis.matches[0].score_band, "high");
}

#[test]
fn similar_function_analysis_persists_fingerprint_cache() {
    let repo_root = make_repo_root();
    write_repo_file(
        &repo_root,
        "src/lib.rs",
        "pub fn alpha(value: i32) -> i32 {\n    let next = value + 1;\n    next * 2\n}\n\npub fn beta(input: i32) -> i32 {\n    let next = input + 1;\n    next * 2\n}\n",
    );

    let mut alpha = node(
        0,
        "alpha",
        "src/lib.rs::fn::alpha",
        "src/lib.rs",
        NodeKind::Function,
    );
    alpha.params = Some("(value: i32)".to_owned());
    alpha.return_type = Some("i32".to_owned());
    alpha.line_start = 1;
    alpha.line_end = 4;
    let mut beta = node(
        0,
        "beta",
        "src/lib.rs::fn::beta",
        "src/lib.rs",
        NodeKind::Function,
    );
    beta.params = Some("(input: i32)".to_owned());
    beta.return_type = Some("i32".to_owned());
    beta.line_start = 6;
    beta.line_end = 9;

    let mut store = make_store();
    store
        .replace_file_graph(
            "src/lib.rs",
            "hash-v1",
            Some("rust"),
            None,
            &[alpha, beta],
            &[],
        )
        .unwrap();
    let engine = insights_engine(&store);
    engine
        .find_similar_functions(
            &repo_root,
            SimilarFunctionRequest {
                symbol: "src/lib.rs::fn::alpha".to_owned(),
                limit: Some(5),
                min_score: Some(0.2),
                include_same_file: true,
            },
        )
        .expect("similar function analysis");

    let cache = read_fingerprint_cache_json(&repo_root);
    assert_eq!(cache["version"], json!(1));
    assert_eq!(cache["files"]["src/lib.rs"]["file_hash"], json!("hash-v1"));
    assert!(
        cache["files"]["src/lib.rs"]["callables"]["src/lib.rs::fn::alpha"]["body_shingles"]
            .is_array()
    );
}

#[test]
fn fingerprint_cache_invalidates_only_changed_files() {
    let repo_root = make_repo_root();
    write_repo_file(
        &repo_root,
        "src/a.rs",
        "pub fn first(input: i32) -> i32 {\n    let local = input + 1;\n    local * 2\n}\n",
    );
    write_repo_file(
        &repo_root,
        "src/b.rs",
        "pub fn second(value: i32) -> i32 {\n    let result = value + 1;\n    result * 2\n}\n",
    );

    let mut first = node(
        0,
        "first",
        "src/a.rs::fn::first",
        "src/a.rs",
        NodeKind::Function,
    );
    first.params = Some("(input: i32)".to_owned());
    first.return_type = Some("i32".to_owned());
    first.line_start = 1;
    first.line_end = 4;
    let mut second = node(
        0,
        "second",
        "src/b.rs::fn::second",
        "src/b.rs",
        NodeKind::Function,
    );
    second.params = Some("(value: i32)".to_owned());
    second.return_type = Some("i32".to_owned());
    second.line_start = 1;
    second.line_end = 4;

    let mut store = make_store();
    store
        .replace_file_graph("src/a.rs", "hash-a-v1", Some("rust"), None, &[first], &[])
        .unwrap();
    store
        .replace_file_graph("src/b.rs", "hash-b-v1", Some("rust"), None, &[second], &[])
        .unwrap();
    let engine = insights_engine(&store);
    engine
        .find_duplicates(
            &repo_root,
            DuplicateDetectionRequest {
                files: None,
                limit: Some(10),
                min_score: Some(0.6),
                include_tests: false,
                suppressions: Vec::new(),
            },
        )
        .expect("duplicate detection");

    let first_cache = read_fingerprint_cache_json(&repo_root);
    let cached_a = first_cache["files"]["src/a.rs"].clone();
    let cached_b = first_cache["files"]["src/b.rs"].clone();

    write_repo_file(
        &repo_root,
        "src/b.rs",
        "pub fn second(value: i32) -> i32 {\n    let result = value + 2;\n    result * 3\n}\n",
    );
    let mut second_v2 = node(
        0,
        "second",
        "src/b.rs::fn::second",
        "src/b.rs",
        NodeKind::Function,
    );
    second_v2.params = Some("(value: i32)".to_owned());
    second_v2.return_type = Some("i32".to_owned());
    second_v2.line_start = 1;
    second_v2.line_end = 4;
    store
        .replace_file_graph(
            "src/b.rs",
            "hash-b-v2",
            Some("rust"),
            None,
            &[second_v2],
            &[],
        )
        .unwrap();

    let engine = insights_engine(&store);
    engine
        .find_duplicates(
            &repo_root,
            DuplicateDetectionRequest {
                files: None,
                limit: Some(10),
                min_score: Some(0.6),
                include_tests: false,
                suppressions: Vec::new(),
            },
        )
        .expect("duplicate detection");

    let second_cache = read_fingerprint_cache_json(&repo_root);
    assert_eq!(second_cache["files"]["src/a.rs"], cached_a);
    assert_eq!(
        second_cache["files"]["src/b.rs"]["file_hash"],
        json!("hash-b-v2")
    );
    assert_ne!(second_cache["files"]["src/b.rs"], cached_b);
}

#[test]
fn infer_modules_uses_graph_communities_before_path_fallback() {
    let repo_root = make_repo_root();
    write_repo_file(&repo_root, "src/feature.rs", "pub fn feature() {}\n");
    let feature = node(
        0,
        "feature",
        "src/feature.rs::fn::feature",
        "src/feature.rs",
        NodeKind::Function,
    );

    let mut store = make_store();
    seed_graph(&mut store, vec![feature], vec![]);
    let community_id = store
        .create_community("feature_cluster", Some("test"), Some(0), None)
        .unwrap();
    store
        .add_community_node(community_id, "src/feature.rs::fn::feature")
        .unwrap();
    let engine = insights_engine(&store);
    let analysis = engine.infer_modules(&repo_root).expect("module inference");

    assert!(
        analysis.modules.iter().any(|module| module.module_id
            == format!("community:{community_id}")
            && module.display_name == "feature_cluster"),
        "modules={:?}",
        analysis.modules
    );
}

#[test]
fn infer_modules_distinguishes_explicit_owner_and_path_bucket() {
    let repo_root = make_repo_root();
    write_repo_file(
        &repo_root,
        "packages/atlas-cli/src/lib.rs",
        "pub fn cli() {}\n",
    );
    write_repo_file(&repo_root, "src/core.rs", "pub fn core() {}\n");

    let cli = node(
        0,
        "cli",
        "packages/atlas-cli/src/lib.rs::fn::cli",
        "packages/atlas-cli/src/lib.rs",
        NodeKind::Function,
    );
    let core = node(
        0,
        "core",
        "src/core.rs::fn::core",
        "src/core.rs",
        NodeKind::Function,
    );

    let mut store = make_store();
    seed_graph(&mut store, vec![cli, core], vec![]);
    attach_owner(
        &mut store,
        "packages/atlas-cli/src/lib.rs",
        "packages/atlas-cli/Cargo.toml",
    );
    let engine = insights_engine(&store);
    let analysis = engine.infer_modules(&repo_root).expect("module inference");

    assert!(analysis.modules.iter().any(|module| module.explicit));
    assert!(
        analysis
            .modules
            .iter()
            .any(|module| module.module_id == "infer:src"
                || module.module_id == "infer:src/core.rs"
                || module.display_name == "src"),
        "modules={:?}",
        analysis.modules
    );
}

#[test]
fn label_components_assigns_cli_and_review_labels() {
    let repo_root = make_repo_root();
    write_repo_file(
        &repo_root,
        "packages/atlas-cli/src/commands/changes.rs",
        "pub fn render_review() {}\n",
    );

    let review = node(
        0,
        "render_review",
        "packages/atlas-cli/src/commands/changes.rs::fn::render_review",
        "packages/atlas-cli/src/commands/changes.rs",
        NodeKind::Function,
    );
    let mut store = make_store();
    seed_graph(&mut store, vec![review], vec![]);
    let engine = insights_engine(&store);
    let analysis = engine
        .label_components(
            &repo_root,
            ComponentLabelRequest {
                files: Some(vec![
                    "packages/atlas-cli/src/commands/changes.rs".to_owned(),
                ]),
                symbols: Some(vec![
                    "packages/atlas-cli/src/commands/changes.rs::fn::render_review".to_owned(),
                ]),
                limit: Some(10),
            },
        )
        .expect("component labeling");

    assert!(analysis.assignments.iter().any(|assignment| {
        assignment.file_path == "packages/atlas-cli/src/commands/changes.rs"
            && assignment.labels.iter().any(|label| label.label == "cli")
            && assignment
                .labels
                .iter()
                .any(|label| label.label == "review_context")
    }));
}
