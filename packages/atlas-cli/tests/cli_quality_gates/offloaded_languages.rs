use super::*;

#[test]
fn build_and_update_replace_bundled_language_file_graphs() {
    let repo = setup_repo(&[
        (
            "src/main.rs",
            "fn helper() {}\n\nfn run() {\n    helper();\n}\n",
        ),
        (
            "src/app.go",
            "package main\n\nimport \"fmt\"\n\nfunc Run() {\n\tHelper()\n}\n\nfunc Helper() {\n\tfmt.Println(\"ok\")\n}\n",
        ),
        (
            "src/app.py",
            "import os\n\n\ndef run():\n    helper()\n\n\ndef helper():\n    pass\n",
        ),
        (
            "src/native.c",
            "#include \"util.h\"\ntypedef unsigned long size_t;\nstatic void helper(void) {}\nvoid run(void) { helper(); }\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    let build = read_json_data_output("build", run_atlas(repo.path(), &["--json", "build"]));
    assert_eq!(build["parse_errors"], json!(0));
    assert!(build["parsed"].as_u64().unwrap_or_default() >= 4);

    let store = open_store(repo.path());

    let rust_nodes = store.nodes_by_file("src/main.rs").expect("rust nodes");
    assert!(
        rust_nodes
            .iter()
            .any(|node| node.qualified_name == "src/main.rs::fn::run")
    );
    assert!(
        rust_nodes
            .iter()
            .any(|node| node.qualified_name == "src/main.rs::fn::helper")
    );
    let rust_edges = store.edges_by_file("src/main.rs").expect("rust edges");
    assert!(rust_edges.iter().any(|edge| {
        edge.kind == EdgeKind::Calls && edge.target_qn == "src/main.rs::fn::helper"
    }));

    let go_nodes = store.nodes_by_file("src/app.go").expect("go nodes");
    assert!(
        go_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "fmt")
    );
    assert!(
        go_nodes
            .iter()
            .any(|node| node.qualified_name == "src/app.go::fn::Run")
    );
    let go_edges = store.edges_by_file("src/app.go").expect("go edges");
    assert!(go_edges.iter().any(|edge| {
        edge.kind == EdgeKind::Calls && edge.target_qn == "src/app.go::fn::Helper"
    }));

    let python_nodes = store.nodes_by_file("src/app.py").expect("python nodes");
    assert!(
        python_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "os")
    );
    assert!(
        python_nodes
            .iter()
            .any(|node| node.qualified_name == "src/app.py::fn::run")
    );
    assert!(
        python_nodes
            .iter()
            .any(|node| node.qualified_name == "src/app.py::fn::helper")
    );
    let python_edges = store.edges_by_file("src/app.py").expect("python edges");
    assert!(
        python_edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls && edge.target_qn.ends_with("::fn::helper")
        }),
        "expected python helper call edge; edges: {python_edges:?}"
    );

    let c_nodes = store.nodes_by_file("src/native.c").expect("c nodes");
    assert!(
        c_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "util.h")
    );
    assert!(
        c_nodes
            .iter()
            .any(|node| node.qualified_name == "src/native.c::typedef::size_t")
    );

    write_repo_file(
        repo.path(),
        "src/main.rs",
        "fn assist() {}\n\nfn run() {\n    assist();\n}\n",
    );
    write_repo_file(
        repo.path(),
        "src/app.go",
        "package main\n\nimport \"os\"\n\nfunc Run() {\n\tAssist()\n}\n\nfunc Assist() {}\n",
    );
    write_repo_file(
        repo.path(),
        "src/app.py",
        "import sys\n\n\ndef run():\n    assist()\n\n\ndef assist():\n    pass\n",
    );
    write_repo_file(
        repo.path(),
        "src/native.c",
        "#include \"other.h\"\ntypedef unsigned long count_t;\nstatic void assist(void) {}\nvoid run(void) { assist(); }\n",
    );

    let update = read_json_data_output(
        "update",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "update",
                "--files",
                "src/main.rs",
                "--files",
                "src/app.go",
                "--files",
                "src/app.py",
                "--files",
                "src/native.c",
            ],
        ),
    );
    assert_eq!(update["parse_errors"], json!(0));
    assert_eq!(update["parsed"], json!(4));

    let store = open_store(repo.path());

    let rust_nodes = store
        .nodes_by_file("src/main.rs")
        .expect("updated rust nodes");
    assert!(
        rust_nodes
            .iter()
            .any(|node| node.qualified_name == "src/main.rs::fn::assist")
    );
    assert!(
        !rust_nodes
            .iter()
            .any(|node| node.qualified_name == "src/main.rs::fn::helper")
    );

    let go_nodes = store.nodes_by_file("src/app.go").expect("updated go nodes");
    assert!(
        go_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "os")
    );
    assert!(
        go_nodes
            .iter()
            .any(|node| node.qualified_name == "src/app.go::fn::Assist")
    );
    assert!(
        !go_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "fmt")
    );

    let python_nodes = store
        .nodes_by_file("src/app.py")
        .expect("updated python nodes");
    assert!(
        python_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "sys")
    );
    assert!(
        python_nodes
            .iter()
            .any(|node| node.qualified_name == "src/app.py::fn::assist")
    );
    assert!(
        !python_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "os")
    );

    let c_nodes = store
        .nodes_by_file("src/native.c")
        .expect("updated c nodes");
    assert!(
        c_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "other.h")
    );
    assert!(
        c_nodes
            .iter()
            .any(|node| node.qualified_name == "src/native.c::typedef::count_t")
    );
    assert!(
        !c_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "util.h")
    );
}
