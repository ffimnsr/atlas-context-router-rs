//! Versioned JSON/stdio protocol for out-of-process parser plugins.
//!
//! The plugin owns extraction completely: it receives the file bytes and returns
//! a serialized [`ParsedFile`]. Everything is validated before the graph sees
//! the output, and mutated identity fields are normalized to the request.

use atlas_core::{NodeId, ParsedFile};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};

/// Protocol identifier understood by this Atlas build.
pub(super) const PLUGIN_PROTOCOL: &str = "atlas-parser.v1";
/// Default per-file plugin timeout.
pub(super) const DEFAULT_TIMEOUT_MS: u64 = 30_000;
/// Default maximum accepted plugin response size (16 MiB).
pub(super) const DEFAULT_MAX_RESPONSE_BYTES: u64 = 16_777_216;

#[derive(Serialize)]
struct PluginRequest<'a> {
    protocol: &'a str,
    language_name: &'a str,
    path: &'a str,
    file_hash: &'a str,
    source_b64: String,
}

#[derive(Deserialize)]
struct PluginResponse {
    protocol: String,
    parsed_file: ParsedFile,
}

/// Serialize one parse request for the plugin stdin.
pub(super) fn encode_request(
    protocol: &str,
    language_name: &str,
    path: &str,
    file_hash: &str,
    source: &[u8],
) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&PluginRequest {
        protocol,
        language_name,
        path,
        file_hash,
        source_b64: STANDARD.encode(source),
    })
    .map_err(|error| format!("cannot encode plugin request: {error}"))
}

/// Validate and normalize one plugin response.
///
/// Rejects protocol, path, and hash mismatches plus node/edge file paths for a
/// different file. Node and edge ids are reset so the store assigns real ids,
/// and node identity metadata is normalized to the request.
pub(super) fn decode_response(
    bytes: &[u8],
    protocol: &str,
    language_name: &str,
    path: &str,
    file_hash: &str,
) -> Result<ParsedFile, String> {
    let response: PluginResponse =
        serde_json::from_slice(bytes).map_err(|error| format!("invalid JSON response: {error}"))?;
    if response.protocol != protocol {
        return Err(format!(
            "protocol mismatch: expected '{protocol}', got '{}'",
            response.protocol
        ));
    }

    let mut parsed = response.parsed_file;
    if parsed.path != path {
        return Err(format!(
            "response path mismatch: expected '{path}', got '{}'",
            parsed.path
        ));
    }
    if parsed.hash != file_hash {
        return Err(format!(
            "response hash mismatch for '{path}': expected '{file_hash}', got '{}'",
            parsed.hash
        ));
    }
    for node in &parsed.nodes {
        if node.file_path != path {
            return Err(format!(
                "node '{}' file_path mismatch: expected '{path}', got '{}'",
                node.qualified_name, node.file_path
            ));
        }
    }
    for edge in &parsed.edges {
        if edge.file_path != path {
            return Err(format!(
                "edge '{}' -> '{}' file_path mismatch: expected '{path}', got '{}'",
                edge.source_qn, edge.target_qn, edge.file_path
            ));
        }
    }

    parsed.language = Some(language_name.to_owned());
    for node in &mut parsed.nodes {
        node.id = NodeId::UNSET;
        node.file_hash = file_hash.to_owned();
        node.language = language_name.to_owned();
    }
    for edge in &mut parsed.edges {
        edge.id = 0;
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const PATH: &str = "src/main.cust";
    const HASH: &str = "hash-success";
    const LANGUAGE: &str = "custom";

    fn node_json(file_path: &str, file_hash: &str) -> serde_json::Value {
        json!({
            "id": 42,
            "kind": "function",
            "name": "greet",
            "qualified_name": "src/main.cust::function::greet",
            "file_path": file_path,
            "line_start": 1,
            "line_end": 1,
            "language": "plugin-language",
            "parent_name": null,
            "params": null,
            "return_type": null,
            "modifiers": null,
            "is_test": false,
            "file_hash": file_hash,
            "extra_json": null,
        })
    }

    fn edge_json(file_path: &str) -> serde_json::Value {
        json!({
            "id": 7,
            "kind": "contains",
            "source_qn": PATH,
            "target_qn": "src/main.cust::function::greet",
            "file_path": file_path,
            "line": 1,
            "confidence": 1.0,
            "confidence_tier": "definite",
            "extra_json": null,
        })
    }

    fn response_json(
        protocol: &str,
        path: &str,
        hash: &str,
        node_path: &str,
        edge_path: &str,
    ) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "protocol": protocol,
            "parsed_file": {
                "path": path,
                "language": "plugin-language",
                "hash": hash,
                "size": 12,
                "nodes": [node_json(node_path, hash)],
                "edges": [edge_json(edge_path)],
            }
        }))
        .expect("serialize response fixture")
    }

    #[test]
    fn request_round_trip_encodes_source_and_identity() {
        let bytes = encode_request(PLUGIN_PROTOCOL, LANGUAGE, PATH, HASH, b"fn greet() {}")
            .expect("encode request");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("request JSON");
        assert_eq!(value["protocol"], json!(PLUGIN_PROTOCOL));
        assert_eq!(value["language_name"], json!(LANGUAGE));
        assert_eq!(value["path"], json!(PATH));
        assert_eq!(value["file_hash"], json!(HASH));
        let decoded = STANDARD
            .decode(value["source_b64"].as_str().expect("source_b64"))
            .expect("base64 decode");
        assert_eq!(decoded, b"fn greet() {}");
    }

    #[test]
    fn response_validates_and_normalizes_identity() {
        let bytes = response_json(PLUGIN_PROTOCOL, PATH, HASH, PATH, PATH);
        let parsed =
            decode_response(&bytes, PLUGIN_PROTOCOL, LANGUAGE, PATH, HASH).expect("valid response");
        assert_eq!(parsed.language.as_deref(), Some(LANGUAGE));
        assert_eq!(parsed.nodes[0].id, NodeId::UNSET);
        assert_eq!(parsed.nodes[0].language, LANGUAGE);
        assert_eq!(parsed.nodes[0].file_hash, HASH);
        assert_eq!(parsed.edges[0].id, 0);
    }

    #[test]
    fn protocol_mismatch_is_rejected() {
        let bytes = response_json("atlas-parser.v2", PATH, HASH, PATH, PATH);
        let error = decode_response(&bytes, PLUGIN_PROTOCOL, LANGUAGE, PATH, HASH)
            .expect_err("protocol mismatch");
        assert!(
            error.contains("protocol mismatch"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn path_and_hash_mismatches_are_rejected() {
        let wrong_path = response_json(PLUGIN_PROTOCOL, "src/other.cust", HASH, PATH, PATH);
        let error = decode_response(&wrong_path, PLUGIN_PROTOCOL, LANGUAGE, PATH, HASH)
            .expect_err("path mismatch");
        assert!(error.contains("path mismatch"), "unexpected error: {error}");

        let wrong_hash = response_json(PLUGIN_PROTOCOL, PATH, "other", PATH, PATH);
        let error = decode_response(&wrong_hash, PLUGIN_PROTOCOL, LANGUAGE, PATH, HASH)
            .expect_err("hash mismatch");
        assert!(error.contains("hash mismatch"), "unexpected error: {error}");
    }

    #[test]
    fn nested_file_paths_are_rejected() {
        let wrong_node = response_json(PLUGIN_PROTOCOL, PATH, HASH, "src/other.cust", PATH);
        let error = decode_response(&wrong_node, PLUGIN_PROTOCOL, LANGUAGE, PATH, HASH)
            .expect_err("node path mismatch");
        assert!(error.contains("node '"), "unexpected error: {error}");

        let wrong_edge = response_json(PLUGIN_PROTOCOL, PATH, HASH, PATH, "src/other.cust");
        let error = decode_response(&wrong_edge, PLUGIN_PROTOCOL, LANGUAGE, PATH, HASH)
            .expect_err("edge path mismatch");
        assert!(error.contains("edge '"), "unexpected error: {error}");
    }

    #[test]
    fn malformed_json_is_rejected() {
        let error = decode_response(b"not json", PLUGIN_PROTOCOL, LANGUAGE, PATH, HASH)
            .expect_err("malformed JSON");
        assert!(error.contains("invalid JSON"), "unexpected error: {error}");
    }
}
