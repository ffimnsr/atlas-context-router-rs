use std::collections::HashMap;

use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile};
use toml::Value;

use crate::traits::{LangParser, ParseContext};

pub struct TomlParser;

impl LangParser for TomlParser {
    fn language_name(&self) -> &'static str {
        "toml"
    }

    fn supports(&self, path: &str) -> bool {
        path.ends_with(".toml")
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> (ParsedFile, Option<tree_sitter::Tree>) {
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let line_count = ctx.source.iter().filter(|&&b| b == b'\n').count() as u32 + 1;
        nodes.push(file_node(ctx.rel_path, ctx.file_hash, line_count));

        if let Ok(source) = std::str::from_utf8(ctx.source)
            && let Ok(value) = source.parse::<Value>()
        {
            let index = TomlLineIndex::from_source(source);
            let mut walker = TomlWalker {
                ctx,
                index,
                nodes: Vec::new(),
                edges: Vec::new(),
            };
            walker.walk_root(&value);
            nodes.append(&mut walker.nodes);
            edges.append(&mut walker.edges);
        }

        (
            ParsedFile {
                path: ctx.rel_path.to_owned(),
                language: Some("toml".to_owned()),
                hash: ctx.file_hash.to_owned(),
                size: Some(ctx.source.len() as i64),
                nodes,
                edges,
            },
            None,
        )
    }
}

fn file_node(rel_path: &str, file_hash: &str, line_end: u32) -> Node {
    Node {
        id: NodeId::UNSET,
        kind: NodeKind::File,
        name: rel_path.rsplit('/').next().unwrap_or(rel_path).to_owned(),
        qualified_name: rel_path.to_owned(),
        file_path: rel_path.to_owned(),
        line_start: 1,
        line_end,
        language: "toml".to_owned(),
        parent_name: None,
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
    }
}

fn contains_edge(parent_qn: &str, child_qn: &str, file_path: &str, line: u32) -> Edge {
    Edge {
        id: 0,
        kind: EdgeKind::Contains,
        source_qn: parent_qn.to_owned(),
        target_qn: child_qn.to_owned(),
        file_path: file_path.to_owned(),
        line: Some(line),
        confidence: 1.0,
        confidence_tier: Some("definite".to_owned()),
        extra_json: serde_json::Value::Null,
    }
}

#[derive(Default)]
struct TomlLineIndex {
    table_lines: HashMap<String, u32>,
    key_lines: HashMap<String, u32>,
}

impl TomlLineIndex {
    fn from_source(source: &str) -> Self {
        let mut out = Self::default();
        let mut current_table: Vec<String> = Vec::new();

        for (offset, raw_line) in source.lines().enumerate() {
            let line_no = offset as u32 + 1;
            let stripped = strip_toml_comment(raw_line);
            let line = stripped.trim();
            if line.is_empty() {
                continue;
            }

            if line.starts_with('[') && line.ends_with(']') && !line.starts_with("[[") {
                let inner = &line[1..line.len() - 1];
                let path = normalize_toml_path(inner);
                if !path.is_empty() {
                    out.table_lines.entry(path.clone()).or_insert(line_no);
                    current_table = split_dotted_path(&path);
                }
                continue;
            }

            if let Some((key, _value)) = split_toml_key_value(line) {
                let mut full_path = current_table.clone();
                full_path.extend(split_dotted_path(key));
                let joined = full_path.join(".");
                if !joined.is_empty() {
                    out.key_lines.entry(joined).or_insert(line_no);
                }
            }
        }

        out
    }

    fn table_line(&self, path: &str) -> Option<u32> {
        self.table_lines.get(path).copied()
    }

    fn key_line(&self, path: &str) -> Option<u32> {
        self.key_lines.get(path).copied()
    }
}

struct TomlWalker<'a> {
    ctx: &'a ParseContext<'a>,
    index: TomlLineIndex,
    nodes: Vec<Node>,
    edges: Vec<Edge>,
}

impl<'a> TomlWalker<'a> {
    fn walk_root(&mut self, value: &Value) {
        if let Value::Table(table) = value {
            for (key, child) in table {
                let path = key.to_owned();
                self.walk_value(self.ctx.rel_path, "", key, &path, child, 1);
            }
        }
    }

    fn walk_value(
        &mut self,
        parent_qn: &str,
        parent_path: &str,
        name: &str,
        path: &str,
        value: &Value,
        fallback_line: u32,
    ) {
        let (kind, qn_prefix, value_kind) = match value {
            Value::Table(_) => (NodeKind::Module, "table", "table"),
            Value::Array(_) => (NodeKind::Variable, "key", "array"),
            Value::String(_) => (NodeKind::Variable, "key", "string"),
            Value::Integer(_) | Value::Float(_) => (NodeKind::Variable, "key", "number"),
            Value::Boolean(_) => (NodeKind::Variable, "key", "boolean"),
            Value::Datetime(_) => (NodeKind::Variable, "key", "datetime"),
        };
        let qn = format!("{}::{}::{}", self.ctx.rel_path, qn_prefix, path);
        let line = self
            .index
            .table_line(path)
            .or_else(|| self.index.key_line(path))
            .unwrap_or(fallback_line);
        self.nodes.push(Node {
            id: NodeId::UNSET,
            kind,
            name: name.to_owned(),
            qualified_name: qn.clone(),
            file_path: self.ctx.rel_path.to_owned(),
            line_start: line,
            line_end: line,
            language: "toml".to_owned(),
            parent_name: Some(parent_qn.to_owned()),
            params: None,
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: self.ctx.file_hash.to_owned(),
            extra_json: serde_json::json!({
                "path": path,
                "value_kind": value_kind,
                "parent_path": parent_path,
            }),
        });
        self.edges
            .push(contains_edge(parent_qn, &qn, self.ctx.rel_path, line));

        match value {
            Value::Table(table) => {
                for (child_name, child_value) in table {
                    let child_path = if path.is_empty() {
                        child_name.to_owned()
                    } else {
                        format!("{path}.{child_name}")
                    };
                    self.walk_value(&qn, path, child_name, &child_path, child_value, line);
                }
            }
            Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    let item_path = format!("{path}[{index}]");
                    let item_qn = format!("{}::key::{}", self.ctx.rel_path, item_path);
                    self.nodes.push(Node {
                        id: NodeId::UNSET,
                        kind: match item {
                            Value::Table(_) => NodeKind::Module,
                            _ => NodeKind::Variable,
                        },
                        name: format!("[{index}]"),
                        qualified_name: item_qn.clone(),
                        file_path: self.ctx.rel_path.to_owned(),
                        line_start: line,
                        line_end: line,
                        language: "toml".to_owned(),
                        parent_name: Some(qn.clone()),
                        params: None,
                        return_type: None,
                        modifiers: None,
                        is_test: false,
                        file_hash: self.ctx.file_hash.to_owned(),
                        extra_json: serde_json::json!({
                            "path": item_path,
                            "value_kind": "array_item",
                            "parent_path": path,
                        }),
                    });
                    self.edges
                        .push(contains_edge(&qn, &item_qn, self.ctx.rel_path, line));
                    if let Value::Table(table) = item {
                        for (child_name, child_value) in table {
                            let child_path = format!("{path}[{index}].{child_name}");
                            self.walk_value(
                                &item_qn,
                                path,
                                child_name,
                                &child_path,
                                child_value,
                                line,
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

fn strip_toml_comment(line: &str) -> String {
    let mut out = String::new();
    let mut in_string = false;
    let mut escaped = false;
    for ch in line.chars() {
        if ch == '"' && !escaped {
            in_string = !in_string;
        }
        if ch == '#' && !in_string {
            break;
        }
        escaped = ch == '\\' && !escaped;
        if ch != '\\' {
            escaped = false;
        }
        out.push(ch);
    }
    out
}

fn split_toml_key_value(line: &str) -> Option<(&str, &str)> {
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if ch == '"' && !escaped {
            in_string = !in_string;
        }
        if ch == '=' && !in_string {
            let (key, value) = line.split_at(index);
            return Some((key.trim(), value[1..].trim()));
        }
        escaped = ch == '\\' && !escaped;
        if ch != '\\' {
            escaped = false;
        }
    }
    None
}

fn normalize_toml_path(path: &str) -> String {
    split_dotted_path(path).join(".")
}

fn split_dotted_path(path: &str) -> Vec<String> {
    path.split('.')
        .map(str::trim)
        .map(|segment| segment.trim_matches('"').trim_matches('\''))
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> ParsedFile {
        let (pf, _) = TomlParser.parse(&ParseContext {
            rel_path: "Cargo.toml",
            file_hash: "deadbeef",
            source: src.as_bytes(),
            old_tree: None,
        });
        pf
    }

    #[test]
    fn extracts_tables_keys_and_arrays() {
        let pf = parse(
            r#"[package]
name = "atlas"
keywords = ["rust", "graph"]

[tool.meta]
enabled = true
"#,
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "Cargo.toml::table::package")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "Cargo.toml::key::package.name")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "Cargo.toml::key::package.keywords[0]")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.qualified_name == "Cargo.toml::table::tool.meta")
        );
    }

    #[test]
    fn malformed_source_keeps_file_node_only() {
        let pf = parse("[package\nname = \"atlas\"\n");
        assert_eq!(pf.nodes.len(), 1);
        assert_eq!(pf.nodes[0].kind, NodeKind::File);
    }
}
