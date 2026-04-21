use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use atlas_parser::ParserRegistry;
use serde_json::Value;

const FIXTURE_HASH: &str = "fixture-hash";
const FIXTURE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

#[derive(Debug, Clone)]
struct FixtureCase {
    fixture_path: PathBuf,
    rel_path: String,
}

impl FixtureCase {
    fn golden_path(&self) -> PathBuf {
        let stem = self
            .fixture_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .expect("fixture file stem");
        self.fixture_path
            .with_file_name(format!("{stem}.golden.json"))
    }
}

#[test]
fn parser_fixtures_match_goldens() {
    let fixture_root = Path::new(FIXTURE_DIR);
    let cases = fixture_cases(fixture_root);
    assert!(
        !cases.is_empty(),
        "expected parser fixtures under {}",
        fixture_root.display()
    );

    let registry = ParserRegistry::with_defaults();
    let update_goldens = env::var_os("ATLAS_UPDATE_GOLDENS").is_some();

    for case in cases {
        let source = fs::read(&case.fixture_path)
            .unwrap_or_else(|err| panic!("failed reading {}: {err}", case.fixture_path.display()));
        let (parsed, _) = registry
            .parse(&case.rel_path, FIXTURE_HASH, &source, None)
            .unwrap_or_else(|| panic!("registry did not support {}", case.rel_path));
        let actual = serde_json::to_value(&parsed)
            .unwrap_or_else(|err| panic!("failed serializing {}: {err}", case.rel_path));
        let golden_path = case.golden_path();

        if update_goldens {
            fs::write(
                &golden_path,
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&actual).unwrap_or_else(|err| panic!(
                        "failed pretty-printing {}: {err}",
                        case.rel_path
                    ))
                ),
            )
            .unwrap_or_else(|err| panic!("failed writing {}: {err}", golden_path.display()));
        }

        let expected: Value = serde_json::from_str(
            &fs::read_to_string(&golden_path)
                .unwrap_or_else(|err| panic!("failed reading {}: {err}", golden_path.display())),
        )
        .unwrap_or_else(|err| panic!("failed parsing {}: {err}", golden_path.display()));

        assert_eq!(
            actual,
            expected,
            "golden mismatch for {} (fixture {})",
            case.rel_path,
            case.fixture_path.display()
        );
    }
}

fn fixture_cases(root: &Path) -> Vec<FixtureCase> {
    let mut cases = Vec::new();
    collect_fixture_cases(root, root, &mut cases);
    cases.sort_by(|left, right| left.rel_path.cmp(&right.rel_path));
    cases
}

fn collect_fixture_cases(root: &Path, dir: &Path, cases: &mut Vec<FixtureCase>) {
    let mut entries = fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("failed reading fixture dir {}: {err}", dir.display()))
        .collect::<Result<Vec<_>, _>>()
        .unwrap_or_else(|err| panic!("failed iterating fixture dir {}: {err}", dir.display()));
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_fixture_cases(root, &path, cases);
            continue;
        }

        if is_fixture_source(&path) {
            let rel_path = path
                .strip_prefix(root)
                .unwrap_or_else(|err| panic!("failed relativizing {}: {err}", path.display()))
                .to_string_lossy()
                .into_owned();
            cases.push(FixtureCase {
                fixture_path: path,
                rel_path: format!("fixtures/{rel_path}"),
            });
        }
    }
}

fn is_fixture_source(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if name.ends_with(".golden.json") {
        return false;
    }

    matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some(
            "rs" | "go"
                | "py"
                | "js"
                | "ts"
                | "json"
                | "toml"
                | "html"
                | "css"
                | "sh"
                | "md"
                | "java"
                | "cs"
                | "php"
                | "c"
                | "cpp"
                | "scala"
                | "rb"
        )
    )
}
