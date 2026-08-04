use super::*;

#[test]
fn split_camel_basic() {
    let parts = split_camel("ReplaceFileGraph");
    assert_eq!(parts, vec!["Replace", "File", "Graph"]);
}

#[test]
fn split_camel_all_lower() {
    let parts = split_camel("lowercase");
    assert_eq!(parts, vec!["lowercase"]);
}

#[test]
fn split_camel_acronym_boundary() {
    // "HTTPClient" → ["HTTP", "Client"]
    let parts = split_camel("HTTPClient");
    assert!(parts.len() >= 2, "expected at least 2 parts, got {parts:?}");
    assert_eq!(parts.last().unwrap(), "Client");
}

#[test]
fn build_fts_query_camel() {
    let q = build_fts_query("ReplaceFileGraph");
    assert!(q.contains("replace"), "should contain 'replace': {q}");
    assert!(q.contains("file"), "should contain 'file': {q}");
    assert!(q.contains("graph"), "should contain 'graph': {q}");
}

#[test]
fn build_fts_query_snake() {
    let q = build_fts_query("impact_radius");
    assert!(q.contains("impact"), "should contain 'impact': {q}");
    assert!(q.contains("radius"), "should contain 'radius': {q}");
}

#[test]
fn build_fts_query_plain() {
    let q = build_fts_query("simple");
    assert_eq!(q, "simple");
}

#[test]
fn build_relaxed_fts_query_plain() {
    let q = build_relaxed_fts_query("greter");
    assert_eq!(q, "gre*");
}

#[test]
fn build_relaxed_fts_query_snake() {
    let q = build_relaxed_fts_query("gret_twice");
    assert!(q.contains("gre*"), "expected typo prefix token: {q}");
    assert!(q.contains("tw*"), "expected stable suffix token: {q}");
}
