use super::*;

fn setup_content_repo() -> TempDir {
    let repo = setup_repo(&[
        (
            "src/lib.rs",
            "pub fn alpha() {\n    beta();\n}\n\npub fn beta() {}\n",
        ),
        ("docs/guide.md", "# Guide\n\nAlpha calls beta.\n"),
        (
            "templates/core.html",
            "<html><body>{{ alpha }}</body></html>\n",
        ),
        ("config/settings.toml", "alpha = true\n"),
    ]);
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);
    repo
}

#[test]
fn files_command_matches_glob_pattern() {
    let repo = setup_content_repo();
    let data = read_json_data_output(
        "files",
        run_atlas(repo.path(), &["--json", "content", "files", "*guide*"]),
    );

    let matches = data["matches"].as_array().expect("files matches array");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0]["path"], json!("docs/guide.md"));
}

#[test]
fn grep_command_reports_grouped_hits() {
    let repo = setup_content_repo();
    let data = read_json_data_output(
        "grep",
        run_atlas(repo.path(), &["--json", "content", "grep", "beta", "--max-results", "5"]),
    );

    let groups = data["matches"].as_array().expect("grep matches array");
    let lib_group = groups
        .iter()
        .find(|group| group["file"] == json!("src/lib.rs"))
        .expect("src/lib.rs match group");
    assert!(
        lib_group["hits"]
            .as_array()
            .is_some_and(|hits| !hits.is_empty())
    );
}

#[test]
fn excerpt_command_accepts_repeated_ranges_and_reports_emitted_counts() {
    let repo = setup_content_repo();
    let data = read_json_data_output(
        "excerpt",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "content",
                "excerpt",
                "src/lib.rs",
                "--range",
                "1:2",
                "--range",
                "5:5",
            ],
        ),
    );

    assert_eq!(data["selection_mode"], json!("ranges"));
    assert_eq!(data["summary"]["ranges"]["requested"], json!(2));
    assert_eq!(data["summary"]["ranges"]["emitted"], json!(2));
    assert_eq!(data["summary"]["lines"]["emitted"], json!(3));
}

#[test]
fn match_context_command_finds_literal_match() {
    let repo = setup_content_repo();
    let data = read_json_data_output(
        "match_context",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "content",
                "match-context",
                "src/lib.rs",
                "beta",
                "--before",
                "0",
                "--after",
                "0",
            ],
        ),
    );

    let snippets = data["snippets"].as_array().expect("snippets array");
    assert!(!snippets.is_empty());
    assert_eq!(snippets[0]["match"][0][0], json!(2));
}

#[test]
fn templates_and_text_assets_commands_discover_assets() {
    let repo = setup_content_repo();

    let templates = read_json_data_output(
        "templates",
        run_atlas(repo.path(), &["--json", "content", "templates", "--kind", "html"]),
    );
    assert!(
        templates["matches"]
            .as_array()
            .is_some_and(|matches| matches.iter().any(|item| item["path"] == json!("templates/core.html")))
    );

    let assets = read_json_data_output(
        "text_assets",
        run_atlas(
            repo.path(),
            &["--json", "content", "text-assets", "--kind", "config"],
        ),
    );
    assert!(
        assets["matches"]
            .as_array()
            .is_some_and(|matches| matches.iter().any(|item| item["path"] == json!("config/settings.toml")))
    );
}

#[test]
fn neighbors_command_lists_callers() {
    let repo = setup_content_repo();
    let data = read_json_data_output(
        "neighbors",
        run_atlas(
            repo.path(),
            &["--json", "content", "neighbors", "src/lib.rs::fn::beta"],
        ),
    );

    assert_eq!(data["symbol"]["found"], json!(true));
    assert!(
        data["callers"]
            .as_array()
            .is_some_and(|callers| callers.iter().any(|node| node["qn"] == json!("src/lib.rs::fn::alpha")))
    );
}

#[test]
fn links_and_clusters_commands_accept_repo_files() {
    let repo = setup_content_repo();

    let links = read_json_data_output(
        "links",
        run_atlas(
            repo.path(),
            &["--json", "content", "links", "src/lib.rs", "--include-weak"],
        ),
    );
    assert_eq!(links["summary"]["status"], json!("ok"));

    let clusters = read_json_data_output(
        "clusters",
        run_atlas(
            repo.path(),
            &["--json", "content", "clusters", "src/lib.rs", "--limit", "3"],
        ),
    );
    assert_eq!(clusters["summary"]["status"], json!("ok"));
    assert_eq!(clusters["summary"]["seed_file_count"], json!(1));
}

#[test]
fn docs_section_miss_prints_near_match_candidates() {
    let repo = setup_repo(&[
        (
            "README.md",
            "# Overview\nintro\n## Install\nstep one\n## Usage\nrun it\n",
        ),
        ("src/lib.rs", "pub fn helper() {}\n"),
    ]);
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let output = run_command_capture(
        repo.path(),
        env!("CARGO_BIN_EXE_atlas"),
        &[
            "--json",
            "docs-section",
            "README.md",
            "--heading",
            "document.overview.instal",
        ],
    );
    assert!(!output.status.success(), "missing heading must exit non-zero");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("document.overview.install"),
        "miss must suggest the near-match heading: {stderr}"
    );
}
