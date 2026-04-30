use super::*;

#[test]
fn docs_section_resolves_nested_heading_in_json_and_text() {
    let repo = setup_repo(&[
        (
            "README.md",
            "# Overview\nintro\n## Install\nstep one\n## Usage\nrun it\n",
        ),
        ("src/lib.rs", "pub fn helper() {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let json = read_json_data_output(
        "docs_section",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "docs-section",
                "README.md",
                "--heading",
                "document.overview.install",
            ],
        ),
    );
    assert_eq!(json["resolved"], json!(true));
    assert_eq!(json["heading_path"], json!("document.overview.install"));
    assert_eq!(json["start_line"], json!(3));
    assert_eq!(json["end_line"], json!(4));

    let text = stdout_text(&run_atlas(
        repo.path(),
        &[
            "docs-section",
            "README.md",
            "--heading",
            "document.overview.install",
        ],
    ));
    assert_contains_all(&text, &["document.overview.install", "step one"]);
}

#[test]
fn docs_section_returns_candidates_for_duplicate_slugs() {
    let repo = setup_repo(&[
        ("README.md", "# One\n## Install\na\n# Two\n## Install\nb\n"),
        ("src/lib.rs", "pub fn helper() {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let json = read_json_data_output(
        "docs_section",
        run_atlas(
            repo.path(),
            &["--json", "docs-section", "README.md", "--heading", "install"],
        ),
    );
    assert_eq!(json["resolved"], json!(false));
    assert_eq!(json["candidates"][0]["heading_path"], json!("document.one.install"));
    assert_eq!(json["candidates"][1]["heading_path"], json!("document.two.install"));
}

#[test]
fn docs_section_reports_stable_missing_errors() {
    let repo = setup_repo(&[
        ("README.md", "# Overview\nintro\n"),
        ("src/lib.rs", "pub fn helper() {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let missing_file = run_atlas_capture(
        repo.path(),
        &["docs-section", "docs/missing.md", "--heading", "overview"],
    );
    assert!(!missing_file.status.success());
    assert!(String::from_utf8_lossy(&missing_file.stderr).contains("file not found: docs/missing.md"));

    let missing_heading = run_atlas_capture(
        repo.path(),
        &["docs-section", "README.md", "--heading", "missing"],
    );
    assert!(!missing_heading.status.success());
    assert!(String::from_utf8_lossy(&missing_heading.stderr).contains("heading not found in README.md: missing"));
}

#[test]
fn docs_section_truncates_to_max_bytes() {
    let repo = setup_repo(&[
        ("README.md", "# Overview\nalpha beta gamma\n"),
        ("src/lib.rs", "pub fn helper() {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let json = read_json_data_output(
        "docs_section",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "docs-section",
                "README.md",
                "--heading",
                "overview",
                "--max-bytes",
                "12",
            ],
        ),
    );
    assert_eq!(json["truncated"], json!(true));
    assert!(json["omitted_byte_count"].as_u64().is_some_and(|count| count > 0));
}
