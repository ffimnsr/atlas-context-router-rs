use super::*;

// --- flows ---------------------------------------------------------------

#[test]
fn flows_create_list_delete() {
    let store = open_in_memory();
    assert!(store.list_flows().unwrap().is_empty());

    let id = store
        .create_flow("login", Some("auth"), Some("Login flow"))
        .unwrap();
    assert!(id > 0);

    let flows = store.list_flows().unwrap();
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0].name, "login");
    assert_eq!(flows[0].kind.as_deref(), Some("auth"));
    assert_eq!(flows[0].description.as_deref(), Some("Login flow"));

    store.delete_flow(id).unwrap();
    assert!(store.list_flows().unwrap().is_empty());
}

#[test]
fn flows_get_by_id_and_name() {
    let store = open_in_memory();
    let id = store.create_flow("checkout", None, None).unwrap();

    let by_id = store.get_flow(id).unwrap().expect("must exist");
    assert_eq!(by_id.name, "checkout");

    let by_name = store
        .get_flow_by_name("checkout")
        .unwrap()
        .expect("must exist");
    assert_eq!(by_name.id, id);

    assert!(store.get_flow(9999).unwrap().is_none());
    assert!(store.get_flow_by_name("missing").unwrap().is_none());
}

#[test]
fn flows_add_remove_member_lifecycle() {
    let store = open_in_memory();
    let flow_id = store.create_flow("pipeline", None, None).unwrap();

    store
        .add_flow_member(flow_id, "pkg::fn::step_a", Some(0), Some("entry"))
        .unwrap();
    store
        .add_flow_member(flow_id, "pkg::fn::step_b", Some(1), None)
        .unwrap();
    store
        .add_flow_member(flow_id, "pkg::fn::step_c", Some(2), None)
        .unwrap();

    let members = store.get_flow_members(flow_id).unwrap();
    assert_eq!(members.len(), 3);
    assert_eq!(members[0].node_qualified_name, "pkg::fn::step_a");
    assert_eq!(members[0].position, Some(0));
    assert_eq!(members[0].role.as_deref(), Some("entry"));
    assert_eq!(members[2].node_qualified_name, "pkg::fn::step_c");

    store
        .remove_flow_member(flow_id, "pkg::fn::step_b")
        .unwrap();
    let members = store.get_flow_members(flow_id).unwrap();
    assert_eq!(members.len(), 2);
    assert!(
        !members
            .iter()
            .any(|m| m.node_qualified_name == "pkg::fn::step_b")
    );
}

#[test]
fn flow_membership_survives_node_rebuild() {
    let mut store = open_in_memory();
    let node = make_node(NodeKind::Function, "step", "pkg::fn::step", "a.rs", "rust");
    store
        .replace_file_graph("a.rs", "h1", None, None, std::slice::from_ref(&node), &[])
        .unwrap();

    let flow_id = store.create_flow("myflow", None, None).unwrap();
    store
        .add_flow_member(flow_id, "pkg::fn::step", Some(0), None)
        .unwrap();

    // Simulate `atlas build` re-indexing the same file.
    store
        .replace_file_graph("a.rs", "h2", None, None, &[node], &[])
        .unwrap();

    // Membership must still exist after rebuild.
    let members = store.get_flow_members(flow_id).unwrap();
    assert_eq!(members.len(), 1, "membership must survive node rebuild");
    assert_eq!(members[0].node_qualified_name, "pkg::fn::step");
}

#[test]
fn flow_add_member_update_idempotent() {
    let store = open_in_memory();
    let flow_id = store.create_flow("f", None, None).unwrap();
    store
        .add_flow_member(flow_id, "a::b", Some(0), Some("old"))
        .unwrap();
    // Replace with updated position/role.
    store
        .add_flow_member(flow_id, "a::b", Some(5), Some("new"))
        .unwrap();
    let members = store.get_flow_members(flow_id).unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].position, Some(5));
    assert_eq!(members[0].role.as_deref(), Some("new"));
}

#[test]
fn flows_for_node_returns_correct_flows() {
    let store = open_in_memory();
    let f1 = store.create_flow("flow1", None, None).unwrap();
    let f2 = store.create_flow("flow2", None, None).unwrap();
    store.add_flow_member(f1, "my::fn", None, None).unwrap();
    store.add_flow_member(f2, "my::fn", None, None).unwrap();
    store.add_flow_member(f2, "other::fn", None, None).unwrap();

    let flows = store.flows_for_node("my::fn").unwrap();
    assert_eq!(flows.len(), 2);

    let flows = store.flows_for_node("other::fn").unwrap();
    assert_eq!(flows.len(), 1);
    assert_eq!(flows[0].name, "flow2");

    let flows = store.flows_for_node("no::such::fn").unwrap();
    assert!(flows.is_empty());
}

#[test]
fn flow_delete_cascades_memberships() {
    let store = open_in_memory();
    let flow_id = store.create_flow("cascade-test", None, None).unwrap();
    store.add_flow_member(flow_id, "x::fn", None, None).unwrap();

    store.delete_flow(flow_id).unwrap();

    // Cascaded — querying members of deleted flow returns empty.
    let members = store.get_flow_members(flow_id).unwrap();
    assert!(members.is_empty());
}

// --- communities ---------------------------------------------------------

#[test]
fn communities_create_list_delete() {
    let store = open_in_memory();
    assert!(store.list_communities().unwrap().is_empty());

    let id = store
        .create_community("cluster-a", Some("louvain"), Some(0), None)
        .unwrap();
    assert!(id > 0);

    let comms = store.list_communities().unwrap();
    assert_eq!(comms.len(), 1);
    assert_eq!(comms[0].name, "cluster-a");
    assert_eq!(comms[0].algorithm.as_deref(), Some("louvain"));
    assert_eq!(comms[0].level, Some(0));

    store.delete_community(id).unwrap();
    assert!(store.list_communities().unwrap().is_empty());
}

#[test]
fn communities_get_by_id_and_name() {
    let store = open_in_memory();
    let id = store.create_community("c1", None, None, None).unwrap();

    let by_id = store.get_community(id).unwrap().expect("must exist");
    assert_eq!(by_id.name, "c1");

    let by_name = store
        .get_community_by_name("c1")
        .unwrap()
        .expect("must exist");
    assert_eq!(by_name.id, id);

    assert!(store.get_community(9999).unwrap().is_none());
    assert!(store.get_community_by_name("missing").unwrap().is_none());
}

#[test]
fn community_parent_child_relationship() {
    let store = open_in_memory();
    let parent_id = store
        .create_community("parent", Some("louvain"), Some(0), None)
        .unwrap();
    let child_id = store
        .create_community("child", Some("louvain"), Some(1), Some(parent_id))
        .unwrap();

    let child = store.get_community(child_id).unwrap().expect("must exist");
    assert_eq!(child.parent_community_id, Some(parent_id));
}

#[test]
fn community_add_remove_node_lifecycle() {
    let store = open_in_memory();
    let comm_id = store.create_community("grp", None, None, None).unwrap();

    store.add_community_node(comm_id, "a::fn").unwrap();
    store.add_community_node(comm_id, "b::struct").unwrap();
    store.add_community_node(comm_id, "c::method").unwrap();

    let nodes = store.get_community_nodes(comm_id).unwrap();
    assert_eq!(nodes.len(), 3);
    // ordered by qname
    assert_eq!(nodes[0].node_qualified_name, "a::fn");
    assert_eq!(nodes[1].node_qualified_name, "b::struct");
    assert_eq!(nodes[2].node_qualified_name, "c::method");

    store.remove_community_node(comm_id, "b::struct").unwrap();
    let nodes = store.get_community_nodes(comm_id).unwrap();
    assert_eq!(nodes.len(), 2);
    assert!(!nodes.iter().any(|n| n.node_qualified_name == "b::struct"));
}

#[test]
fn community_membership_survives_node_rebuild() {
    let mut store = open_in_memory();
    let node = make_node(NodeKind::Function, "fn_a", "pkg::fn::fn_a", "a.rs", "rust");
    store
        .replace_file_graph("a.rs", "h1", None, None, std::slice::from_ref(&node), &[])
        .unwrap();

    let comm_id = store.create_community("mycomm", None, None, None).unwrap();
    store.add_community_node(comm_id, "pkg::fn::fn_a").unwrap();

    // Rebuild — simulates `atlas build`.
    store
        .replace_file_graph("a.rs", "h2", None, None, &[node], &[])
        .unwrap();

    let nodes = store.get_community_nodes(comm_id).unwrap();
    assert_eq!(
        nodes.len(),
        1,
        "community membership must survive node rebuild"
    );
    assert_eq!(nodes[0].node_qualified_name, "pkg::fn::fn_a");
}

#[test]
fn community_add_node_idempotent() {
    let store = open_in_memory();
    let comm_id = store.create_community("idem", None, None, None).unwrap();
    store.add_community_node(comm_id, "x::fn").unwrap();
    store.add_community_node(comm_id, "x::fn").unwrap(); // duplicate — no error
    let nodes = store.get_community_nodes(comm_id).unwrap();
    assert_eq!(nodes.len(), 1);
}

#[test]
fn communities_for_node_returns_correct() {
    let store = open_in_memory();
    let c1 = store.create_community("c1", None, None, None).unwrap();
    let c2 = store.create_community("c2", None, None, None).unwrap();
    store.add_community_node(c1, "shared::fn").unwrap();
    store.add_community_node(c2, "shared::fn").unwrap();
    store.add_community_node(c2, "only_c2::fn").unwrap();

    let comms = store.communities_for_node("shared::fn").unwrap();
    assert_eq!(comms.len(), 2);

    let comms = store.communities_for_node("only_c2::fn").unwrap();
    assert_eq!(comms.len(), 1);
    assert_eq!(comms[0].name, "c2");

    let comms = store.communities_for_node("nobody").unwrap();
    assert!(comms.is_empty());
}

#[test]
fn community_delete_cascades_nodes() {
    let store = open_in_memory();
    let comm_id = store.create_community("cascade", None, None, None).unwrap();
    store.add_community_node(comm_id, "x::fn").unwrap();

    store.delete_community(comm_id).unwrap();

    let nodes = store.get_community_nodes(comm_id).unwrap();
    assert!(nodes.is_empty());
}
