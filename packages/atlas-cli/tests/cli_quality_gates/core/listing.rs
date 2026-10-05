use super::*;

#[test]
fn symbols_paginates_functions_with_totals() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let first = read_json_data_output(
        "symbols",
        run_atlas(
            repo.path(),
            &["--json", "symbols", "--kind", "function", "--limit", "1"],
        ),
    );
    let total = first["total"].as_u64().expect("total count");
    assert!(
        total >= 2,
        "fixture repo should define multiple functions: {first:?}"
    );
    assert_eq!(first["returned"], json!(1));
    assert_eq!(first["limit"], json!(1));
    assert_eq!(first["offset"], json!(0));
    assert_eq!(first["has_more"], json!(true));
    assert_eq!(first["next_offset"], json!(1));
    assert_eq!(first["symbols"].as_array().map(Vec::len), Some(1));

    let rest = read_json_data_output(
        "symbols",
        run_atlas(
            repo.path(),
            &[
                "--json", "symbols", "--kind", "function", "--limit", "100", "--offset", "1",
            ],
        ),
    );
    assert_eq!(rest["offset"], json!(1));
    let returned = rest["returned"].as_u64().expect("returned count");
    assert_eq!(returned + 1, total);
    assert_eq!(rest["has_more"], json!(false));
    assert!(rest["next_offset"].is_null());
}

#[test]
fn symbols_repo_id_filter_and_null_field_omission() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let missing = read_json_data_output(
        "symbols",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "symbols",
                "--kind",
                "function",
                "--repo-id",
                "repo_missing",
            ],
        ),
    );
    assert_eq!(missing["total"], json!(0));
    assert_eq!(missing["filters"]["repo_id"], json!("repo_missing"));

    let page = read_json_data_output(
        "symbols",
        run_atlas(repo.path(), &["--json", "symbols", "--kind", "function"]),
    );
    let symbols = page["symbols"].as_array().expect("symbols array");
    assert!(!symbols.is_empty());
    for symbol in symbols {
        let object = symbol.as_object().expect("symbol object");
        assert!(
            object.values().all(|value| !value.is_null()),
            "CLI symbol entries must omit absent optional keys, got nulls: {symbol:?}"
        );
    }
}

#[test]
fn symbols_accepts_kind_aliases_and_subpath_filters() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let canonical = read_json_data_output(
        "symbols",
        run_atlas(repo.path(), &["--json", "symbols", "--kind", "function"]),
    );
    let aliased = read_json_data_output(
        "symbols",
        run_atlas(repo.path(), &["--json", "symbols", "--kind", "fn"]),
    );
    assert_eq!(aliased["total"], canonical["total"]);
    assert_eq!(aliased["filters"]["kind"], json!("function"));

    let filtered = read_json_data_output(
        "symbols",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "symbols",
                "--kind",
                "function",
                "--subpath",
                "src",
            ],
        ),
    );
    let symbols = filtered["symbols"].as_array().expect("symbols array");
    assert!(!symbols.is_empty());
    for symbol in symbols {
        assert!(
            symbol["file"]
                .as_str()
                .is_some_and(|file| file.starts_with("src/")),
            "subpath filter leaked non-matching file: {symbol:?}"
        );
    }

    let text = stdout_text(&run_atlas(
        repo.path(),
        &["symbols", "--kind", "function", "--limit", "1"],
    ));
    let canonical_total = canonical["total"].as_u64().expect("total");
    assert!(
        text.contains(&format!("total {canonical_total}")),
        "text output should summarize totals: {text}"
    );
    assert!(
        text.contains("next page: atlas symbols --offset 1 --limit 1 --kind function"),
        "next-page hint must preserve active filters: {text}"
    );
}
