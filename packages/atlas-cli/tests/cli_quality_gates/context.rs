use super::*;

#[test]
fn natural_language_context_queries_map_to_graph_requests() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let usage = read_json_data_output(
        "context",
        run_atlas(
            repo.path(),
            &["--json", "context", "where is greet_twice used?"],
        ),
    );
    assert_eq!(usage["request"]["intent"], json!("usage_lookup"));
    assert!(
        usage["nodes"]
            .as_array()
            .expect("usage nodes array")
            .iter()
            .all(|node| {
                matches!(
                    node["selection_reason"].as_str().unwrap_or_default(),
                    "direct_target" | "caller" | "importee" | "importer"
                )
            }),
        "usage lookup should not include callee noise: {usage:?}"
    );

    let what_calls = read_json_data_output(
        "context",
        run_atlas(
            repo.path(),
            &["--json", "context", "what calls greet_twice?"],
        ),
    );
    assert_eq!(what_calls["request"]["intent"], json!("usage_lookup"));

    let breaks = read_json_data_output(
        "context",
        run_atlas(
            repo.path(),
            &["--json", "context", "what breaks if I change greet_twice?"],
        ),
    );
    assert_eq!(breaks["request"]["intent"], json!("impact_analysis"));
    assert!(
        breaks["workflow"]["headline"].is_string() || breaks["nodes"].is_array(),
        "impact-analysis query should route to graph context: {breaks:?}"
    );
}

#[test]
fn context_symbol_flow_returns_bounded_json() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let data = read_json_data_output(
        "context",
        run_atlas(repo.path(), &["--json", "context", "greet_twice"]),
    );

    let nodes = data["nodes"].as_array().expect("nodes must be an array");
    assert!(
        !nodes.is_empty(),
        "symbol context must return at least one node"
    );
    assert!(
        nodes.iter().any(|n| n["node"]["qualified_name"]
            .as_str()
            .unwrap_or_default()
            .contains("greet_twice")),
        "greet_twice must appear in symbol context nodes"
    );
    assert!(
        data["truncation"].is_object(),
        "truncation metadata must be present"
    );
    assert_eq!(data["request"]["intent"], json!("symbol"));
}

#[test]
fn context_file_flag_returns_file_intent_json() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let data = read_json_data_output(
        "context",
        run_atlas(repo.path(), &["--json", "context", "--file", "src/lib.rs"]),
    );

    assert!(
        data["files"]
            .as_array()
            .expect("files must be an array")
            .iter()
            .any(|f| f["path"] == json!("src/lib.rs")),
        "file context must include src/lib.rs in files"
    );
    assert_eq!(data["request"]["intent"], json!("file"));
}

#[test]
fn context_files_flag_returns_review_intent_json() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let data = read_json_data_output(
        "context",
        run_atlas(repo.path(), &["--json", "context", "--files", "src/lib.rs"]),
    );

    assert_eq!(data["request"]["intent"], json!("review"));
    assert!(
        data["files"]
            .as_array()
            .expect("files must be an array")
            .iter()
            .any(|f| f["path"] == json!("src/lib.rs")),
        "review context must include src/lib.rs"
    );
}

#[test]
fn context_intent_override_changes_request_intent() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let data = read_json_data_output(
        "context",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "context",
                "--files",
                "src/lib.rs",
                "--intent",
                "impact",
            ],
        ),
    );

    assert_eq!(data["request"]["intent"], json!("impact"));
}

#[test]
fn context_not_found_exits_ok_with_empty_nodes() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let data = read_json_data_output(
        "context",
        run_atlas(
            repo.path(),
            &["--json", "context", "totally_nonexistent_xyz_symbol"],
        ),
    );

    assert!(
        data["nodes"]
            .as_array()
            .is_none_or(|array| array.is_empty()),
        "not-found must return empty nodes"
    );
    assert!(data["truncation"].is_object());
}

#[test]
fn context_ambiguous_symbol_returns_ambiguity_metadata() {
    let repo = setup_repo(&[
        ("src/foo.rs", "pub fn process() {}\n"),
        ("src/bar.rs", "pub fn process() {}\n"),
    ]);
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let data = read_json_data_output(
        "context",
        run_atlas(repo.path(), &["--json", "context", "process"]),
    );

    if let Some(ambiguity) = data.get("ambiguity").filter(|value| !value.is_null()) {
        let candidates = ambiguity["candidates"]
            .as_array()
            .expect("ambiguity.candidates must be an array");
        assert!(
            candidates.len() >= 2,
            "ambiguity must list at least two candidates"
        );
    }
    assert!(data["truncation"].is_object());
}

#[test]
fn context_human_readable_symbol_output() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let out = stdout_text(&run_atlas(repo.path(), &["context", "greet_twice"]));
    assert!(
        out.contains("Nodes"),
        "human output must contain 'Nodes': {out}"
    );
    assert!(
        out.contains("Summary"),
        "human output must contain 'Summary' line: {out}"
    );
}

#[test]
fn context_json_contract_stable_for_golden_snapshot() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let mut data = read_json_data_output(
        "context",
        run_atlas(repo.path(), &["--json", "context", "helper"]),
    );

    normalize_context_result(&mut data);

    let golden = read_golden_json("context_helper.json");
    assert_eq!(
        data, golden,
        "context JSON output must match golden snapshot"
    );
}

#[test]
fn context_truncation_metadata_matches_snapshot() {
    let repo = setup_repo(&[
        (
            "src/lib.rs",
            "pub fn alpha() {\n    beta();\n    gamma();\n}\n\npub fn beta() {}\n\npub fn gamma() {}\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let data = read_json_data_output(
        "context",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "context",
                "alpha",
                "--max-nodes",
                "1",
                "--max-edges",
                "0",
                "--max-files",
                "1",
            ],
        ),
    );

    let snapshot = json!({
        "intent": data["request"]["intent"],
        "truncation": data["truncation"],
        "selected_node_qnames": data["nodes"]
            .as_array()
            .expect("context nodes array")
            .iter()
            .filter_map(|node| node["node"]["qualified_name"].as_str())
            .collect::<Vec<_>>(),
        "selected_edge_pairs": data["edges"]
            .as_array()
            .expect("context edges array")
            .iter()
            .map(|edge| format!(
                "{} -> {}",
                edge["edge"]["source_qn"].as_str().unwrap_or_default(),
                edge["edge"]["target_qn"].as_str().unwrap_or_default()
            ))
            .collect::<Vec<_>>(),
        "selected_file_paths": data["files"]
            .as_array()
            .expect("context files array")
            .iter()
            .filter_map(|file| file["path"].as_str())
            .collect::<Vec<_>>(),
    });

    let golden = read_golden_json("context_truncation_alpha.json");
    assert_eq!(snapshot, golden, "truncation snapshot must stay stable");
}
