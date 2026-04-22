use super::*;

#[test]
fn fts5_escape_preserves_safe_prefix_query() {
    assert_eq!(fts5_escape("gre* OR tw*"), "gre* OR tw*");
}

#[test]
fn fts5_escape_quotes_unsafe_query() {
    assert_eq!(fts5_escape("gre* OR tw*(foo)"), "\"gre* OR tw*(foo)\"");
}
