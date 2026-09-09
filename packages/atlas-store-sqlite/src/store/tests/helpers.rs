use super::*;

#[test]
fn fts5_escape_preserves_safe_prefix_query() {
    assert_eq!(fts5_escape("gre* OR tw*"), "gre* OR tw*");
}

#[test]
fn fts5_escape_quotes_unsafe_query() {
    assert_eq!(fts5_escape("gre* OR tw*(foo)"), "\"gre* OR tw*(foo)\"");
}

#[test]
fn fts5_escape_quotes_bare_and_misplaced_operators() {
    for bare in ["AND", "OR", "NOT"] {
        assert_eq!(fts5_escape(bare), format!("\"{bare}\""), "bare {bare}");
    }
    assert_eq!(fts5_escape("OR x"), "\"OR x\"");
    assert_eq!(fts5_escape("x OR"), "\"x OR\"");
    assert_eq!(fts5_escape("x OR OR y"), "\"x OR OR y\"");
}

#[test]
fn fts5_escape_preserves_well_placed_operators_and_lowercase_words() {
    assert_eq!(fts5_escape("x OR y"), "x OR y");
    assert_eq!(fts5_escape("x AND y AND z"), "x AND y AND z");
    assert_eq!(fts5_escape("x NOT y"), "x NOT y");
    // FTS5 bare keywords are case-sensitive: lowercase spellings are terms.
    assert_eq!(fts5_escape("or"), "or");
    assert_eq!(fts5_escape("and"), "and");
    assert_eq!(fts5_escape("not"), "not");
}
