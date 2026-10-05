use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    File,
    Package,
    Module,
    Import,
    Class,
    Interface,
    Struct,
    Enum,
    Function,
    Method,
    Variable,
    Constant,
    Trait,
    Test,
}

impl NodeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NodeKind::File => "file",
            NodeKind::Package => "package",
            NodeKind::Module => "module",
            NodeKind::Import => "import",
            NodeKind::Class => "class",
            NodeKind::Interface => "interface",
            NodeKind::Struct => "struct",
            NodeKind::Enum => "enum",
            NodeKind::Function => "function",
            NodeKind::Method => "method",
            NodeKind::Variable => "variable",
            NodeKind::Constant => "constant",
            NodeKind::Trait => "trait",
            NodeKind::Test => "test",
        }
    }
}

/// Canonicalize a user-supplied node kind or alias to its graph kind string.
///
/// Accepts canonical `NodeKind::as_str` values plus common aliases (`fn`,
/// `func`, `record`, `iface`, `mod`, `var`, `field`, `const`, `use`, `pkg`).
/// Unknown input is lowercased and returned unchanged so callers can keep
/// treating it as a filter that simply matches nothing. CLI and MCP kind
/// filters share this table through this single function.
pub fn normalize_kind_alias(input: &str) -> String {
    match input.to_ascii_lowercase().as_str() {
        "fn" | "func" | "function" => "function",
        "method" | "meth" => "method",
        "class" => "class",
        "struct" | "record" => "struct",
        "interface" | "iface" => "interface",
        "trait" => "trait",
        "enum" => "enum",
        "module" | "mod" => "module",
        "variable" | "var" | "field" => "variable",
        "constant" | "const" => "constant",
        "test" => "test",
        "import" | "use" => "import",
        "package" | "pkg" => "package",
        "file" => "file",
        other => other,
    }
    .to_owned()
}

/// Parse a user-supplied kind or alias into a [`NodeKind`].
///
/// Accepts canonical names and aliases (`fn`, `record`, ...) in any case.
/// Unknown input returns `None` so callers can warn or skip explicitly.
pub fn parse_kind_alias(input: &str) -> Option<NodeKind> {
    normalize_kind_alias(input).parse().ok()
}

impl std::fmt::Display for NodeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for NodeKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "file" => Ok(NodeKind::File),
            "package" => Ok(NodeKind::Package),
            "module" => Ok(NodeKind::Module),
            "import" => Ok(NodeKind::Import),
            "class" => Ok(NodeKind::Class),
            "interface" => Ok(NodeKind::Interface),
            "struct" => Ok(NodeKind::Struct),
            "enum" => Ok(NodeKind::Enum),
            "function" => Ok(NodeKind::Function),
            "method" => Ok(NodeKind::Method),
            "variable" => Ok(NodeKind::Variable),
            "constant" => Ok(NodeKind::Constant),
            "trait" => Ok(NodeKind::Trait),
            "test" => Ok(NodeKind::Test),
            other => Err(format!("unknown node kind: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    Contains,
    Imports,
    Calls,
    Defines,
    Implements,
    Extends,
    Tests,
    References,
    TestedBy,
}

impl EdgeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeKind::Contains => "contains",
            EdgeKind::Imports => "imports",
            EdgeKind::Calls => "calls",
            EdgeKind::Defines => "defines",
            EdgeKind::Implements => "implements",
            EdgeKind::Extends => "extends",
            EdgeKind::Tests => "tests",
            EdgeKind::References => "references",
            EdgeKind::TestedBy => "tested_by",
        }
    }

    /// Traversal weight used for weighted impact scoring.
    ///
    /// Higher weight = stronger dependency, contributes more to impact score.
    pub fn traversal_weight(self) -> f64 {
        match self {
            EdgeKind::Calls => 3.0,
            EdgeKind::Implements | EdgeKind::Extends => 2.5,
            EdgeKind::Imports => 2.0,
            EdgeKind::Defines | EdgeKind::Contains => 1.5,
            EdgeKind::Tests | EdgeKind::TestedBy => 1.5,
            EdgeKind::References => 1.0,
        }
    }
}

impl std::fmt::Display for EdgeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for EdgeKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "contains" => Ok(EdgeKind::Contains),
            "imports" => Ok(EdgeKind::Imports),
            "calls" => Ok(EdgeKind::Calls),
            "defines" => Ok(EdgeKind::Defines),
            "implements" => Ok(EdgeKind::Implements),
            "extends" => Ok(EdgeKind::Extends),
            "tests" => Ok(EdgeKind::Tests),
            "references" => Ok(EdgeKind::References),
            "tested_by" => Ok(EdgeKind::TestedBy),
            other => Err(format!("unknown edge kind: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    // -------------------------------------------------------------------------
    // NodeKind serialization / parsing (used in qualified-name scheme)
    // -------------------------------------------------------------------------

    #[test]
    fn node_kind_as_str_matches_serde_name() {
        let cases = [
            (NodeKind::File, "file"),
            (NodeKind::Package, "package"),
            (NodeKind::Module, "module"),
            (NodeKind::Import, "import"),
            (NodeKind::Class, "class"),
            (NodeKind::Interface, "interface"),
            (NodeKind::Struct, "struct"),
            (NodeKind::Enum, "enum"),
            (NodeKind::Function, "function"),
            (NodeKind::Method, "method"),
            (NodeKind::Variable, "variable"),
            (NodeKind::Constant, "constant"),
            (NodeKind::Trait, "trait"),
            (NodeKind::Test, "test"),
        ];
        for (kind, expected) in cases {
            assert_eq!(kind.as_str(), expected, "NodeKind::{kind:?} as_str");
            assert_eq!(kind.to_string(), expected, "NodeKind::{kind:?} Display");
            // round-trip via FromStr
            let parsed = NodeKind::from_str(expected).unwrap();
            assert_eq!(parsed, kind, "NodeKind::from_str({expected})");
        }
    }

    #[test]
    fn node_kind_unknown_string_errors() {
        assert!(NodeKind::from_str("unknown_kind").is_err());
        assert!(NodeKind::from_str("").is_err());
    }

    #[test]
    fn normalize_kind_alias_canonicalizes_aliases_and_lowercases_unknown() {
        let cases = [
            ("fn", "function"),
            ("FUNC", "function"),
            ("function", "function"),
            ("record", "struct"),
            ("iface", "interface"),
            ("mod", "module"),
            ("var", "variable"),
            ("field", "variable"),
            ("const", "constant"),
            ("use", "import"),
            ("pkg", "package"),
            ("test", "test"),
            ("Function", "function"),
            ("Mystery", "mystery"),
        ];
        for (input, expected) in cases {
            assert_eq!(
                normalize_kind_alias(input),
                expected,
                "normalize_kind_alias({input})"
            );
        }
        // Every canonical kind resolves to itself through the alias table.
        for (kind, canonical) in [
            (NodeKind::File, "file"),
            (NodeKind::Package, "package"),
            (NodeKind::Module, "module"),
            (NodeKind::Import, "import"),
            (NodeKind::Class, "class"),
            (NodeKind::Interface, "interface"),
            (NodeKind::Struct, "struct"),
            (NodeKind::Enum, "enum"),
            (NodeKind::Function, "function"),
            (NodeKind::Method, "method"),
            (NodeKind::Variable, "variable"),
            (NodeKind::Constant, "constant"),
            (NodeKind::Trait, "trait"),
            (NodeKind::Test, "test"),
        ] {
            assert_eq!(normalize_kind_alias(canonical), canonical);
            assert_eq!(normalize_kind_alias(kind.as_str()), canonical);
        }
    }

    #[test]
    fn parse_kind_alias_accepts_aliases_and_rejects_unknown() {
        assert_eq!(parse_kind_alias("fn"), Some(NodeKind::Function));
        assert_eq!(parse_kind_alias("FUNC"), Some(NodeKind::Function));
        assert_eq!(parse_kind_alias("function"), Some(NodeKind::Function));
        assert_eq!(parse_kind_alias("record"), Some(NodeKind::Struct));
        assert_eq!(parse_kind_alias("iface"), Some(NodeKind::Interface));
        assert_eq!(parse_kind_alias("mod"), Some(NodeKind::Module));
        assert_eq!(parse_kind_alias("mystery"), None);
        assert_eq!(parse_kind_alias(""), None);
    }

    #[test]
    fn node_kind_serde_snake_case() {
        let kind = NodeKind::Function;
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, "\"function\"");
        let back: NodeKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, kind);
    }

    // -------------------------------------------------------------------------
    // EdgeKind serialization / parsing
    // -------------------------------------------------------------------------

    #[test]
    fn edge_kind_as_str_matches_serde_name() {
        let cases = [
            (EdgeKind::Contains, "contains"),
            (EdgeKind::Imports, "imports"),
            (EdgeKind::Calls, "calls"),
            (EdgeKind::Defines, "defines"),
            (EdgeKind::Implements, "implements"),
            (EdgeKind::Extends, "extends"),
            (EdgeKind::Tests, "tests"),
            (EdgeKind::References, "references"),
            (EdgeKind::TestedBy, "tested_by"),
        ];
        for (kind, expected) in cases {
            assert_eq!(kind.as_str(), expected, "EdgeKind::{kind:?} as_str");
            assert_eq!(kind.to_string(), expected, "EdgeKind::{kind:?} Display");
            let parsed = EdgeKind::from_str(expected).unwrap();
            assert_eq!(parsed, kind, "EdgeKind::from_str({expected})");
        }
    }

    #[test]
    fn edge_kind_unknown_string_errors() {
        assert!(EdgeKind::from_str("unknown_edge").is_err());
        assert!(EdgeKind::from_str("").is_err());
    }

    #[test]
    fn edge_kind_serde_snake_case() {
        let kind = EdgeKind::TestedBy;
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, "\"tested_by\"");
        let back: EdgeKind = serde_json::from_str(&json).unwrap();
        assert_eq!(back, kind);
    }

    #[test]
    fn edge_kind_traversal_weights_ordered() {
        // calls must outweigh references
        assert!(
            EdgeKind::Calls.traversal_weight() > EdgeKind::References.traversal_weight(),
            "Calls weight must exceed References weight"
        );
        // imports must outweigh references
        assert!(
            EdgeKind::Imports.traversal_weight() > EdgeKind::References.traversal_weight(),
            "Imports weight must exceed References weight"
        );
    }
}
