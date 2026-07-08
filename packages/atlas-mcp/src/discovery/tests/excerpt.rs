use super::super::*;
use crate::output::OutputFormat;
use atlas_core::{NodeId, kinds::NodeKind, model::Node};
use atlas_store_sqlite::Store;
use serde_json::json;
use std::fs;
use std::path::Path;

fn make_repo(files: &[(&str, &str)]) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_str().unwrap().to_owned();
    fs::create_dir_all(format!("{root}/.git")).unwrap();
    for (rel, content) in files {
        let full = format!("{root}/{rel}");
        if let Some(parent) = Path::new(&full).parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&full, content).unwrap();
    }
    (dir, root)
}

fn markdown_heading(title: &str, path: &str, level: u32, start_line: u32) -> Node {
    Node {
        id: NodeId::UNSET,
        kind: NodeKind::Module,
        name: title.to_owned(),
        qualified_name: format!("README.md::heading::{path}"),
        file_path: "README.md".to_owned(),
        line_start: start_line,
        line_end: start_line,
        language: "markdown".to_owned(),
        parent_name: None,
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: "hash:README.md".to_owned(),
        extra_json: json!({ "level": level, "path": path }),
    }
}

fn seed_docs_index(root: &str, nodes: &[Node]) -> String {
    let db_path = format!("{root}/atlas.db");
    let mut store = Store::open(&db_path).expect("open store");
    store
        .replace_file_graph(
            "README.md",
            "hash:README.md",
            Some("markdown"),
            Some(32),
            nodes,
            &[],
        )
        .expect("replace README graph");
    db_path
}

// -----------------------------------------------------------------------
// read_file_excerpt
// -----------------------------------------------------------------------

#[test]
fn read_file_excerpt_reads_single_range() {
    let (_dir, root) = make_repo(&[(
        "src/lib.rs",
        "fn one() {}\nfn two() {}\nfn three() {}\nfn four() {}\n",
    )]);
    let args = serde_json::json!({
        "file": "src/lib.rs",
        "start_line": 2,
        "end_line": 3,
    });
    let resp = tool_read_file_excerpt(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();

    assert_eq!(v["atlas_result_kind"], "file_excerpt");
    assert_eq!(v["mode"], "single_range");
    assert_eq!(v["excerpt_count"], 1);
    assert_eq!(v["excerpts"][0]["start_line"], 2);
    assert_eq!(v["excerpts"][0]["end_line"], 3);
    assert!(
        v["excerpts"][0]["content"]
            .as_str()
            .is_some_and(|text| { text.contains("fn two() {}") && text.contains("fn three() {}") })
    );
}

#[test]
fn read_file_excerpt_ignores_absent_equivalent_wrapper_fields_for_single_range() {
    let (_dir, root) = make_repo(&[(
        "src/lib.rs",
        "fn one() {}\nfn two() {}\nfn three() {}\nfn four() {}\n",
    )]);
    let args = serde_json::json!({
        "file": "src/lib.rs",
        "start_line": 2,
        "end_line": 3,
        "line": 0,
        "before": 0,
        "after": 0,
        "line_ranges": [],
    });
    let resp = tool_read_file_excerpt(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();

    assert_eq!(v["mode"], "single_range");
    assert_eq!(v["excerpt_count"], 1);
    assert_eq!(v["excerpts"][0]["start_line"], 2);
    assert_eq!(v["excerpts"][0]["end_line"], 3);
}

#[test]
fn read_file_excerpt_ignores_zero_line_when_line_ranges_is_real_selector() {
    let (_dir, root) = make_repo(&[(
        "src/lib.rs",
        "fn one() {}\nfn two() {}\nfn three() {}\nfn four() {}\n",
    )]);
    let args = serde_json::json!({
        "file": "src/lib.rs",
        "start_line": 0,
        "end_line": 0,
        "line": 0,
        "before": 0,
        "after": 0,
        "line_ranges": [
            { "start_line": 2, "end_line": 4 }
        ],
    });
    let resp = tool_read_file_excerpt(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();

    assert_eq!(v["mode"], "line_ranges");
    assert_eq!(v["excerpt_count"], 1);
    assert_eq!(v["excerpts"][0]["start_line"], 2);
    assert_eq!(v["excerpts"][0]["end_line"], 4);
}

#[test]
fn read_file_excerpt_supports_line_with_context() {
    let (_dir, root) = make_repo(&[(
        "src/lib.rs",
        "fn one() {}\nfn two() {}\nfn three() {}\nfn four() {}\n",
    )]);
    let args = serde_json::json!({
        "file": "src/lib.rs",
        "line": 3,
        "before": 1,
        "after": 1,
    });
    let resp = tool_read_file_excerpt(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();

    assert_eq!(v["mode"], "line_context");
    assert_eq!(v["excerpts"][0]["start_line"], 2);
    assert_eq!(v["excerpts"][0]["end_line"], 4);
    let lines = v["excerpts"][0]["lines"].as_array().expect("excerpt lines");
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[1]["line"], 3);
    assert_eq!(lines[1]["text"], "fn three() {}");
}

#[test]
fn read_file_excerpt_merges_overlapping_ranges() {
    let (_dir, root) = make_repo(&[(
        "src/lib.rs",
        "fn one() {}\nfn two() {}\nfn three() {}\nfn four() {}\n",
    )]);
    let args = serde_json::json!({
        "file": "src/lib.rs",
        "line_ranges": [
            { "start_line": 1, "end_line": 2 },
            { "start_line": 2, "end_line": 4 }
        ],
    });
    let resp = tool_read_file_excerpt(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();

    assert_eq!(v["mode"], "line_ranges");
    assert_eq!(v["excerpt_count"], 1);
    assert_eq!(v["excerpts"][0]["start_line"], 1);
    assert_eq!(v["excerpts"][0]["end_line"], 4);
}

#[test]
fn read_file_excerpt_truncates_to_budgeted_max_lines() {
    let (_dir, root) = make_repo(&[(
        "src/lib.rs",
        "fn one() {}\nfn two() {}\nfn three() {}\nfn four() {}\n",
    )]);
    let args = serde_json::json!({
        "file": "src/lib.rs",
        "start_line": 1,
        "end_line": 4,
        "max_lines": 2,
    });
    let resp = tool_read_file_excerpt(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();

    assert_eq!(resp["budget_name"], "review_context_extraction.max_nodes");
    assert_eq!(resp["budget_limit"], 2);
    assert_eq!(resp["budget_observed"], 4);
    assert_eq!(resp["budget_hit"], true);
    assert_eq!(v["truncated"], true);
    assert_eq!(v["excerpts"][0]["end_line"], 2);
}

#[test]
fn read_file_excerpt_conflicting_real_selectors_return_actionable_error() {
    let (_dir, root) = make_repo(&[("src/lib.rs", "fn x() {}\nfn y() {}\n")]);
    let args = serde_json::json!({
        "file": "src/lib.rs",
        "start_line": 1,
        "end_line": 1,
        "line": 2,
        "before": 0,
        "after": 0,
    });
    let result = tool_read_file_excerpt(Some(&args), &root, OutputFormat::Json)
        .expect("conflicting selectors should return tool error result");
    let message = result["structuredContent"]["message"]
        .as_str()
        .expect("message");
    let details = &result["structuredContent"]["details"];

    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["structuredContent"]["code"],
        serde_json::json!("invalid_input")
    );
    assert!(
        message.contains("provide exactly one selector"),
        "got: {message}"
    );
    assert_eq!(
        details["selector_families_seen"],
        serde_json::json!("start_line/end_line, line with optional before/after")
    );
    assert_eq!(
        details["accepted_argument_families"],
        serde_json::json!([
            "line_ranges",
            "start_line/end_line",
            "line with optional before/after"
        ])
    );
    assert_eq!(
        details["retry_example"],
        serde_json::json!({ "file": "src/lib.rs", "start_line": 10, "end_line": 20 })
    );
    assert_eq!(
        details["fail_closed_reason"],
        serde_json::json!("Atlas refused to guess between conflicting selector families")
    );
}

#[test]
fn read_file_excerpt_zero_line_with_context_returns_actionable_error() {
    let (_dir, root) = make_repo(&[("src/lib.rs", "fn x() {}\nfn y() {}\n")]);
    let args = serde_json::json!({
        "file": "src/lib.rs",
        "line": 0,
        "before": 2,
        "after": 2,
    });
    let result = tool_read_file_excerpt(Some(&args), &root, OutputFormat::Json)
        .expect("invalid line context should return tool error result");
    let details = &result["structuredContent"]["details"];

    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["structuredContent"]["code"],
        serde_json::json!("invalid_input")
    );
    assert_eq!(
        details["detail"],
        serde_json::json!("line-context selector requires line >= 1")
    );
    assert_eq!(
        details["retry_example"],
        serde_json::json!({ "file": "src/lib.rs", "line": 42, "before": 2, "after": 2 })
    );
    assert_eq!(
        details["offending_fields"],
        serde_json::json!(["line", "before", "after"])
    );
}

#[test]
fn read_file_excerpt_path_traversal_is_rejected() {
    let (_dir, root) = make_repo(&[("src/lib.rs", "fn x() {}\n")]);
    for bad in &["../", "../../etc/passwd", "/etc/passwd"] {
        let args = serde_json::json!({
            "file": bad,
            "start_line": 1,
            "end_line": 1,
        });
        let result = tool_read_file_excerpt(Some(&args), &root, OutputFormat::Json)
            .expect("path traversal should return tool error result");
        assert_eq!(result["isError"], serde_json::json!(true));
        assert_eq!(
            result["structuredContent"]["code"],
            serde_json::json!("invalid_input"),
            "file path '{bad}' should be rejected as traversal attempt"
        );
    }
}

#[test]
fn read_file_excerpt_duplicate_root_prefix_returns_repo_relative_hint() {
    let (_dir, root) = make_repo(&[("src/lib.rs", "fn x() {}\n")]);
    let repo_name = Path::new(&root)
        .file_name()
        .and_then(|name| name.to_str())
        .expect("repo name");
    let args = serde_json::json!({
        "file": format!("{repo_name}/src/lib.rs"),
        "start_line": 1,
        "end_line": 1,
    });
    let result = tool_read_file_excerpt(Some(&args), &root, OutputFormat::Json)
        .expect("duplicate root prefix should return tool error result");
    let details = &result["structuredContent"]["details"];

    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["structuredContent"]["code"],
        serde_json::json!("invalid_input")
    );
    assert_eq!(details["repo_root"], serde_json::json!(root));
    assert_eq!(
        details["workspace_root_prefix"],
        serde_json::json!(format!("{repo_name}/"))
    );
    assert_eq!(
        details["suggested_repo_relative_path"],
        serde_json::json!("src/lib.rs")
    );
    assert_eq!(
        details["suggestion_reason"],
        serde_json::json!("duplicated_root_prefix")
    );
    assert_eq!(details["accepted_root_prefixes"], serde_json::json!([""]));
}

#[test]
fn read_file_excerpt_nested_root_prefix_returns_repo_relative_hint() {
    let (_dir, root) = make_repo(&[("src/lib.rs", "fn x() {}\n")]);
    let args = serde_json::json!({
        "file": "clients/mach-one/src/lib.rs",
        "start_line": 1,
        "end_line": 1,
    });
    let result = tool_read_file_excerpt(Some(&args), &root, OutputFormat::Json)
        .expect("nested root prefix should return tool error result");
    let details = &result["structuredContent"]["details"];

    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        details["suggested_repo_relative_path"],
        serde_json::json!("src/lib.rs")
    );
    assert_eq!(
        details["suggestion_reason"],
        serde_json::json!("nested_subdir_root_prefix")
    );
    assert!(
        details["canonical_path_guidance"]
            .as_str()
            .is_some_and(|value| value.contains("repo-relative paths under current repo root"))
    );
}

#[test]
fn read_file_excerpt_valid_repo_relative_path_under_current_root_succeeds() {
    let (_dir, root) = make_repo(&[("src/lib.rs", "fn x() {}\nfn y() {}\n")]);
    let args = serde_json::json!({
        "file": "src/lib.rs",
        "start_line": 1,
        "end_line": 1,
    });
    let result = tool_read_file_excerpt(Some(&args), &root, OutputFormat::Json)
        .expect("valid repo-relative path should succeed");

    assert_eq!(result["isError"], serde_json::Value::Null);
    let body: serde_json::Value =
        serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(body["file"], serde_json::json!("src/lib.rs"));
}

#[test]
fn read_file_excerpt_missing_file_uses_unique_basename_suggestion() {
    let (_dir, root) = make_repo(&[("crate/service.rs", "fn x() {}\n")]);
    let args = serde_json::json!({
        "file": "src/service.rs",
        "start_line": 1,
        "end_line": 1,
    });
    let result = tool_read_file_excerpt(Some(&args), &root, OutputFormat::Json)
        .expect("missing file should return tool error result");
    let details = &result["structuredContent"]["details"];

    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["structuredContent"]["code"],
        serde_json::json!("file_not_found")
    );
    assert_eq!(
        details["suggested_repo_relative_path"],
        serde_json::json!("crate/service.rs")
    );
    assert_eq!(
        details["suggestion_reason"],
        serde_json::json!("unique_basename_match")
    );
}

#[test]
fn read_file_excerpt_ambiguous_root_recovery_still_fails_closed() {
    let (_dir, root) = make_repo(&[("src/lib.rs", "fn x() {}\n"), ("lib.rs", "fn y() {}\n")]);
    let args = serde_json::json!({
        "file": "workspace/src/lib.rs",
        "start_line": 1,
        "end_line": 1,
    });
    let result = tool_read_file_excerpt(Some(&args), &root, OutputFormat::Json)
        .expect("ambiguous recovery should return tool error result");
    let details = &result["structuredContent"]["details"];
    let candidate_paths = details["candidate_paths"]
        .as_array()
        .expect("candidate paths array");

    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["structuredContent"]["code"],
        serde_json::json!("invalid_input")
    );
    assert!(details.get("suggested_repo_relative_path").is_none());
    assert_eq!(candidate_paths.len(), 2);
    assert_eq!(
        details["ambiguity"],
        serde_json::json!(
            "multiple deterministic repo-relative candidates exist; Atlas refused to guess"
        )
    );
}

// -----------------------------------------------------------------------
// get_docs_section
// -----------------------------------------------------------------------

#[test]
fn get_docs_section_by_heading_path_returns_section() {
    let (_dir, root) = make_repo(&[(
        "README.md",
        "# Overview\nintro\n## Install\nstep one\n## Usage\nrun it\n",
    )]);
    let db_path = seed_docs_index(
        &root,
        &[
            markdown_heading("Overview", "document.overview", 1, 1),
            markdown_heading("Install", "document.overview.install", 2, 3),
            markdown_heading("Usage", "document.overview.usage", 2, 5),
        ],
    );
    let args = serde_json::json!({
        "file": "README.md",
        "heading": "document.overview.install",
    });
    let resp = tool_get_docs_section(Some(&args), &root, &db_path, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();

    assert_eq!(v["atlas_result_kind"], "docs_section");
    assert_eq!(v["heading_path"], "document.overview.install");
    assert_eq!(v["heading_level"], 2);
    assert!(
        v["content"]
            .as_str()
            .is_some_and(|text| text.contains("step one"))
    );
}

#[test]
fn get_docs_section_by_line_returns_containing_section() {
    let (_dir, root) = make_repo(&[(
        "README.md",
        "# Overview\nintro\n## Install\nstep one\n## Usage\nrun it\n",
    )]);
    let db_path = seed_docs_index(
        &root,
        &[
            markdown_heading("Overview", "document.overview", 1, 1),
            markdown_heading("Install", "document.overview.install", 2, 3),
            markdown_heading("Usage", "document.overview.usage", 2, 5),
        ],
    );
    let args = serde_json::json!({
        "file": "README.md",
        "line": 4,
    });
    let resp = tool_get_docs_section(Some(&args), &root, &db_path, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();

    assert_eq!(v["heading_path"], "document.overview.install");
    assert_eq!(v["start_line"], 3);
    assert_eq!(v["end_line"], 4);
}

#[test]
fn get_docs_section_returns_candidates_for_ambiguous_slug() {
    let (_dir, root) = make_repo(&[("README.md", "# One\n## Install\na\n# Two\n## Install\nb\n")]);
    let db_path = seed_docs_index(
        &root,
        &[
            markdown_heading("One", "document.one", 1, 1),
            markdown_heading("Install", "document.one.install", 2, 2),
            markdown_heading("Two", "document.two", 1, 4),
            markdown_heading("Install", "document.two.install", 2, 5),
        ],
    );
    let args = serde_json::json!({
        "file": "README.md",
        "heading": "install",
    });
    let resp = tool_get_docs_section(Some(&args), &root, &db_path, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();

    assert_eq!(v["resolved"], false);
    assert_eq!(v["candidates"][0]["heading_path"], "document.one.install");
    assert_eq!(v["candidates"][1]["heading_path"], "document.two.install");
}

// -----------------------------------------------------------------------
// read_file_around_match
// -----------------------------------------------------------------------

#[test]
fn read_file_around_match_groups_nearby_matches() {
    let (_dir, root) = make_repo(&[(
        "src/lib.rs",
        "zero\nalpha target\nbeta\ngamma target\nomega\n",
    )]);
    let args = serde_json::json!({
        "file": "src/lib.rs",
        "query": "target",
        "before": 1,
        "after": 1,
    });
    let resp = tool_read_file_around_match(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();

    assert_eq!(v["atlas_result_kind"], "file_match_snippets");
    assert_eq!(v["total_matches"], 2);
    assert_eq!(v["snippet_count"], 1);
    let snippet = &v["snippets"][0];
    assert_eq!(snippet["start_line"], 1);
    assert_eq!(snippet["end_line"], 5);
    assert_eq!(snippet["match_lines"][0], 2);
    assert_eq!(snippet["match_lines"][1], 4);
}

#[test]
fn read_file_around_match_supports_regex() {
    let (_dir, root) = make_repo(&[("src/lib.rs", "alpha\nBeta\ngamma\n")]);
    let args = serde_json::json!({
        "file": "src/lib.rs",
        "query": r"^[A-Z][a-z]+$",
        "is_regex": true,
        "before": 0,
        "after": 0,
    });
    let resp = tool_read_file_around_match(Some(&args), &root, OutputFormat::Json).unwrap();
    let v: serde_json::Value =
        serde_json::from_str(resp["content"][0]["text"].as_str().unwrap()).unwrap();

    assert_eq!(v["total_matches"], 1);
    assert_eq!(v["snippets"][0]["match_lines"][0], 2);
}

#[test]
fn read_file_around_match_path_traversal_is_rejected() {
    let (_dir, root) = make_repo(&[("src/lib.rs", "target\n")]);
    let args = serde_json::json!({
        "file": "../etc/passwd",
        "query": "target",
    });
    let result = tool_read_file_around_match(Some(&args), &root, OutputFormat::Json)
        .expect("path traversal should return tool error result");

    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["structuredContent"]["code"],
        serde_json::json!("invalid_input")
    );
    assert_eq!(
        result["structuredContent"]["details"]["path"],
        serde_json::json!("../etc/passwd")
    );
}

// -----------------------------------------------------------------------
// get_docs_section errors
// -----------------------------------------------------------------------

#[test]
fn get_docs_section_invalid_selector_returns_tool_error_result() {
    let (_dir, root) = make_repo(&[("README.md", "# Overview\nintro\n## Install\nstep one\n")]);
    let db_path = seed_docs_index(
        &root,
        &[
            markdown_heading("Overview", "document.overview", 1, 1),
            markdown_heading("Install", "document.overview.install", 2, 3),
        ],
    );
    let args = serde_json::json!({
        "file": "README.md",
        "heading": "document.overview.install",
        "line": 3,
    });
    let result = tool_get_docs_section(Some(&args), &root, &db_path, OutputFormat::Json)
        .expect("invalid selector must return tool error result");

    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["structuredContent"]["code"],
        serde_json::json!("invalid_input")
    );
}
