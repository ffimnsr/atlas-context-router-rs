use super::*;

#[test]
fn trace_command_follows_downstream_calls_and_shortest_path() {
    let repo = setup_repo(&[
        ("src/main.rs", "mod a;\nmod b;\n\nfn main() { a::a(); }\n"),
        ("src/a.rs", "pub fn a() { crate::b::b(); }\n"),
        ("src/b.rs", "pub fn b() {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let data = read_json_data_output(
        "trace",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "trace",
                "src/a.rs::fn::a",
                "--direction",
                "down",
                "--depth",
                "2",
            ],
        ),
    );

    let names = data["nodes"]
        .as_array()
        .expect("trace nodes")
        .iter()
        .filter_map(|node| node["qualified_name"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        vec!["src/a.rs::fn::a", "src/b.rs::fn::b"],
        "expected downstream callee chain: {data:?}"
    );

    let path = read_json_data_output(
        "trace",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "trace",
                "src/a.rs::fn::a",
                "--to",
                "src/b.rs::fn::b",
            ],
        ),
    );
    assert_eq!(path["path"]["found"], json!(true));
    assert_eq!(path["summary"]["edge_count"], json!(1));
}
