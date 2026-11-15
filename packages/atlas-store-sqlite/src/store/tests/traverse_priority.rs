use super::*;
use atlas_core::{EdgeKind, NodeKind};

/// Regression: same-file containment rows used to consume the whole node
/// budget because equal-depth rows were ordered by qualified name only, which
/// evicted a cross-file `calls` edge that sorts later alphabetically.
#[test]
fn traverse_keeps_cross_file_calls_edge_under_small_node_budget() {
    let mut store = open_in_memory();
    let file = make_node(NodeKind::File, "a.rs", "a.rs", "a.rs", "rust");
    let seed = make_node(NodeKind::Function, "seed", "a.rs::fn::seed", "a.rs", "rust");
    let noise = (0..6)
        .map(|index| {
            make_node(
                NodeKind::Function,
                &format!("helper{index}"),
                &format!("a.rs::fn::helper{index}"),
                "a.rs",
                "rust",
            )
        })
        .collect::<Vec<_>>();
    let caller = make_node(
        NodeKind::Function,
        "caller",
        "z.rs::fn::caller",
        "z.rs",
        "rust",
    );

    let mut nodes = vec![file, seed];
    nodes.extend(noise.iter().cloned());
    let mut edges = Vec::new();
    for helper in &noise {
        edges.push(make_edge(
            EdgeKind::Contains,
            "a.rs",
            &helper.qualified_name,
            "a.rs",
        ));
    }

    store
        .replace_file_graph_for_repo("repo_test", "a.rs", "h", None, None, &nodes, &edges)
        .unwrap();
    let caller_edge = make_edge(
        EdgeKind::Calls,
        "z.rs::fn::caller",
        "a.rs::fn::seed",
        "z.rs",
    );
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "z.rs",
            "h",
            None,
            None,
            &[caller],
            &[caller_edge],
        )
        .unwrap();

    let result = store
        .traverse_from_qnames(&["a.rs::fn::seed"], 1, 3, 100)
        .unwrap();

    let impacted = result
        .impacted_nodes
        .iter()
        .map(|node| node.qualified_name.as_str())
        .collect::<Vec<_>>();
    assert!(
        impacted.contains(&"z.rs::fn::caller"),
        "cross-file caller must survive a small node budget, got {impacted:?}"
    );
    assert_eq!(
        impacted.first().copied(),
        Some("z.rs::fn::caller"),
        "calls edges must rank ahead of containment rows"
    );
    assert!(result.impacted_files.iter().any(|entry| entry == "z.rs"));
}
