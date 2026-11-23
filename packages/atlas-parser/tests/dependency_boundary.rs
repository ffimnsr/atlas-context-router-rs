use std::fs;
use std::path::Path;

const OFFLOADED_GRAMMARS: &[&str] = &[
    "tree-sitter-bash",
    "tree-sitter-c-sharp",
    "tree-sitter-cpp",
    "tree-sitter-java",
    "tree-sitter-php",
    "tree-sitter-ruby",
    "tree-sitter-scala",
];

#[test]
fn offloaded_grammars_are_dev_dependencies_only() {
    let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let manifest: toml::Value = fs::read_to_string(&manifest_path)
        .unwrap_or_else(|err| panic!("failed reading {}: {err}", manifest_path.display()))
        .parse()
        .unwrap_or_else(|err| panic!("failed parsing {}: {err}", manifest_path.display()));
    let dependencies = manifest
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .expect("atlas-parser dependencies table");
    let dev_dependencies = manifest
        .get("dev-dependencies")
        .and_then(toml::Value::as_table)
        .expect("atlas-parser dev-dependencies table");

    for grammar in OFFLOADED_GRAMMARS {
        assert!(
            !dependencies.contains_key(*grammar),
            "{grammar} must not be a normal atlas-parser dependency"
        );
        assert!(
            dev_dependencies.contains_key(*grammar),
            "{grammar} must remain available to parser tests as a dev-dependency"
        );
    }
}
