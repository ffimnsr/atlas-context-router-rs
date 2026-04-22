use super::*;

#[test]
fn query_includes_owner_identity_for_ambiguous_workspace_results() {
    let repo = setup_repo(&[
        ("Cargo.toml", "[workspace]\nmembers = ['packages/*']\n"),
        (
            "packages/foo/Cargo.toml",
            "[package]\nname = 'foo'\nversion = '0.1.0'\nedition = '2021'\n",
        ),
        ("packages/foo/src/lib.rs", "pub fn helper() {}\n"),
        (
            "packages/bar/Cargo.toml",
            "[package]\nname = 'bar'\nversion = '0.1.0'\nedition = '2021'\n",
        ),
        ("packages/bar/src/lib.rs", "pub fn helper() {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let query = stdout_text(&run_atlas(repo.path(), &["query", "helper"]));
    assert_contains_all(
        &query,
        &[
            "packages/foo/src/lib.rs::fn::helper",
            "packages/bar/src/lib.rs::fn::helper",
            "[owner cargo:packages/foo/Cargo.toml]",
            "[owner cargo:packages/bar/Cargo.toml]",
        ],
    );
}

#[test]
fn update_rename_across_package_roots_refreshes_owner_identity() {
    let repo = setup_repo(&[
        (
            "crates/foo/Cargo.toml",
            "[package]\nname = 'foo'\nversion = '0.1.0'\nedition = '2021'\n",
        ),
        ("crates/foo/src/lib.rs", "pub fn helper() {}\n"),
        (
            "crates/bar/Cargo.toml",
            "[package]\nname = 'bar'\nversion = '0.1.0'\nedition = '2021'\n",
        ),
        ("crates/bar/src/mod.rs", "pub fn marker() {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);
    run_command(
        repo.path(),
        "git",
        &["mv", "crates/foo/src/lib.rs", "crates/bar/src/ported.rs"],
    );

    let update = read_json_data_output(
        "update",
        run_atlas(repo.path(), &["--json", "update", "--staged"]),
    );
    assert_eq!(update["renamed"], json!(0));
    assert!(update["parsed"].as_u64().unwrap_or_default() >= 1);

    let store = open_store(repo.path());
    let new_owner = store
        .file_owner("crates/bar/src/ported.rs")
        .expect("new owner lookup")
        .expect("stored new owner");
    assert_eq!(new_owner.owner_id, "cargo:crates/bar/Cargo.toml");
    assert!(
        store
            .file_owner("crates/foo/src/lib.rs")
            .expect("old owner lookup")
            .is_none(),
        "old path owner metadata must be removed"
    );
}

#[test]
fn multi_package_workspace_flow_uses_owner_identity_end_to_end() {
    let repo = setup_repo(&[
        (
            "package.json",
            r#"{"private":true,"workspaces":["apps/*","packages/*"]}"#,
        ),
        (
            "tsconfig.json",
            r#"{
    "compilerOptions": {
        "baseUrl": ".",
        "paths": {
            "@ui/*": ["packages/ui/src/*"]
        }
    }
}
"#,
        ),
        (
            "apps/web/package.json",
            r#"{"name":"web","version":"0.1.0"}"#,
        ),
        (
            "apps/web/src/app.ts",
            "import { helper } from '@ui/helper';\nexport function run(): string {\n    return helper();\n}\n",
        ),
        (
            "packages/ui/package.json",
            r#"{"name":"ui","version":"0.1.0"}"#,
        ),
        (
            "packages/ui/src/helper.ts",
            "export function helper(): string {\n    return 'v1';\n}\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let app_edges = store
        .edges_by_file("apps/web/src/app.ts")
        .expect("app edges after build");
    assert!(
        app_edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "packages/ui/src/helper.ts::fn::helper"
        }),
        "build must resolve cross-package helper call before impact/review checks: {app_edges:?}"
    );

    let analyze = read_json_data_output(
        "analyze_dependency",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "analyze",
                "dependency",
                "packages/ui/src/helper.ts::fn::helper",
            ],
        ),
    );
    assert!(
        analyze["blocking_references"]
            .as_array()
            .expect("blocking references array")
            .iter()
            .any(|node| node["file_path"] == json!("apps/web/src/app.ts")),
        "reasoning must see cross-package dependency: {analyze:?}"
    );

    write_repo_file(
        repo.path(),
        "apps/web/src/app.ts",
        "import { helper } from '@ui/helper';\nexport function run(): string {\n    return `${helper()}!`;\n}\n",
    );

    let update = read_json_data_output(
        "update",
        run_atlas(repo.path(), &["--json", "update", "--base", "HEAD"]),
    );
    assert!(update["parsed"].as_u64().unwrap_or_default() >= 1);

    run_atlas(repo.path(), &["build"]);

    let impact = read_json_data_output(
        "impact",
        run_atlas(repo.path(), &["--json", "impact", "--base", "HEAD"]),
    );
    assert!(
        impact["analysis"]["boundary_violations"]
            .as_array()
            .expect("boundary violations array")
            .iter()
            .any(|violation| violation["kind"] == json!("cross_package")),
        "impact must flag cross-package boundary: {impact:?}"
    );

    let review = stdout_text(&run_atlas(
        repo.path(),
        &["review-context", "--base", "HEAD"],
    ));
    assert_contains_all(
        &review,
        &[
            "Changed files (1):",
            "  apps/web/src/app.ts",
            "Cross-package impact: true",
        ],
    );
}
