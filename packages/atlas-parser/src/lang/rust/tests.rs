use crate::traits::LangParser;
use atlas_core::{EdgeKind, NodeKind, ParsedFile};

use super::*;
use crate::query_helpers::{compile_query, read_capture_text, run_query};
use crate::traits::ParseContext;

fn parse(src: &str) -> ParsedFile {
    let p = RustParser;
    let (pf, _) = p.parse(&ParseContext {
        rel_path: "src/lib.rs",
        file_hash: "deadbeef",
        source: src.as_bytes(),
        old_tree: None,
    });
    pf
}

#[test]
fn extracts_file_node() {
    let pf = parse("fn foo() {}");
    assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::File));
}

#[test]
fn extracts_free_function() {
    let pf = parse("pub fn greet(name: &str) -> String { todo!() }");
    let func = pf
        .nodes
        .iter()
        .find(|n| n.kind == NodeKind::Function)
        .expect("function");
    assert_eq!(func.name, "greet");
    assert!(func.qualified_name.contains("fn::greet"));
}

#[test]
fn extracts_struct() {
    let pf = parse("pub struct Foo { x: i32 }");
    let s = pf
        .nodes
        .iter()
        .find(|n| n.kind == NodeKind::Struct)
        .expect("struct");
    assert_eq!(s.name, "Foo");
}

#[test]
fn extracts_enum() {
    let pf = parse("enum Color { Red, Green, Blue }");
    assert!(
        pf.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Enum && n.name == "Color")
    );
}

#[test]
fn extracts_trait() {
    let pf = parse("pub trait Drawable { fn draw(&self); }");
    assert!(
        pf.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Trait && n.name == "Drawable")
    );
}

#[test]
fn trait_method_declaration_emitted_and_contained_by_trait() {
    let pf = parse("pub trait Drawable { fn draw(&self); }");
    assert!(pf.nodes.iter().any(|n| {
        n.kind == NodeKind::Method
            && n.qualified_name == "src/lib.rs::method::Drawable::draw"
            && n.parent_name.as_deref() == Some("src/lib.rs::trait::Drawable")
    }));
    assert!(pf.edges.iter().any(|e| {
        e.kind == EdgeKind::Contains
            && e.source_qn == "src/lib.rs::trait::Drawable"
            && e.target_qn == "src/lib.rs::method::Drawable::draw"
    }));
}

#[test]
fn free_function_and_trait_method_with_same_name_stay_distinct() {
    let pf = parse("fn draw() {} trait Drawable { fn draw(&self); }");
    assert!(
        pf.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Function && n.qualified_name == "src/lib.rs::fn::draw")
    );
    assert!(pf.nodes.iter().any(|n| {
        n.kind == NodeKind::Method && n.qualified_name == "src/lib.rs::method::Drawable::draw"
    }));
}

#[test]
fn extracts_method_and_impl_edge() {
    let src = "struct Foo; impl Foo { pub fn bar(&self) {} }";
    let pf = parse(src);
    assert!(
        pf.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Module && n.qualified_name == "src/lib.rs::impl::Foo")
    );
    assert!(
        pf.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Method && n.name == "bar")
    );
    assert!(pf.edges.iter().any(|e| e.kind == EdgeKind::Contains
        && e.source_qn == "src/lib.rs::impl::Foo"
        && e.target_qn == "src/lib.rs::method::Foo::bar"));
}

#[test]
fn implements_edge_for_trait_impl() {
    let src = "trait Greet {} struct Hi; impl Greet for Hi {}";
    let pf = parse(src);
    assert!(pf.edges.iter().any(|e| e.kind == EdgeKind::Implements
        && e.source_qn == "src/lib.rs::struct::Hi"
        && e.target_qn == "src/lib.rs::trait::Greet"));
}

#[test]
fn implements_edge_uses_enum_qn_when_impl_targets_enum() {
    let src = "trait Render {} enum Mode { Fast } impl Render for Mode {}";
    let pf = parse(src);
    assert!(pf.edges.iter().any(|e| e.kind == EdgeKind::Implements
        && e.source_qn == "src/lib.rs::enum::Mode"
        && e.target_qn == "src/lib.rs::trait::Render"));
}

#[test]
fn external_trait_impl_does_not_emit_dangling_implements_edge() {
    let src = "struct Foo; impl std::fmt::Display for Foo {}";
    let pf = parse(src);
    assert!(!pf.edges.iter().any(|e| e.kind == EdgeKind::Implements));
}

#[test]
fn scoped_local_trait_impl_emits_same_file_edge_when_targets_are_unique() {
    let src = r#"
mod local {
pub trait Trait {}
pub struct Type;
}

impl local::Trait for local::Type {}
"#;
    let pf = parse(src);
    assert!(pf.edges.iter().any(|e| {
        e.kind == EdgeKind::Implements
            && e.source_qn == "src/lib.rs::struct::local::Type"
            && e.target_qn == "src/lib.rs::trait::local::Trait"
    }));
}

#[test]
fn test_fn_detected() {
    let src = r#"
#[cfg(test)]
mod tests {
#[test]
fn it_works() {}
}
"#;
    let pf = parse(src);
    assert!(
        pf.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Test && n.name == "it_works")
    );
}

#[test]
fn top_level_test_attr_emits_test_node_kind() {
    let pf = parse("#[test] fn it_works() {}");
    assert!(
        pf.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Test && n.name == "it_works")
    );
}

#[test]
fn cfg_test_module_marks_nested_helper_as_test() {
    let pf = parse("#[cfg(test)] mod tests { fn helper() {} }");
    assert!(
        pf.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Test && n.name == "helper")
    );
}

#[test]
fn cfg_not_test_module_does_not_mark_nested_helper_as_test() {
    let pf = parse("#[cfg(not(test))] mod tests { fn helper() {} }");
    assert!(
        pf.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Function && n.name == "helper")
    );
    assert!(
        !pf.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Test && n.name == "helper")
    );
}

#[test]
fn custom_attribute_containing_test_does_not_mark_function_as_test() {
    let pf = parse("#[mytest] fn helper() {}");
    assert!(
        pf.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Function && n.name == "helper")
    );
    assert!(
        !pf.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Test && n.name == "helper")
    );
}

#[test]
fn nested_module() {
    let src = "mod outer { mod inner { fn deep() {} } }";
    let pf = parse(src);
    assert!(
        pf.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Module && n.name == "outer")
    );
    assert!(
        pf.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Module && n.name == "inner")
    );
    assert!(
        pf.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Function && n.name == "deep")
    );
}

#[test]
fn contains_edges_present() {
    let src = "mod foo { fn bar() {} }";
    let pf = parse(src);
    assert!(pf.edges.iter().any(|e| e.kind == EdgeKind::Contains));
}

#[test]
fn same_file_call_resolved() {
    let src = r#"
fn helper() {}
fn caller() { helper(); }
"#;
    let pf = parse(src);
    assert!(
        pf.edges.iter().any(|e| e.kind == EdgeKind::Calls
            && e.source_qn.contains("caller")
            && e.target_qn.contains("helper")),
        "expected a Calls edge from caller to helper; edges: {:?}",
        pf.edges
    );
}

#[test]
fn generic_function_call_resolved() {
    let src = r#"
fn helper<T>(value: T) -> T { value }
fn caller() { let _ = helper::<u32>(1); }
"#;
    let pf = parse(src);
    assert!(
        pf.edges.iter().any(|e| e.kind == EdgeKind::Calls
            && e.source_qn.contains("caller")
            && e.target_qn.contains("helper")),
        "expected a Calls edge from caller to generic helper; edges: {:?}",
        pf.edges
    );
}

#[test]
fn method_call_resolved() {
    let src = r#"
fn helper() {}
struct S;
impl S {
fn do_work(&self) { helper(); }
}
"#;
    let pf = parse(src);
    assert!(
        pf.edges
            .iter()
            .any(|e| e.kind == EdgeKind::Calls && e.target_qn.contains("helper")),
        "expected Calls edge to helper from method"
    );
}

#[test]
fn method_call_syntax_resolved_to_same_file_method() {
    let src = r#"
struct Worker;

impl Worker {
fn run(&self) {}

fn execute(&self) {
    self.run();
}
}
"#;
    let pf = parse(src);
    assert!(pf.edges.iter().any(|e| {
        e.kind == EdgeKind::Calls
            && e.source_qn == "src/lib.rs::method::Worker::execute"
            && e.target_qn == "src/lib.rs::method::Worker::run"
    }));
}

#[test]
fn no_self_calls_edge() {
    // A recursive call should not produce a self-loop.
    let src = r#"fn recurse(n: u32) -> u32 { if n == 0 { 0 } else { recurse(n-1) } }"#;
    let pf = parse(src);
    assert!(
        !pf.edges
            .iter()
            .any(|e| e.kind == EdgeKind::Calls && e.source_qn == e.target_qn),
        "recursive call must not produce a self-loop edge"
    );
}

#[test]
fn unresolved_call_keeps_text_target() {
    let src = r#"fn caller() { crate::helper(); }"#;
    let pf = parse(src);
    let edge = pf
        .edges
        .iter()
        .find(|e| e.kind == EdgeKind::Calls)
        .expect("call edge");
    assert_eq!(edge.target_qn, "crate::helper");
    assert_eq!(edge.confidence_tier.as_deref(), Some("text"));
}

#[test]
fn skips_variant_and_option_constructor_false_positives() {
    let src = r#"
enum Value { Object }

fn helper() {}

fn caller() {
Value::Object();
Some("x");
helper();
}
"#;
    let pf = parse(src);
    assert!(pf.edges.iter().any(|e| {
        e.kind == EdgeKind::Calls
            && e.source_qn.contains("caller")
            && e.target_qn.contains("helper")
    }));
    assert!(
        !pf.edges
            .iter()
            .any(|e| { e.kind == EdgeKind::Calls && e.target_qn == "Value::Object" })
    );
    assert!(
        !pf.edges
            .iter()
            .any(|e| e.kind == EdgeKind::Calls && e.target_qn == "Some")
    );
}

#[test]
fn extracts_generic_function() {
    let pf = parse("pub fn wrap<T: Clone>(value: T) -> Option<T> { Some(value) }");
    let func = pf
        .nodes
        .iter()
        .find(|n| n.kind == NodeKind::Function && n.name == "wrap")
        .expect("generic function");
    assert_eq!(func.return_type.as_deref(), Some("Option<T>"));
    assert_eq!(func.params.as_deref(), Some("(value: T)"));
}

#[test]
fn resolves_same_file_use_and_type_references() {
    let src = r#"
mod support {
pub struct Helper;
}

use self::support::Helper;

fn build(value: Helper) -> Helper { value }
"#;
    let pf = parse(src);
    assert!(
        pf.edges.iter().any(|e| e.kind == EdgeKind::References
            && e.source_qn == "src/lib.rs"
            && e.target_qn.contains("module::support")),
        "expected use reference to module support; edges: {:?}",
        pf.edges
    );
    assert!(
        pf.edges.iter().any(|e| e.kind == EdgeKind::References
            && e.source_qn.contains("build")
            && e.target_qn.contains("struct::support::Helper")),
        "expected function type reference to Helper; edges: {:?}",
        pf.edges
    );
}

#[test]
fn macro_heavy_file_parses() {
    let src = r#"
macro_rules! call_helper {
($value:expr) => {
    helper($value)
};
}

#[derive(Debug, Clone)]
struct Wrapper<T> {
value: T,
}

fn helper<T>(value: T) -> T { value }

fn caller() {
let _ = call_helper!(Wrapper { value: 1 });
}
"#;
    let pf = parse(src);
    assert!(
        pf.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Struct && n.name == "Wrapper")
    );
    assert!(
        pf.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Function && n.name == "helper")
    );
    assert!(
        pf.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Function && n.name == "caller")
    );
}

#[test]
fn nested_impl_scope_tracks_parent_module() {
    let src = r#"
mod outer {
struct Thing;
impl Thing {
    fn run(&self) {}
}
}
"#;
    let pf = parse(src);
    assert!(pf.nodes.iter().any(|n| {
        n.kind == NodeKind::Module && n.qualified_name == "src/lib.rs::impl::outer::Thing"
    }));
    assert!(pf.nodes.iter().any(|n| {
        n.kind == NodeKind::Method
            && n.qualified_name == "src/lib.rs::method::outer::Thing::run"
            && n.parent_name.as_deref() == Some("src/lib.rs::impl::outer::Thing")
    }));
}

#[test]
fn resolves_calls_to_closest_parent_scope() {
    let src = r#"
fn helper() {}

mod alpha {
fn helper() {}

fn caller() {
    helper();
}
}
"#;
    let pf = parse(src);
    assert!(pf.edges.iter().any(|e| {
        e.kind == EdgeKind::Calls
            && e.source_qn == "src/lib.rs::fn::alpha::caller"
            && e.target_qn == "src/lib.rs::fn::alpha::helper"
    }));
    assert!(!pf.edges.iter().any(|e| {
        e.kind == EdgeKind::Calls
            && e.source_qn == "src/lib.rs::fn::alpha::caller"
            && e.target_qn == "src/lib.rs::fn::helper"
    }));
}

#[test]
fn malformed_source_keeps_file_node_and_best_effort_symbols() {
    let pf = parse("pub fn broken(value: i32) -> i32 { value + 1 } @");
    assert!(pf.nodes.iter().any(|node| node.kind == NodeKind::File));
    assert!(
        pf.nodes
            .iter()
            .any(|node| node.kind == NodeKind::Function && node.name == "broken")
    );
}

#[test]
fn rust_definition_query_extracts_function_capture() {
    let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    let query = compile_query(language.clone(), RUST_DEFINITION_QUERY)
        .expect("rust definition query should compile");
    let source = b"fn helper() {}";

    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&language)
        .expect("tree-sitter-rust grammar failed to load");
    let tree = parser
        .parse(source.as_slice(), None)
        .expect("fixture should parse");

    let matches = run_query(&query, tree.root_node(), source);
    assert!(matches.iter().any(|group| {
        group.captures.iter().any(|capture| {
            capture.name == "atlas.definition.function"
                && read_capture_text(capture, source).contains("fn helper")
        })
    }));
}

#[test]
fn rust_definition_query_extracts_impl_trait_capture() {
    let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    let query = compile_query(language.clone(), RUST_DEFINITION_QUERY)
        .expect("rust definition query should compile");
    let source = b"trait Draw {} struct Shape; impl Draw for Shape {}";

    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&language)
        .expect("tree-sitter-rust grammar failed to load");
    let tree = parser
        .parse(source.as_slice(), None)
        .expect("fixture should parse");

    let matches = run_query(&query, tree.root_node(), source);
    assert!(matches.iter().any(|group| {
        group.captures.iter().any(|capture| {
            capture.name == "atlas.impl.trait" && read_capture_text(capture, source) == "Draw"
        })
    }));
}

#[test]
fn rust_definition_query_extracts_method_call_receiver_and_name() {
    let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    let query = compile_query(language.clone(), RUST_DEFINITION_QUERY)
        .expect("rust definition query should compile");
    let source = b"struct S; impl S { fn run(&self) {} fn call(&self) { self.run(); } }";

    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&language)
        .expect("tree-sitter-rust grammar failed to load");
    let tree = parser
        .parse(source.as_slice(), None)
        .expect("fixture should parse");

    let matches = run_query(&query, tree.root_node(), source);
    assert!(matches.iter().any(|group| {
        let names = group
            .captures
            .iter()
            .map(|capture| capture.name.as_str())
            .collect::<Vec<_>>();
        names.contains(&"atlas.call.receiver")
            && names.contains(&"atlas.call.method")
            && group.captures.iter().any(|capture| {
                capture.name == "atlas.call.method" && read_capture_text(capture, source) == "run"
            })
    }));
}
