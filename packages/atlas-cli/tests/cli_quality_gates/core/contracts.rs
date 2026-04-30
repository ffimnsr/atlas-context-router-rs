use super::*;
use jsonschema::JSONSchema;
use std::fs;

fn schema_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../schemas/atlas_cli.v1")
}

fn load_schema(file_name: &str) -> Value {
    let path = schema_root().join(file_name);
    serde_json::from_str(&fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!("read schema {} failed: {err}", path.display())
    }))
    .unwrap_or_else(|err| panic!("parse schema {} failed: {err}", path.display()))
}

fn assert_schema_metadata(schema: &Value, file_name: &str, command: &str) {
    assert_eq!(
        schema["$schema"],
        json!("https://json-schema.org/draft/2020-12/schema"),
        "schema marker mismatch for {file_name}"
    );
    assert_eq!(
        schema["properties"]["schema_version"]["const"],
        json!("atlas_cli.v1"),
        "schema version const mismatch for {file_name}"
    );
    assert_eq!(
        schema["properties"]["command"]["const"],
        json!(command),
        "command const mismatch for {file_name}"
    );
    assert!(
        schema["$id"]
            .as_str()
            .is_some_and(|id| id.ends_with(file_name)),
        "schema id should end with file name for {file_name}: {schema:?}"
    );
}

fn assert_valid_against_schema(schema_file: &str, command: &str, output: Output) {
    let schema = load_schema(schema_file);
    assert_schema_metadata(&schema, schema_file, command);

    let compiled = JSONSchema::options()
        .compile(&schema)
        .unwrap_or_else(|err| panic!("compile {schema_file} failed: {err}"));
    let value = read_json_output(output);
    if let Err(errors) = compiled.validate(&value) {
        let details = errors.map(|err| err.to_string()).collect::<Vec<_>>().join("\n");
        panic!("schema validation failed for {schema_file}:\n{details}\nvalue={value:#}");
    }
}

#[test]
fn atlas_cli_v1_schemas_validate_live_outputs() {
    let repo = setup_fixture_repo();
    let docs_repo = setup_repo(&[
        (
            "README.md",
            "# Overview\nintro\n## Install\nstep\n",
        ),
        ("src/lib.rs", "pub fn helper() {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);
    run_atlas(docs_repo.path(), &["init"]);
    run_atlas(docs_repo.path(), &["build"]);

    assert_valid_against_schema("status.schema.json", "status", run_atlas(repo.path(), &["--json", "status"]));
    assert_valid_against_schema("build.schema.json", "build", run_atlas(repo.path(), &["--json", "build"]));
    assert_valid_against_schema(
        "doctor.schema.json",
        "doctor",
        run_atlas_capture(repo.path(), &["--json", "doctor"]),
    );
    assert_valid_against_schema(
        "query.schema.json",
        "query",
        run_atlas(repo.path(), &["--json", "query", "greet_twice"]),
    );
    assert_valid_against_schema(
        "context.schema.json",
        "context",
        run_atlas(repo.path(), &["--json", "context", "greet_twice"]),
    );
    assert_valid_against_schema(
        "postprocess.schema.json",
        "postprocess",
        run_atlas(repo.path(), &["--json", "postprocess"]),
    );
    assert_valid_against_schema(
        "docs-section.schema.json",
        "docs_section",
        run_atlas(
            docs_repo.path(),
            &[
                "--json",
                "docs-section",
                "README.md",
                "--heading",
                "document.overview.install",
            ],
        ),
    );

    rewrite_fixture_helper(repo.path());

    assert_valid_against_schema(
        "update.schema.json",
        "update",
        run_atlas(repo.path(), &["--json", "update", "--base", "HEAD"]),
    );

    assert_valid_against_schema(
        "impact.schema.json",
        "impact",
        run_atlas(
            repo.path(),
            &["--json", "impact", "--base", "HEAD", "--max-nodes", "4"],
        ),
    );
    assert_valid_against_schema(
        "review-context.schema.json",
        "review_context",
        run_atlas(
            repo.path(),
            &["--json", "review-context", "--base", "HEAD", "--max-nodes", "4"],
        ),
    );
    assert_valid_against_schema(
        "explain-change.schema.json",
        "explain_change",
        run_atlas(
            repo.path(),
            &["--json", "explain-change", "--base", "HEAD", "--max-nodes", "4"],
        ),
    );
}
