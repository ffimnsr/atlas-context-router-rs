use super::*;

#[test]
fn fixture_query_output_matches_golden() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let mut query = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "greet_twice"]),
    );
    normalize_query_results(&mut query);

    let golden = read_golden_json("query_greet_twice.json");
    assert_eq!(query, golden);
}

#[test]
fn query_fuzzy_flag_recovers_close_typo() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let no_fuzzy = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "gret_twice"]),
    );
    let no_fuzzy_results = no_fuzzy["results"].as_array().expect("query results array");
    assert!(
        !no_fuzzy_results.is_empty(),
        "baseline typo query should still surface a candidate: {no_fuzzy:?}"
    );
    assert_eq!(no_fuzzy_results[0]["node"]["name"], json!("greet_twice"));

    let fuzzy = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "gret_twice", "--fuzzy"]),
    );
    let results = fuzzy["results"]
        .as_array()
        .expect("query results array with fuzzy enabled");
    assert!(
        !results.is_empty(),
        "fuzzy typo query should recover close match: {fuzzy:?}"
    );
    assert_eq!(fuzzy["query"]["fuzzy_match"], json!(true));
    assert_eq!(results[0]["node"]["name"], json!("greet_twice"));
    let no_fuzzy_score = no_fuzzy_results[0]["score"].as_f64().unwrap_or_default();
    let fuzzy_score = results[0]["score"].as_f64().unwrap_or_default();
    assert!(
        fuzzy_score > no_fuzzy_score,
        "fuzzy query should improve score for close typo: no_fuzzy={no_fuzzy_score} fuzzy={fuzzy_score}"
    );
}

#[test]
fn query_fuzzy_typo_prefers_code_symbol_over_markdown_noise() {
    let repo = setup_repo(&[
        ("go.mod", "module example.com/atlasfixture\n\ngo 1.22\n"),
        (
            "internal/requestctx/context.go",
            "package requestctx\n\nfunc LoadIdentityMessages() {}\n",
        ),
        (
            "docs/load_identity_messages.md",
            "# Load Identity Messages\n\nContext guide for identity message loading.\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let fuzzy = read_json_data_output(
        "query",
        run_atlas(
            repo.path(),
            &["--json", "query", "LoadIdentityMesages", "--fuzzy"],
        ),
    );
    let results = fuzzy["results"].as_array().expect("query results array");
    assert!(
        !results.is_empty(),
        "fuzzy typo query should return code-symbol result: {fuzzy:?}"
    );
    assert_eq!(results[0]["node"]["name"], json!("LoadIdentityMessages"));
    assert_eq!(results[0]["node"]["kind"], json!("function"));

    let with_files = read_json_data_output(
        "query",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "query",
                "LoadIdentityMesages",
                "--fuzzy",
                "--include-files",
            ],
        ),
    );
    let with_files_results = with_files["results"]
        .as_array()
        .expect("query results array with files");
    assert!(
        !with_files_results.is_empty(),
        "include-files fuzzy query should still return code-symbol result: {with_files:?}"
    );
    assert_eq!(
        with_files_results[0]["node"]["name"],
        json!("LoadIdentityMessages")
    );
    assert_eq!(with_files["query"]["include_files"], json!(true));
    assert!(
        with_files_results.iter().any(|result| {
            result["node"]["file_path"] == json!("docs/load_identity_messages.md")
                || result["node"]["language"] == json!("markdown")
        }),
        "include-files query should keep markdown/file noise visible for ranking comparison: {with_files:?}"
    );
}

#[test]
fn query_exact_symbol_and_qname_rank_definition_in_top_three() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let expected_qn = "src/lib.rs::method::Greeter::greet_twice";

    let exact_name = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "greet_twice"]),
    );
    let exact_name_qns = atlas_query_qnames(&exact_name);
    assert!(
        exact_name_qns.iter().take(3).any(|qn| qn == expected_qn),
        "exact symbol lookup must rank intended definition in top 3: {exact_name:?}"
    );

    let exact_qname = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", expected_qn]),
    );
    let exact_qname_qns = atlas_query_qnames(&exact_qname);
    assert!(
        exact_qname_qns.iter().take(3).any(|qn| qn == expected_qn),
        "qualified-name lookup must rank intended definition in top 3: {exact_qname:?}"
    );
}

#[test]
fn query_ambiguous_short_name_returns_ranked_candidates_with_metadata() {
    let repo = setup_repo(&[
        ("Cargo.toml", "[workspace]\nmembers = ['packages/*']\n"),
        (
            "packages/foo/Cargo.toml",
            "[package]\nname = 'foo'\nversion = '0.1.0'\nedition = '2021'\n",
        ),
        ("packages/foo/src/lib.rs", "pub fn helper() {}\n"),
        (
            "packages/bar/Cargo.toml",
            "[package]\nname = 'bar'\nversion = '0.1.0'\nedition = '2021'\n",
        ),
        ("packages/bar/src/lib.rs", "pub fn helper() {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let query = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "helper"]),
    );
    let results = query["results"].as_array().expect("query results array");
    assert!(results.len() >= 2, "ambiguous lookup must return candidates");
    assert!(
        results.windows(2).all(|pair| {
            pair[0]["score"].as_f64().unwrap_or_default()
                >= pair[1]["score"].as_f64().unwrap_or_default()
        }),
        "ambiguous candidates must be ranked descending: {query:?}"
    );
    assert!(
        results.iter().take(2).all(|result| {
            result["node"]["kind"].is_string() && result["node"]["file_path"].is_string()
        }),
        "ambiguous candidates must include kind and file metadata: {query:?}"
    );

    let qnames = atlas_query_qnames(&query);
    assert!(
        qnames
            .iter()
            .any(|qn| qn == "packages/foo/src/lib.rs::fn::helper")
            && qnames
                .iter()
                .any(|qn| qn == "packages/bar/src/lib.rs::fn::helper"),
        "ambiguous lookup must surface both helper definitions: {query:?}"
    );
}

#[test]
fn query_graph_expand_surfaces_neighbors_plain_grep_cannot_infer() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let target_qn = "src/lib.rs::method::Greeter::greet_twice";
    let atlas = read_json_data_output(
        "query",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "query",
                target_qn,
                "--expand",
                "--expand-hops",
                "2",
            ],
        ),
    );
    let atlas_qns = atlas_query_qnames(&atlas);
    let grep_qns = plain_grep_ranked_candidates(repo.path(), &store, target_qn, 5);

    assert!(
        atlas_qns.iter().any(|qn| qn == "src/lib.rs::fn::helper"),
        "graph expansion must surface direct neighbor helper: {atlas:?}"
    );
    assert!(
        atlas_qns.iter().any(|qn| qn == "src/main.rs::fn::main"),
        "graph expansion must surface transitive caller main: {atlas:?}"
    );
    assert!(
        !grep_qns.iter().any(|qn| qn == "src/main.rs::fn::main"),
        "plain grep baseline must not infer transitive caller main from qname lookup: {grep_qns:?}"
    );
}

#[test]
fn graph_aware_symbol_lookup_beats_plain_grep_baseline_on_fixtures() {
    let repo = setup_repo(&[
        (
            "src/a_calls.rs",
            "pub fn call_helper() { helper(); }\npub fn call_render() { render(); }\n",
        ),
        (
            "src/b_more_calls.rs",
            "pub fn relay_helper() { helper(); }\npub fn relay_render() { render(); }\n",
        ),
        ("src/z_defs.rs", "pub fn helper() {}\npub fn render() {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let cases = [
        LookupEvalCase {
            query: "helper",
            expected_qn: "src/z_defs.rs::fn::helper",
        },
        LookupEvalCase {
            query: "render",
            expected_qn: "src/z_defs.rs::fn::render",
        },
    ];

    let atlas_top1 = cases
        .iter()
        .filter(|case| {
            let query = read_json_data_output(
                "query",
                run_atlas(repo.path(), &["--json", "query", case.query]),
            );
            atlas_query_qnames(&query)
                .first()
                .is_some_and(|qn| qn == case.expected_qn)
        })
        .count();
    let atlas_top3 = cases
        .iter()
        .filter(|case| {
            let query = read_json_data_output(
                "query",
                run_atlas(repo.path(), &["--json", "query", case.query]),
            );
            atlas_query_qnames(&query)
                .iter()
                .take(3)
                .any(|qn| qn == case.expected_qn)
        })
        .count();
    let grep_top1 = cases
        .iter()
        .filter(|case| {
            plain_grep_ranked_candidates(repo.path(), &store, case.query, 1)
                .first()
                .is_some_and(|qn| qn == case.expected_qn)
        })
        .count();
    let grep_top3 = cases
        .iter()
        .filter(|case| {
            plain_grep_ranked_candidates(repo.path(), &store, case.query, 3)
                .iter()
                .any(|qn| qn == case.expected_qn)
        })
        .count();

    assert!(
        atlas_top1 == cases.len() && atlas_top3 == cases.len(),
        "atlas query must rank expected definitions in top-1 and top-3: atlas top1/top3 = {atlas_top1}/{atlas_top3}"
    );
    assert!(
        atlas_top1 >= grep_top1 && atlas_top3 >= grep_top3,
        "atlas query must not underperform plain grep: atlas top1/top3 = {atlas_top1}/{atlas_top3}, grep = {grep_top1}/{grep_top3}"
    );
}
