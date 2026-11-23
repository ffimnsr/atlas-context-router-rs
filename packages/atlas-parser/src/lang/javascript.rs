use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile};
use std::collections::{HashMap, HashSet};
use tree_sitter::{Language, Node as TsNode};

use crate::ast_helpers::{end_line, file_node, node_text, start_line};
use crate::lang::common::{
    call_edge, call_target, callable_qn_map, caller_simple_name, contains_edge,
};
use crate::query_helpers::{compile_static_query, run_query};
use crate::traits::{LangParser, ParseContext};

// ---------------------------------------------------------------------------
// Public parser types
// ---------------------------------------------------------------------------

// SQ1 migration checklist:
// - manual extraction below owns declaration walks, scope and qualified-name rules, import/reference detection,
//   call-edge heuristics, and source metadata attachment
// - keep public parser API, graph schema, and output contracts unchanged during query migration
// - move tree-sitter syntax matching only into shared @atlas.* query captures

/// JavaScript / JSX parser (`.js`, `.jsx`, `.mjs`, `.cjs`).
pub struct JsParser;

/// TypeScript / TSX parser (`.ts`, `.tsx`).
pub struct TsParser;

const JAVASCRIPT_QUERY: &str = include_str!("../../queries/javascript.scm");
const TYPESCRIPT_QUERY: &str = include_str!("../../queries/typescript.scm");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct JsTsNodeKey {
    start_byte: usize,
    end_byte: usize,
}

#[derive(Clone)]
struct JsTsQueryConfig {
    language_name: &'static str,
    language: Language,
    query_text: &'static str,
}

#[derive(Debug, Default)]
struct JsTsSyntaxFacts {
    exports: HashSet<JsTsNodeKey>,
    imports: HashSet<JsTsNodeKey>,
    functions: HashSet<JsTsNodeKey>,
    classes: HashSet<JsTsNodeKey>,
    methods: HashSet<JsTsNodeKey>,
    variables: HashSet<JsTsNodeKey>,
    interfaces: HashSet<JsTsNodeKey>,
    type_aliases: HashSet<JsTsNodeKey>,
    enums: HashSet<JsTsNodeKey>,
    calls: HashSet<JsTsNodeKey>,
}

mod helpers;
mod parser;
mod visit;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::ParseContext;

    fn parse_js(src: &str) -> ParsedFile {
        let (pf, _) = JsParser.parse(&ParseContext {
            rel_path: "src/app.js",
            file_hash: "cafebabe",
            source: src.as_bytes(),
            old_tree: None,
        });
        pf
    }

    fn parse_ts(src: &str) -> ParsedFile {
        let (pf, _) = TsParser.parse(&ParseContext {
            rel_path: "src/app.ts",
            file_hash: "cafebabe",
            source: src.as_bytes(),
            old_tree: None,
        });
        pf
    }

    fn parse_tsx(src: &str) -> ParsedFile {
        let (pf, _) = TsParser.parse(&ParseContext {
            rel_path: "src/app.tsx",
            file_hash: "cafebabe",
            source: src.as_bytes(),
            old_tree: None,
        });
        pf
    }

    #[test]
    fn js_file_node_present() {
        let pf = parse_js("const x = 1;\n");
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::File));
        assert_eq!(pf.language.as_deref(), Some("javascript"));
    }

    #[test]
    fn js_extracts_function() {
        let pf = parse_js("function greet(name) { return name; }\n");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Function && n.name == "greet")
        );
    }

    #[test]
    fn js_extracts_class() {
        let pf = parse_js("class Greeter { }\n");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Class && n.name == "Greeter")
        );
    }

    #[test]
    fn js_extracts_method() {
        let pf = parse_js("class Greeter { greet(name) { return name; } }\n");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Method && n.name == "greet")
        );
    }

    #[test]
    fn js_export_function() {
        let pf = parse_js("export function add(a, b) { return a + b; }\n");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Function && n.name == "add")
        );
    }

    #[test]
    fn js_extracts_named_arrow_function_variable() {
        let pf = parse_js("const greet = (name) => name;\n");
        assert!(pf.nodes.iter().any(|n| {
            n.kind == NodeKind::Function
                && n.name == "greet"
                && n.qualified_name == "src/app.js::fn::greet"
        }));
    }

    #[test]
    fn js_import_edges() {
        let pf = parse_js("import { foo } from './utils';\n");
        assert!(pf.edges.iter().any(|e| e.kind == EdgeKind::Imports));
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::Import));
    }

    #[test]
    fn ts_file_node_present() {
        let pf = parse_ts("const x: number = 1;\n");
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::File));
        assert_eq!(pf.language.as_deref(), Some("typescript"));
    }

    #[test]
    fn ts_extracts_interface() {
        let pf = parse_ts("interface Greeter { greet(name: string): void; }\n");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Interface && n.name == "Greeter"),
            "interface node not found"
        );
    }

    #[test]
    fn ts_extracts_enum() {
        let pf = parse_ts("enum Color { Red, Green, Blue }\n");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Enum && n.name == "Color")
        );
    }

    #[test]
    fn ts_type_alias() {
        let pf = parse_ts("type UserId = string;\n");
        assert!(pf.nodes.iter().any(|n| n.name == "UserId"));
    }

    #[test]
    fn ts_class_with_methods() {
        let pf = parse_ts(
            "class Service {\n  constructor(private db: DB) {}\n  fetch(id: string): Promise<Data> { return this.db.get(id); }\n}\n",
        );
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Class && n.name == "Service")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Method && n.name == "fetch")
        );
    }

    #[test]
    fn ts_export_class() {
        let pf = parse_ts("export class Router { route() {} }\n");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Class && n.name == "Router")
        );
    }

    #[test]
    fn d_ts_not_supported() {
        assert!(!TsParser.supports("index.d.ts"));
    }

    #[test]
    fn js_same_file_call_resolved() {
        let src = "function helper() {}\nfunction caller() { helper(); }\n";
        let pf = parse_js(src);
        assert!(
            pf.edges.iter().any(|e| e.kind == EdgeKind::Calls
                && e.source_qn.contains("caller")
                && e.target_qn.contains("helper")),
            "expected Calls edge from caller to helper; edges: {:?}",
            pf.edges
        );
    }

    #[test]
    fn ts_same_file_call_resolved() {
        let src = "function helper(): void {}\nfunction caller(): void { helper(); }\n";
        let pf = parse_ts(src);
        assert!(
            pf.edges.iter().any(|e| e.kind == EdgeKind::Calls
                && e.source_qn.contains("caller")
                && e.target_qn.contains("helper")),
            "expected Calls edge from caller to helper in TS"
        );
    }

    #[test]
    fn js_unresolved_call_keeps_text_target() {
        let src = "function caller() { utils.helper(); }\n";
        let pf = parse_js(src);
        let edge = pf
            .edges
            .iter()
            .find(|e| e.kind == EdgeKind::Calls)
            .expect("call edge");
        assert_eq!(edge.target_qn, "utils.helper");
        assert_eq!(edge.confidence_tier.as_deref(), Some("text"));
    }

    #[test]
    fn tsx_const_function_component_is_callable_scope() {
        let src = "const App = () => <Widget />;\n";
        let pf = parse_tsx(src);
        assert!(pf.nodes.iter().any(|n| {
            n.kind == NodeKind::Function
                && n.name == "App"
                && n.qualified_name == "src/app.tsx::fn::App"
        }));
        assert!(
            pf.edges.iter().any(|e| {
                e.kind == EdgeKind::Calls
                    && e.source_qn == "src/app.tsx::fn::App"
                    && e.target_qn == "Widget"
                    && e.confidence_tier.as_deref() == Some("text")
            }),
            "expected JSX component call from App to Widget; edges: {:?}",
            pf.edges
        );
    }

    #[test]
    fn tsx_memo_named_function_component_calls_imported_jsx_component() {
        let src = "export const HistoryTab = memo(function HistoryTab() { return <SideFilterControl />; });\n";
        let pf = parse_tsx(src);
        assert!(
            pf.nodes.iter().any(|n| {
                n.kind == NodeKind::Function
                    && n.name == "HistoryTab"
                    && n.qualified_name == "src/app.tsx::fn::HistoryTab"
            }),
            "expected HistoryTab function node; nodes: {:?}",
            pf.nodes
        );
        assert!(
            pf.edges.iter().any(|e| {
                e.kind == EdgeKind::Calls
                    && e.source_qn == "src/app.tsx::fn::HistoryTab"
                    && e.target_qn == "SideFilterControl"
                    && e.confidence_tier.as_deref() == Some("text")
            }),
            "expected JSX component call from HistoryTab to SideFilterControl; edges: {:?}",
            pf.edges
        );
    }

    #[test]
    fn tsx_ignores_lowercase_jsx_elements() {
        let src = "const App = () => <div><Widget /></div>;\n";
        let pf = parse_tsx(src);
        assert!(
            !pf.edges.iter().any(|e| {
                e.kind == EdgeKind::Calls
                    && e.target_qn == "div"
                    && e.confidence_tier.as_deref() == Some("text")
            }),
            "lowercase intrinsic JSX element should not produce call edge: {:?}",
            pf.edges
        );
    }

    #[test]
    fn no_empty_target_qn_in_call_edges() {
        // A call_expression whose function field produces empty text must not
        // emit a call edge with empty target_qn.
        let src = "function f() { (); }\n";
        let pf = parse_js(src);
        assert!(
            pf.edges.iter().all(|e| !e.target_qn.is_empty()),
            "call edge with empty target_qn found: {:?}",
            pf.edges
        );
    }
}
