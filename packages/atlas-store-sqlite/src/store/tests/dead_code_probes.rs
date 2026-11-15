use super::*;
use atlas_core::{EdgeKind, NodeKind};

/// Receiver method calls are recorded with a short `Type::method` target, so a
/// full `file::method::Type::method` node needs the short-form probe to avoid
/// looking uncalled.
#[test]
fn inbound_semantic_edge_targets_reports_short_method_targets() {
    let mut store = open_in_memory();
    let method = make_node(
        NodeKind::Method,
        "default",
        "a.rs::method::Budget::default",
        "a.rs",
        "rust",
    );
    let caller = make_node(NodeKind::Function, "make", "b.rs::fn::make", "b.rs", "rust");
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "a.rs",
            "h1",
            Some("rust"),
            None,
            &[method],
            &[],
        )
        .unwrap();
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "b.rs",
            "h2",
            Some("rust"),
            None,
            &[caller],
            &[make_edge(
                EdgeKind::Calls,
                "b.rs::fn::make",
                "Budget::default",
                "b.rs",
            )],
        )
        .unwrap();

    let found = store
        .inbound_semantic_edge_targets(&["Budget::default", "Budget::missing"])
        .unwrap();
    assert!(found.contains("Budget::default"));
    assert!(!found.contains("Budget::missing"));
}

#[test]
fn trait_impl_method_qnames_reports_only_trait_implementations() {
    let mut store = open_in_memory();
    let trait_node = make_node(
        NodeKind::Trait,
        "Clock",
        "a.rs::trait::Clock",
        "a.rs",
        "rust",
    );
    // Parser emits `trait_name` on the impl block, so external traits such as
    // `Default` are detectable without a trait node or edge.
    let mut trait_impl = make_node(
        NodeKind::Module,
        "impl Clock for SystemClock",
        "a.rs::impl::SystemClock",
        "a.rs",
        "rust",
    );
    trait_impl.extra_json = serde_json::json!({
        "scope_kind": "impl",
        "type_name": "SystemClock",
        "trait_name": "Clock"
    });
    let mut default_impl = make_node(
        NodeKind::Module,
        "impl Default for SystemClockHelpers",
        "a.rs::impl::SystemClockHelpers",
        "a.rs",
        "rust",
    );
    default_impl.extra_json = serde_json::json!({
        "scope_kind": "impl",
        "type_name": "SystemClockHelpers",
        "trait_name": "Default"
    });
    let mut trait_method = make_node(
        NodeKind::Method,
        "now_utc",
        "a.rs::method::SystemClock::now_utc",
        "a.rs",
        "rust",
    );
    trait_method.parent_name = Some("a.rs::impl::SystemClock".to_owned());
    let mut default_method = make_node(
        NodeKind::Method,
        "default",
        "a.rs::method::SystemClockHelpers::default",
        "a.rs",
        "rust",
    );
    default_method.parent_name = Some("a.rs::impl::SystemClockHelpers".to_owned());

    store
        .replace_file_graph_for_repo(
            "repo_test",
            "a.rs",
            "h1",
            Some("rust"),
            None,
            &[
                trait_node,
                trait_impl,
                default_impl,
                trait_method,
                default_method,
            ],
            &[],
        )
        .unwrap();

    let found = store
        .trait_impl_method_qnames(&[
            "a.rs::method::SystemClock::now_utc",
            "a.rs::method::SystemClockHelpers::default",
        ])
        .unwrap();
    assert!(found.contains("a.rs::method::SystemClock::now_utc"));
    assert!(
        found.contains("a.rs::method::SystemClockHelpers::default"),
        "external trait impls are detected from impl `trait_name` metadata"
    );
}
