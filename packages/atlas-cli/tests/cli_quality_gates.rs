use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};
use tempfile::TempDir;

#[test]
fn sqlite_fts5_smoke_round_trip() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let status = read_json_data_output("status", run_atlas(repo.path(), &["--json", "status"]));
    assert_eq!(status["indexed_file_count"], json!(2));
    assert!(status["node_count"].as_i64().unwrap_or_default() >= 5);
    assert!(status["edge_count"].as_i64().unwrap_or_default() >= 1);

    let query = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "greet_twice"]),
    );
    let results = query["results"]
        .as_array()
        .expect("query results should be an array");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["node"]["name"], json!("greet_twice"));
    assert_eq!(results[0]["node"]["kind"], json!("method"));
    assert_eq!(results[0]["node"]["file_path"], json!("src/lib.rs"));
}

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
fn build_and_update_skip_unsupported_files_without_count_drift() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    write_repo_file(repo.path(), "notes.md", "# atlas notes\n");

    let build = read_json_data_output("build", run_atlas(repo.path(), &["--json", "build"]));
    assert!(build["skipped_unsupported"].as_u64().unwrap_or_default() >= 1);
    assert!(build["parsed"].as_u64().unwrap_or_default() >= 2);

    let update = read_json_data_output(
        "update",
        run_atlas(repo.path(), &["--json", "update", "--files", "notes.md"]),
    );
    assert_eq!(update["skipped_unsupported"], json!(1));
    assert_eq!(update["parsed"], json!(0));
    assert_eq!(update["parse_errors"], json!(0));
}

#[test]
fn mvp_command_contract_holds_for_committed_fixture_repo() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let status = read_json_data_output("status", run_atlas(repo.path(), &["--json", "status"]));
    assert_eq!(status["indexed_file_count"], json!(2));
    assert!(status["node_count"].as_i64().unwrap_or_default() >= 5);
    assert!(status["edge_count"].as_i64().unwrap_or_default() >= 1);

    let query = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "greet_twice"]),
    );
    let results = query["results"]
        .as_array()
        .expect("query results should return a JSON array");
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0]["node"]["qualified_name"],
        json!("src/lib.rs::method::Greeter::greet_twice")
    );

    write_repo_file(
        repo.path(),
        "src/lib.rs",
        r#"pub struct Greeter;

impl Greeter {
    pub fn greet_twice(name: &str) -> String {
        format!("Hello, {name}! Hello again, {name}!")
    }
}

pub fn helper(name: &str) -> String {
    let greeting = Greeter::greet_twice(name);
    format!("{greeting} [updated]")
}
"#,
    );

    let changes = read_json_data_output(
        "detect_changes",
        run_atlas(repo.path(), &["--json", "detect-changes", "--base", "HEAD"]),
    );
    let changes = changes["changes"]
        .as_array()
        .expect("detect-changes changes should return a JSON array");
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["path"], json!("src/lib.rs"));
    assert_eq!(changes[0]["change_type"], json!("modified"));

    let update = read_json_data_output(
        "update",
        run_atlas(repo.path(), &["--json", "update", "--base", "HEAD"]),
    );
    assert!(update["parsed"].as_u64().unwrap_or_default() >= 1);
    assert!(update["nodes_updated"].as_u64().unwrap_or_default() >= 1);

    let impact = read_json_data_output(
        "impact",
        run_atlas(repo.path(), &["--json", "impact", "--base", "HEAD"]),
    );
    // JSON shape is now AdvancedImpactResult: { base: {...}, scored_nodes, risk_level, ... }
    let analysis = &impact["analysis"];
    let base = &analysis["base"];
    assert!(
        base["changed_nodes"]
            .as_array()
            .expect("impact changed_nodes should be an array")
            .iter()
            .any(|node| node["file_path"] == json!("src/lib.rs"))
    );
    assert!(
        base["changed_nodes"]
            .as_array()
            .expect("impact changed_nodes should be an array")
            .iter()
            .any(|node| node["qualified_name"] == json!("src/lib.rs::fn::helper"))
    );
    assert!(
        base["relevant_edges"]
            .as_array()
            .expect("impact relevant_edges should be an array")
            .iter()
            .any(|edge| edge["kind"] == json!("calls"))
    );
    // Advanced fields must be present.
    assert!(
        analysis["risk_level"].is_string(),
        "risk_level must be a string"
    );
    assert!(
        analysis["scored_nodes"].is_array(),
        "scored_nodes must be an array"
    );
    assert!(
        analysis["test_impact"].is_object(),
        "test_impact must be an object"
    );
    assert!(
        analysis["boundary_violations"].is_array(),
        "boundary_violations must be an array"
    );

    let review_context = read_json_data_output(
        "review_context",
        run_atlas(repo.path(), &["--json", "review-context", "--base", "HEAD"]),
    );
    let review_context = &review_context["review_context"];
    assert_eq!(review_context["changed_files"], json!(["src/lib.rs"]));
    assert!(
        review_context["changed_symbols"]
            .as_array()
            .expect("review-context changed_symbols should be an array")
            .iter()
            .any(|node| node["file_path"] == json!("src/lib.rs"))
    );
    assert!(
        review_context["changed_symbol_summaries"].is_array(),
        "review-context changed_symbol_summaries must be an array"
    );
    assert!(
        review_context["impact_overview"].is_object(),
        "review-context impact_overview must be an object"
    );
    assert!(
        review_context["risk_summary"]["cross_package_impact"].is_boolean(),
        "review-context risk_summary.cross_package_impact must be a boolean"
    );

    let status_with_base = read_json_data_output(
        "status",
        run_atlas(repo.path(), &["--json", "status", "--base", "HEAD"]),
    );
    assert_eq!(status_with_base["changed_file_count"], json!(1));
    assert_eq!(status_with_base["diff_target"]["kind"], json!("base_ref"));
    assert_eq!(
        status_with_base["changed_files"][0]["path"],
        json!("src/lib.rs")
    );
}

fn setup_fixture_repo() -> TempDir {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    copy_dir_all(&fixture_repo_root(), temp_dir.path());
    run_command(temp_dir.path(), "git", &["init", "--quiet"]);
    run_command(
        temp_dir.path(),
        "git",
        &["config", "user.name", "Atlas Tests"],
    );
    run_command(
        temp_dir.path(),
        "git",
        &["config", "user.email", "atlas-tests@example.com"],
    );
    run_command(temp_dir.path(), "git", &["add", "."]);
    run_command(
        temp_dir.path(),
        "git",
        &["commit", "--quiet", "-m", "fixture baseline"],
    );
    temp_dir
}

fn fixture_repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sample_repo")
}

fn read_golden_json(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(name);
    serde_json::from_str(&fs::read_to_string(path).expect("golden file")).expect("golden json")
}

fn normalize_query_results(value: &mut Value) {
    let Some(results) = value["results"].as_array_mut() else {
        panic!("query output results should be an array");
    };

    for result in results {
        result["score"] = json!(0.0);
        result["node"]["id"] = json!(0);
        result["node"]["file_hash"] = json!("<hash>");
    }
}

fn read_json_output(output: Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("valid json output")
}

fn read_json_data_output(command: &str, output: Output) -> Value {
    let value = read_json_output(output);
    assert_eq!(value["schema_version"], json!("atlas_cli.v1"));
    assert_eq!(value["command"], json!(command));
    value["data"].clone()
}

fn write_repo_file(repo_root: &Path, relative_path: &str, content: &str) {
    fs::write(repo_root.join(relative_path), content).expect("write repo file");
}

fn run_atlas(repo_root: &Path, args: &[&str]) -> Output {
    run_command(repo_root, env!("CARGO_BIN_EXE_atlas"), args)
}

fn run_command(repo_root: &Path, program: &str, args: &[&str]) -> Output {
    let output = Command::new(program)
        .args(args)
        .current_dir(repo_root)
        .output()
        .unwrap_or_else(|err| panic!("failed to run {program}: {err}"));

    assert!(
        output.status.success(),
        "command failed: {} {}\nstdout:\n{}\nstderr:\n{}",
        program,
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    output
}

fn copy_dir_all(src: &Path, dst: &Path) {
    for entry in fs::read_dir(src).expect("fixture dir") {
        let entry = entry.expect("fixture entry");
        let file_type = entry.file_type().expect("fixture entry type");
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            fs::create_dir_all(&dst_path).expect("create fixture subdir");
            copy_dir_all(&src_path, &dst_path);
        } else {
            fs::copy(&src_path, &dst_path).expect("copy fixture file");
        }
    }
}
