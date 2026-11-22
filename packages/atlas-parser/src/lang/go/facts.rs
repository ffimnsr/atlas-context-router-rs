use std::collections::HashSet;
use tree_sitter::Node as TsNode;

use crate::query_helpers::{compile_static_query, run_query};

const GO_QUERY: &str = include_str!("../../../queries/go.scm");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct GoNodeKey {
    start_byte: usize,
    end_byte: usize,
}

#[derive(Debug, Default)]
pub(super) struct GoSyntaxFacts {
    /// `package_clause` sites.
    pub(super) modules: HashSet<GoNodeKey>,
    /// `function_declaration` sites.
    pub(super) functions: HashSet<GoNodeKey>,
    /// `method_declaration` sites.
    pub(super) methods: HashSet<GoNodeKey>,
    /// `type_declaration` sites.
    pub(super) types: HashSet<GoNodeKey>,
    /// `import_declaration` sites.
    pub(super) imports: HashSet<GoNodeKey>,
    /// `const_declaration` sites.
    pub(super) constants: HashSet<GoNodeKey>,
    /// `var_declaration`, `short_var_declaration`, and `assignment_statement` sites.
    pub(super) variables: HashSet<GoNodeKey>,
    /// `call_expression` sites.
    pub(super) calls: HashSet<GoNodeKey>,
}

impl GoSyntaxFacts {
    pub(super) fn extract(root: TsNode<'_>, _source: &[u8]) -> Result<Self, String> {
        let language: tree_sitter::Language = tree_sitter_go::LANGUAGE.into();
        let query = compile_static_query(language, "go", GO_QUERY)?;
        let matches = run_query(&query, root, _source);
        let mut facts = Self::default();

        for group in matches {
            for capture in group.captures {
                let key = node_key(capture.node);
                match capture.name.as_str() {
                    "atlas.definition.module" => {
                        facts.modules.insert(key);
                    }
                    "atlas.definition.function" => {
                        facts.functions.insert(key);
                    }
                    "atlas.definition.method" => {
                        facts.methods.insert(key);
                    }
                    "atlas.definition.type" => {
                        facts.types.insert(key);
                    }
                    "atlas.import" => {
                        facts.imports.insert(key);
                    }
                    "atlas.definition.const" => {
                        facts.constants.insert(key);
                    }
                    "atlas.definition.variable" => {
                        facts.variables.insert(key);
                    }
                    "atlas.call" => {
                        facts.calls.insert(key);
                    }
                    _ => {}
                }
            }
        }

        Ok(facts)
    }
}

pub(super) fn node_key(node: TsNode<'_>) -> GoNodeKey {
    GoNodeKey {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
    }
}
