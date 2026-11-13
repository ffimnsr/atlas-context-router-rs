//! ICM-H3 — memory-layer JSON snapshots.
//!
//! Every snapshot is produced from the shared [`memory_fixtures`] corpus so the
//! record shapes, freshness states, visibility rules, and ranking evidence stay
//! visible in review. Volatile identity and timestamps are normalized before
//! comparison; run with `UPDATE_GOLDEN=1` to regenerate the committed files.

use super::memory_fixtures::{MemoryLayerFixture, seed_memory_layer};
use super::*;

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(name)
}

/// Compare against the committed golden file, or rewrite it when
/// `UPDATE_GOLDEN=1` is set for an intentional shape change.
fn assert_snapshot(name: &str, value: &Value) {
    let path = golden_path(name);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        fs::write(
            &path,
            format!(
                "{}\n",
                serde_json::to_string_pretty(value).expect("serialize snapshot")
            ),
        )
        .expect("write golden snapshot");
        return;
    }
    let golden: Value = serde_json::from_str(
        &fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {name}: {error}")),
    )
    .unwrap_or_else(|error| panic!("parse {name}: {error}"));
    assert_eq!(value, &golden, "snapshot mismatch for {name}");
}

/// Replace fixture-specific ids and volatile timestamps with stable markers.
fn normalize_fixture_value(value: &mut Value, fixture: &MemoryLayerFixture) {
    scrub_repo_paths(value, Path::new(&fixture.repo_root));
    let replacements = [(fixture.artifact_source_id.as_str(), "<artifact_source_id>")];
    normalize_volatile(value, &replacements);
}

fn normalize_volatile(value: &mut Value, replacements: &[(&str, &str)]) {
    match value {
        Value::String(text) => {
            for (needle, replacement) in replacements {
                if !needle.is_empty() && text == needle {
                    *text = (*replacement).to_owned();
                    return;
                }
            }
            if text.starts_with("sess-") {
                *text = "<session_id>".to_owned();
                return;
            }
            // Memory, feedback, and overview ids are derived hex digests.
            if text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                *text = "<id>".to_owned();
                return;
            }
            // Findings embed the raw id inside prose, so scrub id tokens too.
            if text.len() > 64 && text.split_whitespace().any(is_hex_id) {
                *text = text
                    .split_whitespace()
                    .map(|token| if is_hex_id(token) { "<id>" } else { token })
                    .collect::<Vec<_>>()
                    .join(" ");
            }
        }
        Value::Array(items) => {
            for item in items {
                normalize_volatile(item, replacements);
            }
        }
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                if is_volatile_timestamp_key(key) {
                    *item = json!("<timestamp>");
                    continue;
                }
                if is_volatile_number_key(key) {
                    *item = json!(0.0);
                    continue;
                }
                normalize_volatile(item, replacements);
            }
        }
        _ => {}
    }
}

fn scrub_provenance(value: &mut Value) {
    if let Some(provenance) = value.get_mut("atlas_provenance") {
        provenance["repo_id"] = json!("<repo_id>");
        provenance["registry_root_repo_id"] = json!("<repo_id>");
        provenance["commit_sha"] = json!("<commit_sha>");
    }
}

fn is_volatile_timestamp_key(key: &str) -> bool {
    matches!(
        key,
        "created_at"
            | "updated_at"
            | "last_accessed_at"
            | "generated_at"
            | "graph_last_indexed_at"
            | "last_validated_at"
    )
}

fn is_hex_id(token: &str) -> bool {
    token.len() == 64 && token.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// Wall-clock-derived scores that must not make snapshots time sensitive.
fn is_volatile_number_key(key: &str) -> bool {
    matches!(key, "score" | "recency_rank")
}

fn at(repo: &Path, args: &[&str]) -> Output {
    let mut full = vec!["--json"];
    full.extend_from_slice(args);
    run_atlas(repo, &full)
}

fn data(command: &str, output: Output) -> Value {
    read_json_data_output(command, output)
}

#[test]
fn memory_store_and_recall_json_snapshots() {
    let repo = setup_fixture_repo();
    let fixture = seed_memory_layer(repo.path());

    let mut store = data(
        "memory.store",
        at(
            repo.path(),
            &[
                "memory",
                "store",
                "Connection pool sizing must stay below max_connections.",
                "--topic",
                "postgres",
                "--title",
                "pool sizing",
                "--importance",
                "high",
                "--source-id",
                "artifact-1",
            ],
        ),
    );
    normalize_fixture_value(&mut store, &fixture);
    assert_snapshot("memory_store.json", &store);

    let mut recall = data(
        "memory.recall",
        at(
            repo.path(),
            &["memory", "recall", "postgres pool sizing", "--limit", "5"],
        ),
    );
    normalize_fixture_value(&mut recall, &fixture);
    assert_snapshot("memory_recall.json", &recall);
}

#[test]
fn memory_health_json_snapshot() {
    let repo = setup_fixture_repo();
    let fixture = seed_memory_layer(repo.path());

    let mut health = data("memory.health", at(repo.path(), &["memory", "health"]));
    normalize_fixture_value(&mut health, &fixture);
    assert_snapshot("memory_health.json", &health);
}

#[test]
fn feedback_record_and_search_json_snapshots() {
    let repo = setup_fixture_repo();
    let fixture = seed_memory_layer(repo.path());

    let mut record = data(
        "feedback.record",
        at(
            repo.path(),
            &[
                "feedback",
                "record",
                "--predicted",
                "session_store is unused",
                "--actual",
                "session_store is used by memory recall",
                "--analysis-kind",
                "dead_code",
                "--symbol",
                "src/lib.rs::fn::helper",
                "--file",
                "src/lib.rs",
            ],
        ),
    );
    normalize_fixture_value(&mut record, &fixture);
    assert_snapshot("feedback_record.json", &record);

    let mut search = data(
        "feedback.search",
        at(repo.path(), &["feedback", "search", "dead code"]),
    );
    normalize_fixture_value(&mut search, &fixture);
    assert_snapshot("feedback_search.json", &search);
}

#[test]
fn memoir_inspect_json_snapshot() {
    let repo = setup_fixture_repo();
    let fixture = seed_memory_layer(repo.path());

    let mut inspect = data(
        "memoir.inspect",
        at(
            repo.path(),
            &[
                "memoir",
                "inspect",
                "wake_up_pack",
                "--graph",
                "memory-arch",
            ],
        ),
    );
    normalize_fixture_value(&mut inspect, &fixture);
    assert_snapshot("memoir_inspect.json", &inspect);
}

#[test]
fn overview_show_and_search_json_snapshots() {
    let repo = setup_fixture_repo();
    let fixture = seed_memory_layer(repo.path());

    let subject = fixture.overview_subject_id.clone();
    let mut show = data(
        "overview.show",
        at(
            repo.path(),
            &["overview", "show", &subject, "--level", "l1"],
        ),
    );
    normalize_fixture_value(&mut show, &fixture);
    scrub_provenance(&mut show);
    assert_snapshot("memory_overview_show.json", &show);

    let mut search = data(
        "overview.search",
        at(
            repo.path(),
            &["overview", "search", "greeting", "--level", "l0"],
        ),
    );
    normalize_fixture_value(&mut search, &fixture);
    scrub_provenance(&mut search);
    assert_snapshot("memory_overview_search.json", &search);
}

#[test]
fn wake_up_json_snapshot() {
    let repo = setup_fixture_repo();
    let fixture = seed_memory_layer(repo.path());

    let mut pack = data(
        "wake-up",
        at(repo.path(), &["wake-up", "--topic", "decisions"]),
    );
    normalize_fixture_value(&mut pack, &fixture);
    assert_snapshot("wake_up.json", &pack);
}

/// The shared corpus also proves the visibility and artifact-reference rules:
/// another frontend's private memory never surfaces, and wake-up references a
/// saved artifact by `source_id` only.
#[test]
fn fixture_keeps_frontend_private_memory_out_of_recall_and_wake_up() {
    let repo = setup_fixture_repo();
    let fixture = seed_memory_layer(repo.path());

    let recall = data(
        "memory.recall",
        at(
            repo.path(),
            &["memory", "recall", "preference terse output"],
        ),
    );
    assert_eq!(recall["count"], json!(0));
    assert!(recall["results"].as_array().unwrap().is_empty());

    let pack = data("wake-up", at(repo.path(), &["wake-up"]));
    let entries = pack["critical_memories"].as_array().expect("entries");
    assert!(
        entries
            .iter()
            .all(|entry| entry["memory_id"] != json!(fixture.private_memory_id)),
        "frontend-private memory must not leak into the wake-up pack"
    );
    let artifact = entries
        .iter()
        .find(|entry| entry["kind"] == json!("artifact"))
        .expect("artifact reference");
    assert_eq!(artifact["source_id"], json!(fixture.artifact_source_id));
    assert!(
        artifact.get("body").is_none(),
        "wake-up must reference artifacts by source_id only"
    );
}
