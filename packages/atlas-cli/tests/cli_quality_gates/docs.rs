use super::*;
use std::fs;

const FIXED_TIMESTAMP: &str = "2026-01-01T00:00:00Z";

fn build_repo(repo: &TempDir) {
    run_atlas(repo.path(), &["build"]);
}

fn generate_docs(repo: &TempDir, output_dir: &Path, pre: &[&str], extra: &[&str]) -> Output {
    let mut args = Vec::new();
    args.push("docs");
    args.extend_from_slice(pre);
    args.push("generate");
    args.push("--output");
    args.push(output_dir.to_str().expect("utf8 output path"));
    args.push("--timestamp");
    args.push(FIXED_TIMESTAMP);
    args.extend_from_slice(extra);
    run_atlas_capture(repo.path(), &args)
}

#[test]
fn docs_generate_creates_deterministic_markdown_snapshot() {
    let repo = setup_fixture_repo();
    build_repo(&repo);
    let out_root = tempfile::tempdir().expect("output temp dir");
    let first = out_root.path().join("first");
    let second = out_root.path().join("second");

    let output = generate_docs(&repo, &first, &[], &[]);
    assert!(
        output.status.success(),
        "docs generate failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let expected_files = [
        "index.md",
        "files.md",
        "symbols.md",
        "modules.md",
        "components.md",
    ];
    for name in expected_files {
        assert!(
            first.join(name).exists(),
            "missing generated file {name}: {:?}",
            fs::read_dir(&first).map(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .map(|entry| entry.file_name().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
            })
        );
    }

    let index = fs::read_to_string(first.join("index.md")).expect("index.md");
    assert!(index.contains("# Repository Documentation"));
    assert!(index.contains("Generated: `2026-01-01T00:00:00Z`"));
    assert!(index.contains("- [Files](files.md)"));
    assert!(index.contains("- [Symbols](symbols.md)"));
    assert!(index.contains("## Symbol counts by kind"));
    assert!(index.contains("## Modules"));
    assert!(index.contains("## Components"));
    let files = fs::read_to_string(first.join("files.md")).expect("files.md");
    assert!(files.contains("## `src/lib.rs`"));
    assert!(files.contains("## `src/main.rs`"));
    let symbols = fs::read_to_string(first.join("symbols.md")).expect("symbols.md");
    assert!(symbols.contains("Greeter::greet_twice") || symbols.contains("greet_twice"));

    // Deterministic: a second run with the same timestamp and graph produces
    // byte-identical documents.
    let second_output = generate_docs(&repo, &second, &[], &[]);
    assert!(second_output.status.success());
    for name in expected_files {
        let a = fs::read(first.join(name)).expect("first file");
        let b = fs::read(second.join(name)).expect("second file");
        assert_eq!(a, b, "non-deterministic output for {name}");
    }
}

#[test]
fn docs_generate_include_diagrams_embeds_mermaid() {
    let repo = setup_fixture_repo();
    build_repo(&repo);
    let out = tempfile::tempdir().expect("output temp dir");
    let output_dir = out.path().join("docs");

    let output = generate_docs(&repo, &output_dir, &[], &["--include-diagrams"]);
    assert!(
        output.status.success(),
        "docs generate --include-diagrams failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let index = fs::read_to_string(output_dir.join("index.md")).expect("index.md");
    assert!(index.contains("## Repository dependency diagram"));
    assert!(index.contains("```mermaid"));
    assert!(index.contains("flowchart LR"));
}

#[test]
fn docs_generate_fails_actionably_on_missing_graph() {
    let repo = setup_repo(&[("src/lib.rs", "pub fn lonely() {}\n")]);
    let out = tempfile::tempdir().expect("output temp dir");

    let output = generate_docs(&repo, &out.path().join("docs"), &[], &[]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("graph has not been built"),
        "expected actionable missing-graph error, got: {stderr}"
    );
}

#[test]
fn docs_generate_blocks_on_stale_graph_without_allow_stale() {
    let repo = setup_fixture_repo();
    build_repo(&repo);
    let out = tempfile::tempdir().expect("output temp dir");

    // Dirty the working tree so the graph is stale.
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub struct Greeter;\n\npub fn new_helper() -> u32 { 7 }\n",
    )
    .expect("rewrite fixture file");

    let blocked = generate_docs(&repo, &out.path().join("blocked"), &[], &[]);
    assert!(
        !blocked.status.success(),
        "stale graph must block docs generation without --allow-stale"
    );
    let stderr = String::from_utf8_lossy(&blocked.stderr);
    assert!(
        stderr.contains("stale"),
        "expected stale-graph error, got: {stderr}"
    );

    let allowed = generate_docs(&repo, &out.path().join("allowed"), &["--allow-stale"], &[]);
    assert!(
        allowed.status.success(),
        "docs generate --allow-stale should succeed:\n{}",
        String::from_utf8_lossy(&allowed.stderr)
    );
    let warning = String::from_utf8_lossy(&allowed.stderr);
    assert!(
        warning.contains("Warning:") && warning.contains("stale"),
        "expected freshness warning on stderr, got: {warning}"
    );
}

#[test]
fn docs_export_renders_mermaid_and_dot() {
    let repo = setup_fixture_repo();
    build_repo(&repo);

    let mermaid = run_atlas_capture(
        repo.path(),
        &[
            "docs",
            "export",
            "--format",
            "mermaid",
            "--scope",
            "file",
            "--name",
            "src/lib.rs",
        ],
    );
    assert!(
        mermaid.status.success(),
        "mermaid export failed:\n{}",
        String::from_utf8_lossy(&mermaid.stderr)
    );
    let stdout = String::from_utf8_lossy(&mermaid.stdout);
    assert!(stdout.contains("flowchart LR"));
    assert!(stdout.contains("%% Atlas dependency diagram"));
    assert!(stdout.contains("n0["));

    let dot = run_atlas_capture(
        repo.path(),
        &[
            "docs",
            "export",
            "--format",
            "dot",
            "--scope",
            "file",
            "--name",
            "src/main.rs",
        ],
    );
    assert!(
        dot.status.success(),
        "dot export failed:\n{}",
        String::from_utf8_lossy(&dot.stderr)
    );
    let stdout = String::from_utf8_lossy(&dot.stdout);
    assert!(stdout.contains("digraph atlas {"));
    assert!(stdout.contains("n0 [label="));

    // Repo scope falls back to the file graph when no modules are inferred.
    let repo_scope = run_atlas_capture(repo.path(), &["docs", "export"]);
    assert!(
        repo_scope.status.success(),
        "repo scope export failed:\n{}",
        String::from_utf8_lossy(&repo_scope.stderr)
    );
    assert!(String::from_utf8_lossy(&repo_scope.stdout).contains("flowchart LR"));
}
