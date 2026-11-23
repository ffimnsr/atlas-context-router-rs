use super::*;

#[test]
fn inject_instructions_creates_agents_md() {
    let tmp = TempDir::new().unwrap();
    let files =
        inject_instructions(tmp.path(), &[AGENTS_FILE], false, InstructionsMode::Refresh).unwrap();
    assert!(files.contains(&AGENTS_FILE.to_owned()));
    let content = fs::read_to_string(tmp.path().join(AGENTS_FILE)).unwrap();
    assert!(content.contains(INSTRUCTIONS_MARKER));
}

#[test]
fn inject_instructions_idempotent() {
    let tmp = TempDir::new().unwrap();
    inject_instructions(tmp.path(), &[AGENTS_FILE], false, InstructionsMode::Refresh).unwrap();
    let files =
        inject_instructions(tmp.path(), &[AGENTS_FILE], false, InstructionsMode::Refresh).unwrap();
    assert!(!files.contains(&AGENTS_FILE.to_owned()));
}

#[test]
fn inject_instructions_appends_to_existing() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join(AGENTS_FILE), b"# Existing content\n").unwrap();
    inject_instructions(tmp.path(), &[AGENTS_FILE], false, InstructionsMode::Refresh).unwrap();
    let content = fs::read_to_string(tmp.path().join(AGENTS_FILE)).unwrap();
    assert!(content.starts_with("# Existing content\n"));
    assert!(content.contains(INSTRUCTIONS_MARKER));
}

#[test]
fn instructions_section_excludes_generic_mcp_setup() {
    assert!(
        !INSTRUCTIONS_SECTION.contains(GENERIC_MCP_SETUP),
        "managed AGENTS.md block must not embed the generic MCP setup guidance"
    );
    assert!(GENERIC_MCP_SETUP.contains("command: atlas"));
    assert!(GENERIC_MCP_SETUP.contains("\"mcpServers\""));
    assert!(GENERIC_MCP_SETUP.contains("does not write per-tool configuration files"));
}

#[test]
fn instructions_section_stays_a_stable_pointer() {
    assert!(INSTRUCTIONS_SECTION.contains("atlas skills get core"));
    assert!(INSTRUCTIONS_SECTION.contains("atlas skills list"));
    assert!(INSTRUCTIONS_SECTION.contains("MANDATORY"));
    assert!(INSTRUCTIONS_SECTION.contains("STRICT RULE"));
    assert!(
        INSTRUCTIONS_SECTION.lines().count() < 40,
        "managed AGENTS.md block must stay a small pointer, not a growing guide"
    );
    for moved in [
        "Minimal-first escalation order",
        "`tool_list`",
        "Session memory fallback for hosts without hooks",
        "### Add Atlas as an MCP server",
    ] {
        assert!(
            !INSTRUCTIONS_SECTION.contains(moved),
            "`{moved}` belongs in skill-data guide content, not AGENTS.md"
        );
    }
}

#[test]
fn core_skill_mentions_tool_discovery_helpers() {
    assert!(GUIDE.contains("`tool_list`"));
    assert!(GUIDE.contains("`tool_search`"));
    assert!(GUIDE.contains("`tool_help`"));
    assert!(GUIDE.contains("Runtime docs are canonical"));
    assert!(GUIDE.contains("input contracts"));
}

#[test]
fn core_skill_puts_continuity_before_graph_lookup() {
    let continuity = GUIDE
        .find("## Continuity first")
        .expect("core guide needs a leading continuity section");
    let atlas_first = GUIDE
        .find("## Atlas first")
        .expect("core guide keeps the Atlas-first section");
    assert!(
        continuity < atlas_first,
        "continuity must be the first workflow, before graph lookup"
    );
    assert!(GUIDE.contains("`wake_up` once at the start of every session"));
    assert!(GUIDE.contains("Record, save, and feed back in every session"));
    for tool in [
        "`record_session_event`",
        "`save_context_artifact`",
        "`memory_store`",
        "`feedback_record`",
        "`compact_session`",
    ] {
        assert!(GUIDE.contains(tool), "core guide must document {tool}");
    }
}

#[test]
fn core_skill_documents_workflow_patterns() {
    for heading in ["## Workflow patterns", "## Plan and handoff"] {
        assert!(
            GUIDE.contains(heading),
            "core guide must expose the {heading} section"
        );
    }
    assert!(GUIDE.contains("### Update checklist before finishing substantive work"));
    for tool in [
        "`batch_query_graph`",
        "`concept_clusters`",
        "`cross_file_links`",
        "`cross_session_search`",
        "`analyze_architecture`",
        "`find_duplicates`",
        "`read_saved_context`",
    ] {
        assert!(
            GUIDE.contains(tool),
            "workflow patterns must suggest {tool}"
        );
    }
    assert!(GUIDE.contains("source_type: \"plan\""));
    assert!(GUIDE.contains("512 B"));
    assert!(GUIDE.contains("Artifacts live under `.atlas/artifacts/` and are committed"));
    assert!(GUIDE.contains("**Change traceability.**"));
    assert!(GUIDE.contains("### Failure triage and lifecycle"));
    assert!(GUIDE.contains("**Debug a failure.**"));
    assert!(GUIDE.contains("**Run periodic hygiene.**"));
}

#[test]
fn core_skill_documents_split_graph_build_tools() {
    assert!(GUIDE.contains("`build_graph` with `{}`"));
    assert!(GUIDE.contains("`update_graph` with direct `change_source`"));
    assert!(
        GUIDE
            .contains("`update_graph` with `{ \"change_source\": { \"kind\": \"working_tree\" } }")
    );
    assert!(!GUIDE.contains("build_or_update_graph"));
}

#[test]
fn continuity_reference_documents_session_memory_fallback() {
    let guide = guide_reference("continuity");
    assert!(guide.contains("Session memory fallback for hosts without hooks"));
    assert!(guide.contains("`record_session_event`"));
    assert!(guide.contains("`wake_up` when available"));
    assert!(guide.contains("`resume_session`"));
    assert!(guide.contains("`save_context_artifact`"));
    assert!(guide.contains("source_type: \"plan\""));
    assert!(guide.contains("`read_saved_context`"));
    assert!(guide.contains("`compact_session`"));
    assert!(guide.contains("`search_decisions`"));
    assert!(guide.contains("`get_global_memory`"));
    for event in [
        "user-prompt",
        "post-tool-use",
        "file-changed",
        "tool-failure",
        "pre-compact",
        "stop",
        "session-end",
    ] {
        assert!(
            guide.contains(&format!("\"{event}\"")),
            "fallback protocol must document event {event}"
        );
    }
    assert!(guide.contains("Do NOT store"));
    assert!(guide.contains("Never write any Atlas database directly"));
    assert!(guide.contains("Write and update by layer"));
    assert!(guide.contains("`replaces`"));
    assert!(guide.contains("supersedes the prior active record"));
    assert!(guide.contains("atlas://artifacts/{source_id}/{chunk_index}"));
    assert!(guide.contains("atlas://tool-docs/{name}"));
}

#[test]
fn workflows_reference_documents_continuity_updates() {
    let guide = guide_reference("workflows");
    for heading in [
        "## Build and update memoir knowledge",
        "## Maintain overview memory",
        "## Keep durable memory current",
        "## Recall and curate continuity",
    ] {
        assert!(
            guide.contains(heading),
            "workflows reference must expose {heading}"
        );
    }
    for needle in [
        "`replaces`",
        "`overview_refresh_plan`",
        "`overview_ingest`",
        "`patch_id`",
        "`memory_link`",
        "`experience_inspect`",
        "`memory_ops`",
    ] {
        assert!(
            guide.contains(needle),
            "workflows reference must document {needle}"
        );
    }
    assert!(guide.contains("\"schema_version\": 1"));
    assert!(guide.contains("Before finishing"));
    assert!(guide.contains("`supersedes`"));
    assert!(guide.contains("## Historical and derived graph surfaces"));
}

#[test]
fn inject_instructions_agents_md_points_at_core_skill() {
    let tmp = TempDir::new().unwrap();
    inject_instructions(tmp.path(), &[AGENTS_FILE], false, InstructionsMode::Refresh).unwrap();
    let content = fs::read_to_string(tmp.path().join(AGENTS_FILE)).unwrap();
    assert!(content.contains("atlas skills get core"));
    assert!(content.contains("atlas skills list"));
    assert!(
        !content.contains("Session memory fallback for hosts without hooks"),
        "detailed guide content belongs in the versioned skill, not AGENTS.md"
    );
}

#[test]
fn inject_instructions_fallback_protocol_preserves_user_content() {
    let tmp = TempDir::new().unwrap();
    let seeded = format!(
        "# Existing content\n\n{INSTRUCTIONS_MARKER}\nold stale block\n{INSTRUCTIONS_END_MARKER}\n\n## User Notes\nkeep me\n"
    );
    fs::write(tmp.path().join(AGENTS_FILE), seeded).unwrap();

    inject_instructions(tmp.path(), &[AGENTS_FILE], false, InstructionsMode::Refresh).unwrap();
    let content = fs::read_to_string(tmp.path().join(AGENTS_FILE)).unwrap();

    assert!(content.starts_with("# Existing content"));
    assert!(content.contains("## User Notes\nkeep me"));
    assert!(content.contains("atlas skills get core"));
}

#[test]
fn core_skill_references_runtime_docs_without_duplicating_full_tool_inventory() {
    let exported = exported_mcp_tool_names();
    let mentioned = exported
        .iter()
        .filter(|tool_name| GUIDE.contains(&format!("`{tool_name}`")))
        .count();

    assert!(
        mentioned < exported.len(),
        "instructions block should not duplicate full exported tool inventory"
    );
    assert!(GUIDE.contains("Do not treat this instruction block as exhaustive inventory."));
}

#[test]
fn core_skill_documents_trust_metadata() {
    assert!(GUIDE.contains("atlas_provenance"));
    assert!(GUIDE.contains("atlas_freshness"));
    assert!(GUIDE.contains(
        "Use default JSON output. Trust `structuredContent` as source of truth when present."
    ));
    assert!(GUIDE.contains("## Atlas first"));
    assert!(GUIDE.contains("Priority rule: if instructions conflict, prefer Atlas"));
    assert!(!GUIDE.contains("output_format"));
    assert!(!GUIDE.to_ascii_lowercase().contains("toon"));
    assert!(GUIDE.contains("status` or `doctor"));
    assert!(GUIDE.contains("Canonicalize repo paths before hashing"));
    assert!(GUIDE.contains("noncanonical_path_rows"));
}

#[test]
fn core_skill_documents_minimal_first_escalation() {
    assert!(GUIDE.contains("First code-exploration step: call `status`"));
    assert!(GUIDE.contains("Review/change tasks:"));
    assert!(GUIDE.contains("`get_minimal_context` for first bounded triage"));
    assert!(GUIDE.contains("Symbol/usage tasks:"));
    assert!(GUIDE.contains("`symbol_neighbors` for direct callers, callees, and tests"));
    assert!(GUIDE.contains("Allowed reasons to escalate:"));
    assert!(GUIDE.contains("Anti-patterns:"));
    assert!(GUIDE.contains("skipping Atlas because a terminal command feels faster"));
}

#[test]
fn inject_instructions_replaces_stale_section() {
    let tmp = TempDir::new().unwrap();
    let stale = format!("# Existing content\n\n{INSTRUCTIONS_MARKER}\nold stale block\n");
    fs::write(tmp.path().join(AGENTS_FILE), stale).unwrap();

    let files =
        inject_instructions(tmp.path(), &[AGENTS_FILE], false, InstructionsMode::Refresh).unwrap();
    let content = fs::read_to_string(tmp.path().join(AGENTS_FILE)).unwrap();

    assert!(files.contains(&AGENTS_FILE.to_owned()));
    assert!(content.contains(INSTRUCTIONS_END_MARKER));
    assert!(content.contains("atlas skills get core"));
    assert!(!content.contains("old stale block"));
}

#[test]
fn inject_instructions_replaces_marked_section_without_touching_suffix() {
    let tmp = TempDir::new().unwrap();
    let stale = format!(
        "# Existing content\n\n{INSTRUCTIONS_MARKER}\nold stale block\n{INSTRUCTIONS_END_MARKER}\n\n## User Notes\nkeep me\n"
    );
    fs::write(tmp.path().join(AGENTS_FILE), stale).unwrap();

    inject_instructions(tmp.path(), &[AGENTS_FILE], false, InstructionsMode::Refresh).unwrap();
    let content = fs::read_to_string(tmp.path().join(AGENTS_FILE)).unwrap();

    assert!(content.contains("## User Notes\nkeep me\n"));
    assert!(!content.contains("old stale block"));
}

#[test]
fn refresh_ignores_inline_marker_mentions_before_the_block() {
    let tmp = TempDir::new().unwrap();
    let seeded = format!(
        "# Notes\n\nMention `{INSTRUCTIONS_MARKER}` inline, not as a boundary.\n\n{INSTRUCTIONS_MARKER}\nold stale block\n{INSTRUCTIONS_END_MARKER}\n\n## User Notes\nkeep me\n"
    );
    fs::write(tmp.path().join(AGENTS_FILE), seeded).unwrap();

    inject_instructions(tmp.path(), &[AGENTS_FILE], false, InstructionsMode::Refresh).unwrap();
    let content = fs::read_to_string(tmp.path().join(AGENTS_FILE)).unwrap();

    assert!(content.contains("Mention `<!-- atlas MCP tools -->` inline"));
    assert!(content.contains("## User Notes\nkeep me"));
    assert!(!content.contains("old stale block"));
    assert!(content.contains("atlas skills get core"));
}

#[test]
fn refresh_appends_when_only_an_inline_marker_mention_exists() {
    let tmp = TempDir::new().unwrap();
    let seeded = "# Notes\n\nLiteral `<!-- atlas MCP tools -->` mention only.\n";
    fs::write(tmp.path().join(AGENTS_FILE), seeded).unwrap();

    inject_instructions(tmp.path(), &[AGENTS_FILE], false, InstructionsMode::Refresh).unwrap();
    let content = fs::read_to_string(tmp.path().join(AGENTS_FILE)).unwrap();

    assert!(content.starts_with("# Notes"));
    assert!(content.contains("Literal `<!-- atlas MCP tools -->` mention only."));
    assert!(content.contains(INSTRUCTIONS_END_MARKER));
}

#[test]
fn inject_instructions_replace_file_overwrites_existing_content() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join(AGENTS_FILE),
        "# Existing content\n\n## User Notes\nkeep me\n",
    )
    .unwrap();

    let files = inject_instructions(
        tmp.path(),
        &[AGENTS_FILE],
        false,
        InstructionsMode::ReplaceFile,
    )
    .unwrap();
    let content = fs::read_to_string(tmp.path().join(AGENTS_FILE)).unwrap();

    assert!(files.contains(&AGENTS_FILE.to_owned()));
    assert_eq!(content, INSTRUCTIONS_SECTION);
}
