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
