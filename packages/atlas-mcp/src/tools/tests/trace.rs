use super::*;
use serde_json::json;

#[test]
fn trace_calls_follows_callees_down() {
    let fixture = setup_mcp_fixture();
    let args = json!({
        "from": "src/api.rs::fn::handle_request",
        "direction": "down",
        "output_format": "json"
    });
    let response =
        call("trace_calls", Some(&args), "/ignored", &fixture.db_path).expect("trace_calls down");
    let nodes = response["structuredContent"]["nodes"]
        .as_array()
        .expect("nodes array");
    let names: Vec<&str> = nodes
        .iter()
        .filter_map(|node| node["qualified_name"].as_str())
        .collect();
    assert_eq!(
        names,
        vec![
            "src/api.rs::fn::handle_request",
            "src/service.rs::fn::compute"
        ]
    );
    assert_eq!(response["structuredContent"]["direction"], json!("down"));
}

#[test]
fn trace_calls_follows_callers_up() {
    let fixture = setup_mcp_fixture();
    let args = json!({
        "from": "src/service.rs::fn::compute",
        "direction": "up",
        "output_format": "json"
    });
    let response =
        call("trace_calls", Some(&args), "/ignored", &fixture.db_path).expect("trace_calls up");
    let nodes = response["structuredContent"]["nodes"]
        .as_array()
        .expect("nodes array");
    let names: Vec<&str> = nodes
        .iter()
        .filter_map(|node| node["qualified_name"].as_str())
        .collect();
    assert_eq!(
        names,
        vec![
            "src/service.rs::fn::compute",
            "src/api.rs::fn::handle_request"
        ]
    );
}

#[test]
fn trace_calls_edge_kind_filter_excludes_calls() {
    let fixture = setup_mcp_fixture();
    let args = json!({
        "from": "src/service.rs::fn::compute",
        "direction": "up",
        "edge_kinds": ["tests"],
        "output_format": "json"
    });
    let response = call("trace_calls", Some(&args), "/ignored", &fixture.db_path)
        .expect("trace_calls tests edges");
    let nodes = response["structuredContent"]["nodes"]
        .as_array()
        .expect("nodes array");
    assert_eq!(nodes.len(), 2);
    assert_eq!(
        nodes[1]["qualified_name"],
        json!("tests/service_test.rs::fn::compute_test")
    );
}

#[test]
fn trace_calls_finds_shortest_path() {
    let fixture = setup_mcp_fixture();
    let args = json!({
        "from": "src/api.rs::fn::handle_request",
        "to": "src/service.rs::fn::compute",
        "direction": "down",
        "output_format": "json"
    });
    let response =
        call("trace_calls", Some(&args), "/ignored", &fixture.db_path).expect("trace_calls path");
    assert_eq!(response["structuredContent"]["path"]["found"], json!(true));
    assert_eq!(
        response["structuredContent"]["path"]["nodes"],
        json!([
            "src/api.rs::fn::handle_request",
            "src/service.rs::fn::compute"
        ])
    );
}

#[test]
fn trace_calls_reports_unreachable_path() {
    let fixture = setup_mcp_fixture();
    let args = json!({
        "from": "src/service.rs::fn::compute",
        "to": "src/api.rs::fn::handle_request",
        "direction": "down",
        "output_format": "json"
    });
    let response = call("trace_calls", Some(&args), "/ignored", &fixture.db_path)
        .expect("trace_calls unreachable path");
    assert_eq!(response["structuredContent"]["path"]["found"], json!(false));
}

#[test]
fn trace_calls_accepts_plain_symbol_name() {
    let fixture = setup_mcp_fixture();
    let args = json!({
        "from": "compute",
        "direction": "up",
        "output_format": "json"
    });
    let response = call("trace_calls", Some(&args), "/ignored", &fixture.db_path)
        .expect("trace_calls plain name");
    assert_eq!(
        response["structuredContent"]["root_symbol"],
        json!("src/service.rs::fn::compute")
    );
}

#[test]
fn trace_calls_missing_symbol_returns_structured_error() {
    let fixture = setup_mcp_fixture();
    let args = json!({
        "from": "src/nope.rs::fn::missing",
        "output_format": "json"
    });
    let response = call("trace_calls", Some(&args), "/ignored", &fixture.db_path)
        .expect("missing symbol returns error envelope");
    assert_eq!(response["isError"], json!(true));
    assert_eq!(
        response["structuredContent"]["code"],
        json!("symbol_not_found")
    );
}

#[test]
fn trace_calls_invalid_direction_returns_structured_error() {
    let fixture = setup_mcp_fixture();
    let args = json!({
        "from": "src/service.rs::fn::compute",
        "direction": "sideways",
        "output_format": "json"
    });
    let response = call("trace_calls", Some(&args), "/ignored", &fixture.db_path)
        .expect("invalid direction returns error envelope");
    assert_eq!(response["isError"], json!(true));
    assert_eq!(
        response["structuredContent"]["code"],
        json!("invalid_input")
    );
}
