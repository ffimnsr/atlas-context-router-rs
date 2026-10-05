use super::*;
use std::path::Path;

fn list_symbols(fixture: &McpFixture, args: serde_json::Value) -> serde_json::Value {
    call(
        "list_symbols",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("list_symbols call")
}

#[test]
fn list_symbols_paginates_with_totals_and_next_offset() {
    let fixture = setup_mcp_fixture();

    let first = list_symbols(
        &fixture,
        serde_json::json!({"kind": "function", "limit": 1, "offset": 0}),
    );
    assert_provenance(&first, &fixture.repo_root, &fixture.db_path);
    let structured = &first["structuredContent"];
    assert_eq!(structured["tool"], "list_symbols");
    assert_eq!(structured["total"], 2);
    assert_eq!(structured["returned"], 1);
    assert_eq!(structured["has_more"], true);
    assert_eq!(structured["next_offset"], 1);
    assert_eq!(structured["offset"], 0);
    assert_eq!(
        structured["symbols"][0]["qualified_name"],
        "src/api.rs::fn::handle_request"
    );

    let second = list_symbols(
        &fixture,
        serde_json::json!({"kind": "function", "limit": 1, "offset": 1}),
    );
    let structured = &second["structuredContent"];
    assert_eq!(structured["total"], 2);
    assert_eq!(structured["returned"], 1);
    assert_eq!(structured["has_more"], false);
    assert!(structured["next_offset"].is_null());
    assert_eq!(
        structured["symbols"][0]["qualified_name"],
        "src/service.rs::fn::compute"
    );
}

#[test]
fn list_symbols_accepts_kind_aliases_and_filters() {
    let fixture = setup_mcp_fixture();

    let aliased = list_symbols(&fixture, serde_json::json!({"kind": "fn"}));
    assert_eq!(aliased["structuredContent"]["total"], 2);

    let subpath = list_symbols(
        &fixture,
        serde_json::json!({"kind": "function", "subpath": "src/service"}),
    );
    assert_eq!(subpath["structuredContent"]["total"], 1);
    assert_eq!(
        subpath["structuredContent"]["symbols"][0]["qualified_name"],
        "src/service.rs::fn::compute"
    );

    let no_match = list_symbols(&fixture, serde_json::json!({"kind": "enum"}));
    assert_eq!(no_match["structuredContent"]["total"], 0);
    assert_eq!(no_match["structuredContent"]["returned"], 0);
    assert!(no_match["structuredContent"]["next_offset"].is_null());
}

#[test]
fn list_symbols_clamps_page_size_and_reports_budget() {
    let fixture = setup_mcp_fixture();

    let page = list_symbols(&fixture, serde_json::json!({"limit": 100_000}));
    assert_eq!(page["structuredContent"]["limit"], 500);
    assert_eq!(page["budget_hit"], false);
    assert_eq!(
        page["budget_name"],
        "mcp_cli_payload_serialization.max_mcp_response_bytes"
    );
}

#[test]
fn list_symbols_filters_by_repo_id() {
    let fixture = setup_mcp_fixture();
    let repo_id = atlas_repo::stable_repo_id(Utf8Path::new(&fixture.repo_root));

    let current = list_symbols(&fixture, serde_json::json!({"repo_id": repo_id.as_str()}));
    assert_eq!(current["structuredContent"]["total"], 3);
    assert_eq!(
        current["structuredContent"]["filters"]["repo_id"],
        repo_id.as_str()
    );

    let missing = list_symbols(&fixture, serde_json::json!({"repo_id": "repo_missing"}));
    assert_eq!(missing["structuredContent"]["total"], 0);
    assert_eq!(missing["structuredContent"]["returned"], 0);
}

#[test]
fn list_symbols_trims_to_byte_budget_and_resumes() {
    let fixture = setup_mcp_fixture();

    let single = list_symbols(
        &fixture,
        serde_json::json!({"kind": "function", "limit": 1, "offset": 0}),
    );
    let both = list_symbols(
        &fixture,
        serde_json::json!({"kind": "function", "limit": 500}),
    );
    let structured_bytes = |value: &serde_json::Value| {
        serde_json::to_vec(&value["structuredContent"]).expect("structured content bytes")
    };
    let one_symbol_bytes = structured_bytes(&single).len();
    let two_symbol_bytes = structured_bytes(&both).len();
    assert!(
        one_symbol_bytes < two_symbol_bytes,
        "fixture pages should differ in size: one={one_symbol_bytes} two={two_symbol_bytes}"
    );

    // Configure a byte budget that fits one symbol but not two, so the trim
    // loop must drop the trailing symbol and advertise a resumable page.
    let byte_budget = one_symbol_bytes + (two_symbol_bytes - one_symbol_bytes) / 2;
    let max_bytes = byte_budget + 2048;
    let atlas_dir = Path::new(&fixture.repo_root).join(".atlas");
    std::fs::create_dir_all(&atlas_dir).expect("create .atlas dir");
    std::fs::write(
        atlas_dir.join("config.toml"),
        format!("[mcp]\nmax_mcp_response_bytes = {max_bytes}\n"),
    )
    .expect("write atlas config");

    let trimmed = list_symbols(
        &fixture,
        serde_json::json!({"kind": "function", "limit": 500}),
    );
    let structured = &trimmed["structuredContent"];
    assert_eq!(structured["truncated"], true);
    assert_eq!(structured["returned"], 1);
    assert_eq!(structured["next_offset"], 1);
    assert_eq!(structured["has_more"], true);
    assert_eq!(trimmed["budget_hit"], true);
    assert!(
        trimmed["budget_observed"].as_u64().unwrap_or_default() <= max_bytes as u64,
        "observed bytes must stay within budget: {trimmed:?}"
    );

    let resumed = list_symbols(
        &fixture,
        serde_json::json!({"kind": "function", "limit": 500, "offset": 1}),
    );
    assert_eq!(resumed["structuredContent"]["returned"], 1);
    assert_eq!(resumed["structuredContent"]["has_more"], false);
    assert!(resumed["structuredContent"]["next_offset"].is_null());
}
