use crate::traits::LangParser;
use atlas_core::{EdgeKind, NodeKind, ParsedFile};

use super::*;
use crate::traits::ParseContext;

fn parse(src: &str) -> ParsedFile {
    let p = GoParser;
    let (pf, _) = p.parse(&ParseContext {
        rel_path: "cmd/main.go",
        file_hash: "cafebabe",
        source: src.as_bytes(),
        old_tree: None,
    });
    pf
}

#[test]
fn file_node_present() {
    let pf = parse("package main\n");
    assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::File));
}

#[test]
fn package_node_present() {
    let pf = parse("package widgets\nfunc Hello() {}\n");
    let package = pf
        .nodes
        .iter()
        .find(|n| n.kind == NodeKind::Package)
        .expect("package node");
    assert_eq!(package.name, "widgets");
    assert_eq!(package.qualified_name, "cmd/main.go::package::widgets");
    assert!(pf.edges.iter().any(|e| e.kind == EdgeKind::Contains
        && e.source_qn == "cmd/main.go"
        && e.target_qn == package.qualified_name));
    assert!(
        pf.nodes.iter().any(|n| n.name == "Hello"
            && n.parent_name.as_deref() == Some(package.qualified_name.as_str()))
    );
}

#[test]
fn extracts_function() {
    let pf = parse("package main\nfunc Hello() string { return \"\" }");
    assert!(
        pf.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Function && n.name == "Hello")
    );
}

#[test]
fn extracts_struct() {
    let pf = parse("package main\ntype Foo struct { x int }");
    assert!(
        pf.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Struct && n.name == "Foo")
    );
}

#[test]
fn extracts_interface() {
    let pf = parse("package main\ntype Reader interface { Read(p []byte) (int, error) }");
    assert!(
        pf.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Interface && n.name == "Reader")
    );
}

#[test]
fn extracts_method() {
    let pf = parse("package main\ntype Foo struct{}\nfunc (f *Foo) Bar() {}");
    let method = pf
        .nodes
        .iter()
        .find(|n| n.kind == NodeKind::Method && n.name == "Bar")
        .expect("method node");
    assert_eq!(method.qualified_name, "cmd/main.go::method::Foo.Bar");
    assert_eq!(
        method.parent_name.as_deref(),
        Some("cmd/main.go::package::main")
    );
}

#[test]
fn test_function_detected() {
    let pf = parse("package main\nfunc TestFoo(t *testing.T) {}");
    assert!(
        pf.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Test && n.name == "TestFoo")
    );
}

#[test]
fn import_edges() {
    let pf = parse("package main\nimport \"fmt\"\nfunc main() {}");
    assert!(pf.edges.iter().any(|e| e.kind == EdgeKind::Imports));
}

#[test]
fn same_file_call_resolved() {
    let src = "package main\nfunc helper() {}\nfunc caller() { helper() }";
    let pf = parse(src);
    assert!(
        pf.edges.iter().any(|e| e.kind == EdgeKind::Calls
            && e.source_qn.contains("caller")
            && e.target_qn.contains("helper")),
        "expected Calls edge; edges: {:?}",
        pf.edges
    );
}

#[test]
fn unresolved_call_keeps_text_target() {
    let src = "package main\nfunc caller() { helpers.Run() }";
    let pf = parse(src);
    let edge = pf
        .edges
        .iter()
        .find(|e| e.kind == EdgeKind::Calls)
        .expect("call edge");
    assert_eq!(edge.target_qn, "helpers.Run");
    assert_eq!(edge.confidence_tier.as_deref(), Some("text"));
}

#[test]
fn extracts_const_and_var_nodes() {
    let src = "package main\nconst (\nA = 1\nB = 2\n)\nvar c string\n";
    let pf = parse(src);
    assert!(
        pf.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Constant && n.name == "A")
    );
    assert!(
        pf.nodes
            .iter()
            .any(|n| n.kind == NodeKind::Constant && n.name == "B")
    );
    let var = pf
        .nodes
        .iter()
        .find(|n| n.kind == NodeKind::Variable && n.name == "c")
        .expect("variable node");
    assert_eq!(var.return_type.as_deref(), Some("string"));
    assert_eq!(
        var.parent_name.as_deref(),
        Some("cmd/main.go::package::main")
    );
}

#[test]
fn resolves_method_call_on_receiver_scope() {
    let src = r#"
package main
type Foo struct{}
func (f *Foo) A() { f.B() }
func (f *Foo) B() {}
"#;
    let pf = parse(src);
    assert!(
        pf.edges.iter().any(|e| {
            e.kind == EdgeKind::Calls
                && e.source_qn == "cmd/main.go::method::Foo.A"
                && e.target_qn == "cmd/main.go::method::Foo.B"
        }),
        "edges: {:?}",
        pf.edges
    );
}

#[test]
fn resolves_method_call_on_receiver_alias() {
    let src = r#"
package main
type Foo struct{}
func (f *Foo) A() { alias := f; alias.B() }
func (f *Foo) B() {}
"#;
    let pf = parse(src);
    assert!(
        pf.edges.iter().any(|e| {
            e.kind == EdgeKind::Calls
                && e.source_qn == "cmd/main.go::method::Foo.A"
                && e.target_qn == "cmd/main.go::method::Foo.B"
        }),
        "edges: {:?}",
        pf.edges
    );
}

#[test]
fn resolves_method_call_on_typed_local_variable() {
    let src = r#"
package main
type Foo struct{}
func caller() { var local Foo; local.B() }
func (f Foo) B() {}
"#;
    let pf = parse(src);
    assert!(
        pf.edges.iter().any(|e| {
            e.kind == EdgeKind::Calls
                && e.source_qn == "cmd/main.go::fn::caller"
                && e.target_qn == "cmd/main.go::method::Foo.B"
        }),
        "edges: {:?}",
        pf.edges
    );
}

#[test]
fn resolves_method_call_on_function_return_local() {
    let src = r#"
package main
type Foo struct{}
func NewFoo() Foo { return Foo{} }
func caller() { local := NewFoo(); local.B() }
func (f Foo) B() {}
"#;
    let pf = parse(src);
    assert!(
        pf.edges.iter().any(|e| {
            e.kind == EdgeKind::Calls
                && e.source_qn == "cmd/main.go::fn::caller"
                && e.target_qn == "cmd/main.go::method::Foo.B"
        }),
        "edges: {:?}",
        pf.edges
    );
}

#[test]
fn resolves_method_call_on_method_return_local() {
    let src = r#"
package main
type Foo struct{}
func (f Foo) Clone() Foo { return f }
func caller(seed Foo) { local := seed.Clone(); local.B() }
func (f Foo) B() {}
"#;
    let pf = parse(src);
    assert!(
        pf.edges.iter().any(|e| {
            e.kind == EdgeKind::Calls
                && e.source_qn == "cmd/main.go::fn::caller"
                && e.target_qn == "cmd/main.go::method::Foo.B"
        }),
        "edges: {:?}",
        pf.edges
    );
}

#[test]
fn resolves_method_call_on_function_return_chain() {
    let src = r#"
package main
type Foo struct{}
func NewFoo() Foo { return Foo{} }
func caller() { NewFoo().B() }
func (f Foo) B() {}
"#;
    let pf = parse(src);
    assert!(
        pf.edges.iter().any(|e| {
            e.kind == EdgeKind::Calls
                && e.source_qn == "cmd/main.go::fn::caller"
                && e.target_qn == "cmd/main.go::method::Foo.B"
        }),
        "edges: {:?}",
        pf.edges
    );
}

#[test]
fn resolves_method_call_on_method_return_chain() {
    let src = r#"
package main
type Foo struct{}
func (f Foo) Clone() Foo { return f }
func caller(seed Foo) { seed.Clone().B() }
func (f Foo) B() {}
"#;
    let pf = parse(src);
    assert!(
        pf.edges.iter().any(|e| {
            e.kind == EdgeKind::Calls
                && e.source_qn == "cmd/main.go::fn::caller"
                && e.target_qn == "cmd/main.go::method::Foo.B"
        }),
        "edges: {:?}",
        pf.edges
    );
}

#[test]
fn resolves_method_call_on_struct_field_receiver() {
    let src = r#"
package main
type Foo struct{}
type Holder struct{ foo Foo }
func caller(holder Holder) { holder.foo.B() }
func (f Foo) B() {}
"#;
    let pf = parse(src);
    assert!(
        pf.edges.iter().any(|e| {
            e.kind == EdgeKind::Calls
                && e.source_qn == "cmd/main.go::fn::caller"
                && e.target_qn == "cmd/main.go::method::Foo.B"
        }),
        "edges: {:?}",
        pf.edges
    );
}

#[test]
fn resolves_method_call_on_returned_struct_field_chain() {
    let src = r#"
package main
type Foo struct{}
type Holder struct{ foo Foo }
func NewHolder() Holder { return Holder{} }
func caller() { NewHolder().foo.B() }
func (f Foo) B() {}
"#;
    let pf = parse(src);
    assert!(
        pf.edges.iter().any(|e| {
            e.kind == EdgeKind::Calls
                && e.source_qn == "cmd/main.go::fn::caller"
                && e.target_qn == "cmd/main.go::method::Foo.B"
        }),
        "edges: {:?}",
        pf.edges
    );
}

#[test]
fn resolves_method_call_on_embedded_field_receiver() {
    let src = r#"
package main
type Foo struct{}
type Holder struct{ Foo }
func caller(holder Holder) { holder.Foo.B() }
func (f Foo) B() {}
"#;
    let pf = parse(src);
    assert!(
        pf.edges.iter().any(|e| {
            e.kind == EdgeKind::Calls
                && e.source_qn == "cmd/main.go::fn::caller"
                && e.target_qn == "cmd/main.go::method::Foo.B"
        }),
        "edges: {:?}",
        pf.edges
    );
}

#[test]
fn resolves_promoted_method_call_on_embedded_receiver() {
    let src = r#"
package main
type Foo struct{}
type Holder struct{ Foo }
func caller(holder Holder) { holder.B() }
func (f Foo) B() {}
"#;
    let pf = parse(src);
    assert!(
        pf.edges.iter().any(|e| {
            e.kind == EdgeKind::Calls
                && e.source_qn == "cmd/main.go::fn::caller"
                && e.target_qn == "cmd/main.go::method::Foo.B"
        }),
        "edges: {:?}",
        pf.edges
    );
}
