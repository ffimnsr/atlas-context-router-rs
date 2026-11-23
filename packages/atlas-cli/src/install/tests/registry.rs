use super::*;

#[test]
fn readme_mcp_tools_match_exported_registry() {
    assert_doc_mcp_tools_match_exported_registry(
        &repo_root().join("README.md"),
        "## MCP Tools",
        true,
    );
}

#[test]
fn wiki_mcp_reference_tools_match_exported_registry() {
    assert_doc_mcp_tools_match_exported_registry(
        &repo_root().join("wiki").join("mcp-reference.md"),
        "## Tool List",
        false,
    );
}

#[test]
fn generated_mcp_tools_markdown_matches_exported_registry() {
    let path = repo_root().join("MCP_TOOLS.md");
    let actual = fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "read generated MCP tools file {} failed: {err}",
            path.display()
        )
    });

    assert_eq!(actual, atlas_mcp::tool_list_markdown());
    assert!(actual.contains("serialized from `rmcp::model::Tool`"));
    assert!(actual.contains(
        "| Tool | Title | Input schema | Result contract | Output schema | Description |"
    ));
    assert!(!actual.to_ascii_lowercase().contains("toon"));
}

#[test]
fn continuity_reference_only_uses_supported_fallback_events() {
    let guide = guide_reference("continuity");
    let events = extract_event_literals(&guide);
    assert!(
        events.len() >= 7,
        "fallback protocol must reference its capture events: {events:?}"
    );
    for event in &events {
        assert!(
            atlas_mcp::is_supported_event_name(event),
            "installed instructions reference unsupported event name: {event}"
        );
    }
}

#[test]
fn wiki_hook_docs_and_fallback_page_agree_on_event_names() {
    for (path, pascal_cells) in [
        ("wiki/hooks-claude.md", true),
        ("wiki/hooks-codex.md", true),
        ("wiki/session-memory-fallback.md", false),
    ] {
        let full = repo_root().join(path);
        let text =
            fs::read_to_string(&full).unwrap_or_else(|err| panic!("read {path} failed: {err}"));
        let names: Vec<String> = if pascal_cells {
            extract_pascal_table_event_names(&text)
        } else {
            extract_event_literals(&text)
        };
        assert!(!names.is_empty(), "{path} must document event names");
        for name in &names {
            assert!(
                atlas_mcp::is_supported_event_name(name),
                "{path} describes unsupported event name: {name}"
            );
        }
    }
}

#[test]
fn dry_run_reports_instruction_files_without_writing() {
    let tmp = TempDir::new().unwrap();

    let summary = run_install(
        tmp.path(),
        InstallOptions {
            dry_run: true,
            no_hooks: true,
            ..Default::default()
        },
    )
    .unwrap();

    assert!(
        summary.instruction_files.contains(&AGENTS_FILE.to_owned()),
        "dry-run must report the instruction file it would update: {:?}",
        summary.instruction_files
    );
    assert!(
        !tmp.path().join(AGENTS_FILE).exists(),
        "dry-run must not write instruction files"
    );
    assert!(
        summary.hook_paths.is_empty() && summary.hook_runner_files.is_empty(),
        "dry-run with instruction-only surface must not report hooks"
    );
}
