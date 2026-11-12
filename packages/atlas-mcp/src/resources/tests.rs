use super::{
    docs_completion_items, resource_descriptors, resource_template_descriptors, resources_list,
    resources_read, resources_templates_list, uris,
};
use crate::output::OutputFormat;
use crate::session_tools::tool_save_context_artifact;
use serde_json::json;
use std::fs;
use tempfile::TempDir;

#[test]
fn resources_list_is_deterministic_and_paginates() {
    let first = resources_list(Some(&json!({"limit": 1}))).expect("list resources");
    assert_eq!(first["resources"].as_array().expect("resources").len(), 1);
    assert_eq!(first["resultType"], json!("complete"));
    assert_eq!(first["ttlMs"], json!(300000));
    assert_eq!(first["cacheScope"], json!("public"));
    assert_eq!(first["nextCursor"], json!("offset:1"));

    let second =
        resources_list(Some(&json!({"limit": 1, "cursor": "offset:1"}))).expect("next page");
    assert_eq!(second["resources"].as_array().expect("resources").len(), 1);
    assert_eq!(second["nextCursor"], json!("offset:2"));

    let third =
        resources_list(Some(&json!({"limit": 1, "cursor": "offset:2"}))).expect("third page");
    assert_eq!(third["resources"].as_array().expect("resources").len(), 1);
    assert!(third.get("nextCursor").is_none());
}

#[test]
fn resource_templates_list_is_deterministic_and_paginates() {
    let first = resources_templates_list(Some(&json!({"limit": 1}))).expect("list templates");
    assert_eq!(
        first["resourceTemplates"]
            .as_array()
            .expect("templates")
            .len(),
        1
    );
    assert_eq!(first["resultType"], json!("complete"));
    assert_eq!(first["ttlMs"], json!(300000));
    assert_eq!(first["cacheScope"], json!("public"));
    assert_eq!(first["nextCursor"], json!("offset:1"));

    let second = resources_templates_list(Some(&json!({"limit": 1, "cursor": "offset:1"})))
        .expect("next template page");
    assert_eq!(
        second["resourceTemplates"]
            .as_array()
            .expect("templates")
            .len(),
        1
    );
    assert_eq!(second["nextCursor"], json!("offset:2"));
}

#[test]
fn resource_descriptors_include_metadata_fields() {
    for resource in resource_descriptors() {
        assert!(!resource.uri.is_empty());
        assert!(!resource.mime_type.as_deref().unwrap_or_default().is_empty());
        assert!(resource.icons.is_none());
        assert!(
            resource
                .meta
                .as_ref()
                .and_then(|meta| meta.get("atlas:descriptorKind"))
                .is_some()
        );
    }
    for template in resource_template_descriptors() {
        assert!(!template.uri_template.is_empty());
        assert!(!template.mime_type.as_deref().unwrap_or_default().is_empty());
        assert!(template.icons.is_none());
    }
}

#[test]
fn docs_completion_uses_context_file_for_heading_suggestions() {
    let dir = TempDir::new().expect("tempdir");
    let docs_dir = dir.path().join("wiki");
    fs::create_dir_all(&docs_dir).expect("docs dir");
    fs::write(
        docs_dir.join("guide.md"),
        "# Guide\n## Install\nbody\n## Usage\nmore\n",
    )
    .expect("write guide");

    let headings = docs_completion_items(
        dir.path().to_str().expect("repo root"),
        "heading",
        "document.guide.i",
        Some(&json!({"arguments": {"file": "wiki/guide.md"}})),
    )
    .expect("heading completion");
    assert_eq!(headings, vec!["document.guide.install"]);
}

#[test]
fn resources_read_docs_index_returns_markdown() {
    let dir = TempDir::new().expect("tempdir");
    let repo_root = dir.path().to_str().expect("repo root");
    let db_path = dir
        .path()
        .join("worldtree.db")
        .to_string_lossy()
        .into_owned();

    let result = resources_read(
        Some(&json!({"uri": "atlas://docs/index"})),
        repo_root,
        &db_path,
    )
    .expect("resource read");
    assert_eq!(result["resultType"], json!("complete"));
    assert_eq!(result["ttlMs"], json!(60000));
    assert_eq!(result["cacheScope"], json!("private"));
    assert_eq!(result["contents"][0]["mimeType"], json!("text/markdown"));
    let text = result["contents"][0]["text"]
        .as_str()
        .expect("markdown text");
    assert!(text.contains("Atlas Docs Index"));
    assert!(text.contains("atlas://tool-docs/query_graph"));
}

#[test]
fn resources_read_health_status_returns_content() {
    let dir = TempDir::new().expect("tempdir");
    let repo_root = dir.path().to_str().expect("repo root");
    let db_path = dir
        .path()
        .join("worldtree.db")
        .to_string_lossy()
        .into_owned();

    let result = resources_read(
        Some(&json!({"uri": "atlas://health/status"})),
        repo_root,
        &db_path,
    )
    .expect("resource read");
    assert_eq!(result["resultType"], json!("complete"));
    assert_eq!(result["ttlMs"], json!(60000));
    assert_eq!(result["cacheScope"], json!("private"));
    assert_eq!(result["contents"][0]["mimeType"], json!("application/json"));
    assert!(result["contents"][0]["text"].as_str().is_some());
}

#[test]
fn resources_read_saved_context_round_trips_saved_artifact() {
    let dir = TempDir::new().expect("tempdir");
    let repo_root = dir.path().to_str().expect("repo root");
    let db_path = dir
        .path()
        .join("worldtree.db")
        .to_string_lossy()
        .into_owned();

    let payload = std::iter::repeat_n("saved context payload with safe spacing", 30)
        .collect::<Vec<_>>()
        .join(" ");
    let saved = tool_save_context_artifact(
        Some(&json!({
            "label": "artifact",
            "content": payload
        })),
        repo_root,
        &db_path,
        OutputFormat::Json,
    )
    .expect("save artifact");
    let source_id = saved["structuredContent"]["source_id"]
        .as_str()
        .expect("source id");

    let result = resources_read(
        Some(&json!({"uri": format!("atlas://saved-context/{source_id}")})),
        repo_root,
        &db_path,
    )
    .expect("resource read");
    assert_eq!(result["contents"][0]["text"], json!(payload));
}

#[test]
fn resources_read_tool_docs_returns_examples_and_usage() {
    let dir = TempDir::new().expect("tempdir");
    let repo_root = dir.path().to_str().expect("repo root");
    let db_path = dir
        .path()
        .join("worldtree.db")
        .to_string_lossy()
        .into_owned();

    let result = resources_read(
        Some(&json!({"uri": "atlas://tool-docs/query_graph"})),
        repo_root,
        &db_path,
    )
    .expect("resource read");
    assert_eq!(result["contents"][0]["mimeType"], json!("text/markdown"));
    let text = result["contents"][0]["text"]
        .as_str()
        .expect("markdown text");
    assert!(text.contains("# Tool Docs: `query_graph`"));
    assert!(text.contains("## Input Contract"));
    assert!(text.contains("## Usage"));
    assert!(text.contains("### Tool Usage Guidance"));
    assert!(text.contains("### Target Tool Call Examples"));
    assert!(text.contains("### Few-Shot Prompts"));
}

#[test]
fn resources_read_prompt_docs_returns_prompt_body_and_args() {
    let dir = TempDir::new().expect("tempdir");
    let repo_root = dir.path().to_str().expect("repo root");
    let db_path = dir
        .path()
        .join("worldtree.db")
        .to_string_lossy()
        .into_owned();

    let result = resources_read(
        Some(&json!({"uri": "atlas://prompt-docs/review_change"})),
        repo_root,
        &db_path,
    )
    .expect("resource read");
    assert_eq!(result["contents"][0]["mimeType"], json!("text/markdown"));
    let text = result["contents"][0]["text"]
        .as_str()
        .expect("markdown text");
    assert!(text.contains("# Prompt Docs: `review_change`"));
    assert!(text.contains("## Arguments"));
    assert!(text.contains("## Default Prompt Body"));
    assert!(text.contains("detect_changes"));
}

#[test]
fn resources_read_unknown_uri_returns_error() {
    let error = resources_read(
        Some(&json!({"uri": "atlas://unknown"})),
        "/repo",
        "/repo/.atlas/worldtree.db",
    )
    .expect_err("unknown resource must fail");
    assert!(
        error.to_string().contains("unknown atlas uri")
            || error.to_string().contains("unknown resource uri")
    );
}

#[test]
fn docs_completion_lists_markdown_files() {
    let dir = TempDir::new().expect("tempdir");
    let docs_dir = dir.path().join("wiki");
    fs::create_dir_all(&docs_dir).expect("docs dir");
    fs::write(docs_dir.join("guide.md"), "# Guide\n").expect("guide");
    fs::write(docs_dir.join("notes.txt"), "ignore").expect("notes");

    let files = docs_completion_items(
        dir.path().to_str().expect("repo root"),
        "file",
        "wiki/",
        None,
    )
    .expect("file completion");
    assert_eq!(files, vec!["wiki/guide.md"]);
}

// ---------------------------------------------------------------------------
// atlas:// URI namespace
// ---------------------------------------------------------------------------

#[test]
fn uri_parse_and_format_round_trip_for_every_form() {
    let forms = [
        "atlas://repo/repo_abc/tree/src%2Fauth",
        "atlas://repo/repo_abc/tree/.",
        "atlas://repo/repo_abc/file/src%2Fauth%2Fmiddleware.rs",
        "atlas://repo/repo_abc/symbol/src%2Fauth.rs::fn::verify",
        "atlas://memories/topic/deploy",
        "atlas://memories/id/mem_123",
        "atlas://memoir/core/concept-name",
        "atlas://artifacts/src_123/2",
    ];
    for form in forms {
        let (parsed, level) = uris::parse_uri(form).expect(form);
        assert_eq!(parsed.format(), form);
        assert_eq!(level.as_str(), parsed.default_level().as_str());
    }
}

#[test]
fn uri_parse_rejects_traversal_absolute_and_empty_segments() {
    for bad in [
        "atlas://repo/repo_abc/tree/..",
        "atlas://repo/repo_abc/tree/%2E%2E",
        "atlas://repo/repo_abc/tree/a%2F..%2Fb",
        "atlas://repo/repo_abc//tree",
        "atlas://repo//tree/x",
        "atlas://repo/repo_abc/tree/src/",
        "not-atlas://repo/repo_abc/tree/src",
        "atlas://repo/repo_abc/tree/src?level=l3",
        "atlas://repo/repo_abc/tree/src?unknown=1",
    ] {
        assert!(uris::parse_uri(bad).is_err(), "{bad} must be rejected");
    }
}

#[test]
fn uri_level_query_only_allowed_on_tree_file_and_symbol() {
    for good in [
        "atlas://repo/repo_abc/tree/src?level=l0",
        "atlas://repo/repo_abc/file/src/lib.rs?level=l2",
        "atlas://repo/repo_abc/symbol/src%2Flib.rs::fn::x?level=l1",
    ] {
        let (_, level) = uris::parse_uri(good).expect(good);
        assert!(!level.as_str().is_empty());
    }
    for bad in [
        "atlas://memories/topic/deploy?level=l0",
        "atlas://memories/id/mem_1?level=l1",
        "atlas://memoir/core/concept?level=l2",
        "atlas://artifacts/src_1/0?level=l0",
    ] {
        assert!(uris::parse_uri(bad).is_err(), "{bad} must reject level");
    }
}

#[test]
fn uri_default_levels_match_documented_contract() {
    let (_, level) = uris::parse_uri("atlas://repo/repo_abc/tree/src").unwrap();
    assert_eq!(level.as_str(), "l1");
    let (_, level) = uris::parse_uri("atlas://repo/repo_abc/file/src/lib.rs").unwrap();
    assert_eq!(level.as_str(), "l1");
    let (_, level) = uris::parse_uri("atlas://repo/repo_abc/symbol/x").unwrap();
    assert_eq!(level.as_str(), "l1");
    let (_, level) = uris::parse_uri("atlas://artifacts/src_1/0").unwrap();
    assert_eq!(level.as_str(), "l2");
}

#[test]
fn uri_percent_encoding_round_trips_segments() {
    for raw in [
        "src/auth/middleware.rs",
        "a b+c",
        "src::fn::verify",
        "ünïcode",
    ] {
        let encoded = uris::encode_segment(raw);
        assert_eq!(uris::decode_segment(&encoded).unwrap(), raw);
    }
    assert!(uris::decode_segment("%2E%2E").is_ok());
    assert_eq!(uris::decode_segment("%2E%2E").unwrap(), "..");
}

#[test]
fn every_shipped_atlas_uri_template_is_listed() {
    let templates: Vec<String> = resource_template_descriptors()
        .into_iter()
        .map(|template| template.uri_template)
        .collect();
    for expected in [
        "atlas://repo/{repo_id}/tree/{dir}",
        "atlas://repo/{repo_id}/file/{path}",
        "atlas://repo/{repo_id}/symbol/{qualified_name}",
        "atlas://memories/topic/{topic}",
        "atlas://memories/id/{memory_id}",
        "atlas://memoir/{graph}/{concept}",
        "atlas://artifacts/{source_id}/{chunk_index}",
    ] {
        assert!(templates.contains(&expected.to_owned()), "{expected}");
    }
}

#[test]
fn uri_unknown_repo_id_is_rejected() {
    let dir = TempDir::new().expect("tempdir");
    let repo_root = dir.path().to_str().expect("repo root");
    let db_path = dir
        .path()
        .join(".atlas")
        .join("worldtree.db")
        .to_string_lossy()
        .into_owned();
    let error = uris::read_uri(
        "atlas://repo/repo_missing/tree/.",
        None,
        repo_root,
        &db_path,
    )
    .expect_err("unknown repo id must fail");
    assert!(error.to_string().contains("unknown repo id"));
}
