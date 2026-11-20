//! Canonical frontmatter parse and serialize.

use std::path::Path;

use crate::error::MarkdownError;
use crate::value::{Value, format_scalar, parse_scalar};

/// Ordered frontmatter fields.
///
/// Serialization preserves insertion order, which is what makes rewrites
/// byte-stable: parse keeps file order, so parse-then-serialize reproduces the
/// original bytes for canonical input.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Frontmatter {
    fields: Vec<(String, Value)>,
}

impl Frontmatter {
    /// Empty frontmatter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Borrow a field value.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.fields
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    /// Borrow a string field.
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    /// Borrow a bool field.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(Value::as_bool)
    }

    /// Borrow an integer field.
    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(Value::as_i64)
    }

    /// Borrow a list field.
    pub fn get_list(&self, key: &str) -> Option<&[Value]> {
        self.get(key).and_then(Value::as_list)
    }

    /// Set a field in place, or append it when new.
    pub fn set(&mut self, key: impl Into<String>, value: Value) {
        let key = key.into();
        if let Some((_, slot)) = self.fields.iter_mut().find(|(name, _)| *name == key) {
            *slot = value;
            return;
        }
        self.fields.push((key, value));
    }

    /// Remove a field, returning its value.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        let index = self.fields.iter().position(|(name, _)| name == key)?;
        Some(self.fields.remove(index).1)
    }

    /// Iterate fields in canonical order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.fields.iter().map(|(key, value)| (key.as_str(), value))
    }

    /// Number of fields.
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// `true` when no fields are present.
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Serialize to canonical bytes (without body).
    pub fn to_bytes(&self, path: &Path) -> Result<Vec<u8>, MarkdownError> {
        let mut out = String::from("---\n");
        for (key, value) in &self.fields {
            validate_key(key).map_err(|message| MarkdownError::InvalidValue {
                field: key.clone(),
                message,
            })?;
            match value {
                Value::List(items) => {
                    if items.is_empty() {
                        out.push_str(key);
                        out.push_str(": []\n");
                        continue;
                    }
                    out.push_str(key);
                    out.push_str(":\n");
                    for item in items {
                        match item {
                            Value::Map(entries) => {
                                out.push_str("  - { ");
                                push_map_entries(&mut out, entries)?;
                                out.push_str(" }\n");
                            }
                            Value::List(_) => {
                                return Err(MarkdownError::InvalidValue {
                                    field: key.clone(),
                                    message: "nested lists are not supported".to_string(),
                                });
                            }
                            scalar => {
                                let rendered = format_scalar(scalar).map_err(|message| {
                                    MarkdownError::InvalidValue {
                                        field: key.clone(),
                                        message,
                                    }
                                })?;
                                out.push_str("  - ");
                                out.push_str(&rendered);
                                out.push('\n');
                            }
                        }
                    }
                }
                Value::Map(entries) => {
                    if entries.is_empty() {
                        out.push_str(key);
                        out.push_str(": {}\n");
                        continue;
                    }
                    out.push_str(key);
                    out.push_str(": { ");
                    push_map_entries(&mut out, entries)?;
                    out.push_str(" }\n");
                }
                scalar => {
                    let rendered =
                        format_scalar(scalar).map_err(|message| MarkdownError::InvalidValue {
                            field: key.clone(),
                            message,
                        })?;
                    out.push_str(key);
                    out.push_str(": ");
                    out.push_str(&rendered);
                    out.push('\n');
                }
            }
        }
        out.push_str("---\n");
        let _ = path;
        Ok(out.into_bytes())
    }
}

fn push_map_entries(out: &mut String, entries: &[(String, Value)]) -> Result<(), MarkdownError> {
    for (index, (key, value)) in entries.iter().enumerate() {
        validate_key(key).map_err(|message| MarkdownError::InvalidValue {
            field: key.clone(),
            message,
        })?;
        if index > 0 {
            out.push_str(", ");
        }
        out.push_str(key);
        out.push_str(": ");
        let rendered = format_scalar(value).map_err(|message| MarkdownError::InvalidValue {
            field: key.clone(),
            message,
        })?;
        out.push_str(&rendered);
    }
    Ok(())
}

fn validate_key(key: &str) -> Result<(), String> {
    if key.is_empty() {
        return Err("empty field name".to_string());
    }
    if key
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        Ok(())
    } else {
        Err(format!("invalid field name `{key}`"))
    }
}

/// Parse a complete document, returning frontmatter and the body byte offset.
pub(crate) fn parse_document(
    bytes: &[u8],
    path: &Path,
) -> Result<(Frontmatter, usize), MarkdownError> {
    let text = std::str::from_utf8(bytes).map_err(|_| MarkdownError::InvalidUtf8 {
        path: path.to_path_buf(),
    })?;

    let mut lines = LineCursor::new(text);
    let first = lines
        .next_line()
        .ok_or_else(|| MarkdownError::MissingFence {
            path: path.to_path_buf(),
        })?;
    if first.text.trim_end_matches('\r') != "---" {
        return Err(MarkdownError::MissingFence {
            path: path.to_path_buf(),
        });
    }

    let mut frontmatter = Frontmatter::new();
    let mut open_list: Option<usize> = None;
    let mut line_number = 1usize;

    while let Some(line) = lines.next_line() {
        line_number += 1;
        let trimmed = line.text.trim_end_matches('\r');
        if trimmed == "---" {
            return Ok((frontmatter, line.next_offset));
        }
        if trimmed.trim().is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.starts_with(' ') {
            let item = trimmed.trim_start();
            let Some(item) = item.strip_prefix("- ") else {
                return Err(invalid(path, line_number, "expected list item"));
            };
            let Some(index) = open_list else {
                return Err(invalid(path, line_number, "list item without a list field"));
            };
            let value = if item.trim_start().starts_with('{') {
                parse_flow_map(item.trim(), path, line_number)?
            } else {
                parse_scalar(item).map_err(|message| invalid(path, line_number, message))?
            };
            if let Some((_, Value::List(items))) = frontmatter.fields.get_mut(index) {
                items.push(value);
            } else {
                return Err(invalid(path, line_number, "list item for non-list field"));
            }
            continue;
        }

        let Some((key, rest)) = split_field(trimmed) else {
            return Err(invalid(path, line_number, "expected `key: value`"));
        };
        validate_key(key).map_err(|message| invalid(path, line_number, message))?;
        let rest = rest.trim_start();
        let value = if rest.is_empty() {
            open_list = Some(frontmatter.fields.len());
            Value::List(Vec::new())
        } else if rest == "[]" {
            open_list = None;
            Value::List(Vec::new())
        } else if rest.starts_with('{') {
            open_list = None;
            parse_flow_map(rest, path, line_number)?
        } else {
            open_list = None;
            parse_scalar(rest).map_err(|message| invalid(path, line_number, message))?
        };
        frontmatter.fields.push((key.to_string(), value));
    }

    Err(MarkdownError::MissingFence {
        path: path.to_path_buf(),
    })
}

fn invalid(path: &Path, line: usize, message: impl Into<String>) -> MarkdownError {
    MarkdownError::InvalidFrontmatter {
        path: path.to_path_buf(),
        line,
        message: message.into(),
    }
}

fn split_field(line: &str) -> Option<(&str, &str)> {
    let colon = line.find(':')?;
    let key = line[..colon].trim();
    if key.is_empty() {
        return None;
    }
    Some((key, &line[colon + 1..]))
}

fn parse_flow_map(raw: &str, path: &Path, line: usize) -> Result<Value, MarkdownError> {
    let trimmed = raw.trim();
    let inner = trimmed
        .strip_prefix('{')
        .and_then(|value| value.strip_suffix('}'))
        .ok_or_else(|| invalid(path, line, "unterminated flow map"))?;
    let mut entries = Vec::new();
    if inner.trim().is_empty() {
        return Ok(Value::Map(entries));
    }
    for part in split_top_level(inner) {
        let (key, value) = split_field(part.trim())
            .ok_or_else(|| invalid(path, line, "expected `key: value` in flow map"))?;
        validate_key(key).map_err(|message| invalid(path, line, message))?;
        let parsed = parse_scalar(value.trim()).map_err(|message| invalid(path, line, message))?;
        entries.push((key.to_string(), parsed));
    }
    Ok(Value::Map(entries))
}

fn split_top_level(input: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut in_quotes = false;
    let mut escaped = false;
    for (index, ch) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_quotes => escaped = true,
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                parts.push(&input[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&input[start..]);
    parts
}

struct Line<'a> {
    text: &'a str,
    next_offset: usize,
}

struct LineCursor<'a> {
    text: &'a str,
    offset: usize,
}

impl<'a> LineCursor<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, offset: 0 }
    }

    fn next_line(&mut self) -> Option<Line<'a>> {
        if self.offset >= self.text.len() {
            return None;
        }
        let rest = &self.text[self.offset..];
        match rest.find('\n') {
            Some(index) => {
                let end = self.offset + index;
                let line = Line {
                    text: &self.text[self.offset..end],
                    next_offset: end + 1,
                };
                self.offset = end + 1;
                Some(line)
            }
            None => {
                let line = Line {
                    text: rest,
                    next_offset: self.text.len(),
                };
                self.offset = self.text.len();
                Some(line)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn path() -> PathBuf {
        PathBuf::from("memory.md")
    }

    fn round_trip(input: &str) -> Frontmatter {
        let (frontmatter, _) = parse_document(input.as_bytes(), &path()).expect("parse");
        let bytes = frontmatter.to_bytes(&path()).expect("serialize");
        let (again, _) = parse_document(&bytes, &path()).expect("re-parse");
        assert_eq!(frontmatter, again);
        frontmatter
    }

    #[test]
    fn canonical_round_trip_is_byte_identical() {
        let input = "---\ntitle: hello world\ncount: 3\nratio: 1.0\nflag: true\ntags:\n  - ci/cd\n  - storage\nlinks:\n  - { id: abc, relation: refines }\nmetadata: { category: decision }\n---\nbody\n";
        let (frontmatter, body_offset) = parse_document(input.as_bytes(), &path()).expect("parse");
        let bytes = frontmatter.to_bytes(&path()).expect("serialize");
        let expected = &input.as_bytes()[..body_offset];
        assert_eq!(bytes, expected);
    }

    #[test]
    fn comments_and_blank_lines_are_skipped() {
        let input = "---\n# comment\n\ntitle: x\n---\nbody";
        let frontmatter = round_trip(input);
        assert_eq!(frontmatter.get_str("title"), Some("x"));
        assert_eq!(frontmatter.len(), 1);
    }

    #[test]
    fn crlf_frontmatter_parses() {
        let input = "---\r\ntitle: x\r\n---\r\nbody";
        let (frontmatter, _) = parse_document(input.as_bytes(), &path()).expect("parse");
        assert_eq!(frontmatter.get_str("title"), Some("x"));
    }

    #[test]
    fn empty_list_and_map_round_trip_as_containers() {
        let frontmatter = round_trip("---\ntags: []\nmetadata: {}\n---\nbody\n");
        assert_eq!(frontmatter.get_list("tags"), Some(&[][..]));
        assert!(frontmatter.get("metadata").unwrap().as_map().is_some());

        let mut rebuilt = Frontmatter::new();
        rebuilt.set("tags", Value::List(Vec::new()));
        rebuilt.set("metadata", Value::Map(Vec::new()));
        let bytes = rebuilt.to_bytes(&path()).unwrap();
        let (parsed, _) = parse_document(&bytes, &path()).unwrap();
        assert_eq!(parsed.get_list("tags"), Some(&[][..]));
        assert!(parsed.get("metadata").unwrap().as_map().is_some());
    }

    #[test]
    fn empty_string_differs_from_absent() {
        let frontmatter = round_trip("---\nsource_id: \"\"\ntitle: x\n---\n");
        assert_eq!(frontmatter.get_str("source_id"), Some(""));
        assert_eq!(frontmatter.get("missing"), None);
    }

    #[test]
    fn missing_fence_is_an_error() {
        let error = parse_document(b"title: x\n", &path()).expect_err("must fail");
        assert!(matches!(error, MarkdownError::MissingFence { .. }));
    }

    #[test]
    fn unterminated_frontmatter_is_an_error() {
        let error = parse_document(b"---\ntitle: x\n", &path()).expect_err("must fail");
        assert!(matches!(error, MarkdownError::MissingFence { .. }));
    }

    #[test]
    fn invalid_line_reports_line_number() {
        let error = parse_document(b"---\nnot a field\n---\n", &path()).expect_err("must fail");
        match error {
            MarkdownError::InvalidFrontmatter { line, .. } => assert_eq!(line, 2),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn flow_map_with_quoted_comma() {
        let frontmatter = round_trip("---\nlinks:\n  - { id: abc, note: \"a, b\" }\n---\n");
        let entries = frontmatter.get_list("links").expect("links");
        let map = entries[0].as_map().expect("map");
        assert_eq!(map[1].0, "note");
        assert_eq!(map[1].1.as_str(), Some("a, b"));
    }

    #[test]
    fn nested_list_serialization_rejected() {
        let mut frontmatter = Frontmatter::new();
        frontmatter.set(
            "bad",
            Value::List(vec![Value::List(vec![Value::Str("x".into())])]),
        );
        assert!(frontmatter.to_bytes(&path()).is_err());
    }
}
