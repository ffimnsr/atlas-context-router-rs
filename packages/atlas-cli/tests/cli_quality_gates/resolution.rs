use super::*;

#[test]
fn build_resolves_rust_same_package_call_targets() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store.edges_by_file("src/main.rs").expect("main edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "src/lib.rs::fn::helper"
                && edge.confidence_tier.as_deref() == Some("same_package")
        }),
        "expected src/main.rs helper call to resolve into src/lib.rs::fn::helper; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_rust_same_package_across_directories_in_standalone_package() {
    let repo = setup_repo(&[
        (
            "crates/foo/Cargo.toml",
            "[package]\nname = 'foo'\nversion = '0.1.0'\nedition = '2021'\n",
        ),
        ("crates/foo/src/lib.rs", "pub fn helper() {}\n"),
        ("crates/foo/examples/demo.rs", "fn main() { helper(); }\n"),
        (
            "crates/bar/Cargo.toml",
            "[package]\nname = 'bar'\nversion = '0.1.0'\nedition = '2021'\n",
        ),
        ("crates/bar/src/lib.rs", "pub fn helper() {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store
        .edges_by_file("crates/foo/examples/demo.rs")
        .expect("demo edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "crates/foo/src/lib.rs::fn::helper"
                && edge.confidence_tier.as_deref() == Some("same_package")
        }),
        "expected example call to resolve into crates/foo/src/lib.rs::fn::helper; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_rust_associated_function_by_receiver_hint() {
    let repo = setup_repo(&[
        (
            "src/builder.rs",
            "pub struct Builder;\nimpl Builder { pub fn new() -> Self { Self } }\n",
        ),
        (
            "src/other.rs",
            "pub struct Other;\nimpl Other { pub fn new() -> Self { Self } }\n",
        ),
        (
            "src/main.rs",
            "mod builder;\nmod other;\nuse builder::Builder;\nfn main() { Builder::new(); }\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store.edges_by_file("src/main.rs").expect("main edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "src/builder.rs::method::Builder::new"
                && edge.confidence_tier.as_deref() == Some("same_package")
        }),
        "expected Builder::new to resolve via receiver hint; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_typescript_namespace_import_calls() {
    let repo = setup_repo(&[
        (
            "src/app.ts",
            "import * as utils from './utils';\nexport function caller(): void { utils.helper(); }\n",
        ),
        ("src/utils.ts", "export function helper(): void {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store.edges_by_file("src/app.ts").expect("app edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "src/utils.ts::fn::helper"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected namespace import call to resolve into src/utils.ts::fn::helper; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_typescript_path_alias_calls() {
    let repo = setup_repo(&[
        (
            "tsconfig.json",
            r#"{
  "compilerOptions": {
    "paths": {
            "@utils/*": ["./src/utils/*"]
    }
  }
}
"#,
        ),
        (
            "src/app.ts",
            "import * as math from '@utils/math';\nexport function caller(): void { math.helper(); }\n",
        ),
        ("src/utils/math.ts", "export function helper(): void {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store.edges_by_file("src/app.ts").expect("app edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "src/utils/math.ts::fn::helper"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected path-alias call to resolve into src/utils/math.ts::fn::helper; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_nested_typescript_path_alias_calls() {
    let repo = setup_repo(&[
        (
            "apps/web/tsconfig.json",
            r#"{
  "compilerOptions": {
    "paths": {
            "@lib/*": ["./src/lib/*"]
    }
  }
}
"#,
        ),
        (
            "apps/web/src/app.ts",
            "import * as math from '@lib/math';\nexport function caller(): void { math.helper(); }\n",
        ),
        (
            "apps/web/src/lib/math.ts",
            "export function helper(): void {}\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store
        .edges_by_file("apps/web/src/app.ts")
        .expect("nested app edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "apps/web/src/lib/math.ts::fn::helper"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected nested path-alias call to resolve into apps/web/src/lib/math.ts::fn::helper; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_legacy_typescript_baseurl_prefixed_paths_calls() {
    let repo = setup_repo(&[
        (
            "tsconfig.json",
            r#"{
  "compilerOptions": {
    "baseUrl": ".",
    "paths": {
      "@utils/*": ["src/utils/*"]
    }
  }
}
"#,
        ),
        (
            "src/app.ts",
            "import * as math from '@utils/math';\nexport function caller(): void { math.helper(); }\n",
        ),
        ("src/utils/math.ts", "export function helper(): void {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store.edges_by_file("src/app.ts").expect("app edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "src/utils/math.ts::fn::helper"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected legacy baseUrl-prefixed path alias to keep resolving into src/utils/math.ts::fn::helper; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_typescript_extended_tsconfig_alias_calls() {
    let repo = setup_repo(&[
        (
            "configs/tsconfig.base.json",
            r#"{
  "compilerOptions": {
    "paths": {
            "@shared/*": ["../src/shared/*"]
    }
  }
}
"#,
        ),
        (
            "apps/web/tsconfig.json",
            r#"{
  "extends": "../../configs/tsconfig.base.json"
}
"#,
        ),
        ("src/shared/math.ts", "export function helper(): void {}\n"),
        (
            "apps/web/app.ts",
            "import * as math from '@shared/math';\nexport function caller(): void { math.helper(); }\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store
        .edges_by_file("apps/web/app.ts")
        .expect("extended tsconfig app edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "src/shared/math.ts::fn::helper"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected extended-tsconfig alias call to resolve into src/shared/math.ts::fn::helper; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_typescript_reexport_chain_calls() {
    let repo = setup_repo(&[
        (
            "src/app.ts",
            "import { helper } from './barrel';\nexport function caller(): void { helper(); }\n",
        ),
        ("src/barrel.ts", "export { helper } from './impl';\n"),
        ("src/impl.ts", "export function helper(): void {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store.edges_by_file("src/app.ts").expect("app edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "src/impl.ts::fn::helper"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected re-export chain call to resolve into src/impl.ts::fn::helper; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_tsx_jsx_component_render_calls() {
    let repo = setup_repo(&[
        (
            "src/history-tab.tsx",
            "import { memo } from 'react';\nimport { SideFilterControl } from './side-filter-control';\nexport const HistoryTab = memo(function HistoryTab() { return <SideFilterControl />; });\n",
        ),
        (
            "src/side-filter-control.tsx",
            "export function SideFilterControl() { return <button />; }\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store
        .edges_by_file("src/history-tab.tsx")
        .expect("history tab edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.source_qn == "src/history-tab.tsx::fn::HistoryTab"
                && edge.target_qn == "src/side-filter-control.tsx::fn::SideFilterControl"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected JSX render call to resolve into imported SideFilterControl; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_typescript_package_extends_alias_calls() {
    let repo = setup_repo(&[
        (
            "node_modules/@atlas/tsconfig/base.json",
            r#"{
  "compilerOptions": {
    "paths": {
            "@shared/*": ["../../../src/shared/*"]
    }
  }
}
"#,
        ),
        (
            "apps/web/tsconfig.json",
            r#"{
  "extends": "@atlas/tsconfig/base"
}
"#,
        ),
        (
            "apps/web/app.ts",
            "import * as math from '@shared/math';\nexport function caller(): void { math.helper(); }\n",
        ),
        ("src/shared/math.ts", "export function helper(): void {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store
        .edges_by_file("apps/web/app.ts")
        .expect("package-extends app edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "src/shared/math.ts::fn::helper"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected package-style tsconfig extends to resolve alias into src/shared/math.ts::fn::helper; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_typescript_catch_all_paths_calls() {
    let repo = setup_repo(&[
        (
            "tsconfig.json",
            r#"{
  "compilerOptions": {
    "paths": {
      "*": ["./src/*"]
    }
  }
}
"#,
        ),
        (
            "src/app.ts",
            "import { helper } from 'utils';\nexport function caller(): void { helper(); }\n",
        ),
        ("src/utils.ts", "export function helper(): void {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store.edges_by_file("src/app.ts").expect("app edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "src/utils.ts::fn::helper"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected catch-all paths mapping to resolve bare import into src/utils.ts::fn::helper; edges: {edges:?}"
    );
}

#[test]
fn build_does_not_resolve_typescript_baseurl_only_bare_import_calls() {
    let repo = setup_repo(&[
        (
            "tsconfig.json",
            r#"{
  "compilerOptions": {
    "baseUrl": "./src"
  }
}
"#,
        ),
        (
            "src/app.ts",
            "import { helper } from 'utils';\nexport function caller(): void { helper(); }\n",
        ),
        ("src/utils.ts", "export function helper(): void {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store.edges_by_file("src/app.ts").expect("app edges");
    assert!(
        !edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "src/utils.ts::fn::helper"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected baseUrl-only config not to resolve bare import under TS6 semantics; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_python_relative_import_calls() {
    let repo = setup_repo(&[
        ("pkg/__init__.py", ""),
        (
            "pkg/main.py",
            "from .helpers import ping\n\ndef caller():\n    ping()\n",
        ),
        ("pkg/helpers.py", "def ping():\n    pass\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store.edges_by_file("pkg/main.py").expect("main edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "pkg/helpers.py::fn::ping"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected relative import call to resolve into pkg/helpers.py::fn::ping; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_python_package_submodule_alias_calls() {
    let repo = setup_repo(&[
        ("pkg/__init__.py", ""),
        (
            "pkg/main.py",
            "from pkg import helpers as helpers_mod\n\ndef caller():\n    helpers_mod.ping()\n",
        ),
        ("pkg/helpers.py", "def ping():\n    pass\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store.edges_by_file("pkg/main.py").expect("main edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "pkg/helpers.py::fn::ping"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected package submodule alias call to resolve into pkg/helpers.py::fn::ping; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_python_package_init_export_calls() {
    let repo = setup_repo(&[
        ("pkg/__init__.py", "def ping():\n    pass\n"),
        (
            "pkg/main.py",
            "from pkg import ping\n\ndef caller():\n    ping()\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store.edges_by_file("pkg/main.py").expect("main edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "pkg/__init__.py::fn::ping"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected package __init__ export call to resolve into pkg/__init__.py::fn::ping; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_python_wildcard_import_calls() {
    let repo = setup_repo(&[
        ("pkg/__init__.py", "def ping():\n    pass\n"),
        (
            "pkg/main.py",
            "from pkg import *\n\ndef caller():\n    ping()\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store.edges_by_file("pkg/main.py").expect("main edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "pkg/__init__.py::fn::ping"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected wildcard import call to resolve into pkg/__init__.py::fn::ping; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_go_local_module_import_calls() {
    let repo = setup_repo(&[
        ("go.mod", "module example.com/demo\n\ngo 1.22\n"),
        (
            "cmd/app/main.go",
            "package main\n\nimport \"example.com/demo/internal/helpers\"\n\nfunc caller() { helpers.Run() }\n",
        ),
        (
            "internal/helpers/run.go",
            "package helpers\n\nfunc Run() {}\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store
        .edges_by_file("cmd/app/main.go")
        .expect("go caller edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "internal/helpers/run.go::fn::Run"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected local-module import call to resolve into internal/helpers/run.go::fn::Run; edges: {edges:?}"
    );
}

#[test]
fn build_and_update_replace_json_toml_file_graphs() {
    let repo = setup_repo(&[
        (
            "config/app.json",
            "{\n  \"service\": { \"mode\": \"dev\" },\n  \"enabled\": true\n}\n",
        ),
        (
            "Cargo.toml",
            "[package]\nname = \"atlas\"\nversion = \"0.1.0\"\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    let build = read_json_data_output("build", run_atlas(repo.path(), &["--json", "build"]));
    assert_eq!(build["parse_errors"], json!(0));
    assert!(build["parsed"].as_u64().unwrap_or_default() >= 2);

    let store = open_store(repo.path());
    let json_nodes = store.nodes_by_file("config/app.json").expect("json nodes");
    assert!(
        json_nodes
            .iter()
            .any(|node| node.qualified_name == "config/app.json::key::service.mode")
    );
    assert!(json_nodes.iter().any(|node| node.kind == NodeKind::Module));
    let toml_nodes = store.nodes_by_file("Cargo.toml").expect("toml nodes");
    assert!(
        toml_nodes
            .iter()
            .any(|node| node.qualified_name == "Cargo.toml::key::package.name")
    );

    write_repo_file(
        repo.path(),
        "config/app.json",
        "{\n  \"service\": { \"port\": 8080 },\n  \"enabled\": false\n}\n",
    );
    write_repo_file(
        repo.path(),
        "Cargo.toml",
        "[package]\nname = \"atlas-renamed\"\nversion = \"0.2.0\"\n",
    );

    let update = read_json_data_output(
        "update",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "update",
                "--files",
                "config/app.json",
                "--files",
                "Cargo.toml",
            ],
        ),
    );
    assert_eq!(update["parse_errors"], json!(0));
    assert_eq!(update["parsed"], json!(2));

    let store = open_store(repo.path());
    let json_nodes = store
        .nodes_by_file("config/app.json")
        .expect("updated json nodes");
    assert!(
        !json_nodes
            .iter()
            .any(|node| node.qualified_name == "config/app.json::key::service.mode")
    );
    assert!(
        json_nodes
            .iter()
            .any(|node| node.qualified_name == "config/app.json::key::service.port")
    );
    let toml_nodes = store
        .nodes_by_file("Cargo.toml")
        .expect("updated toml nodes");
    assert!(
        toml_nodes
            .iter()
            .any(|node| node.qualified_name == "Cargo.toml::key::package.name")
    );
    let name_node = toml_nodes
        .iter()
        .find(|node| node.qualified_name == "Cargo.toml::key::package.name")
        .expect("package name node");
    assert_eq!(name_node.line_start, 2);
}

#[test]
fn build_and_update_replace_markup_shell_file_graphs() {
    let repo = setup_repo(&[
        (
            "public/index.html",
            "<!doctype html><html><body><script src=\"app.js\"></script></body></html>\n",
        ),
        (
            "assets/app.css",
            "@import url('base.css');\n.button { color: red; }\n",
        ),
        (
            "scripts/deploy.sh",
            "source ./env.sh\nsetup() {\n  helper\n}\nhelper() {\n  echo ok\n}\n",
        ),
        (
            "README.md",
            "# Intro\n\nSee [guide](docs/guide.md).\n\n## Usage\n\n```bash\necho ok\n```\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    let build = read_json_data_output("build", run_atlas(repo.path(), &["--json", "build"]));
    assert_eq!(build["parse_errors"], json!(0));
    assert!(build["parsed"].as_u64().unwrap_or_default() >= 4);

    let store = open_store(repo.path());
    let html_nodes = store
        .nodes_by_file("public/index.html")
        .expect("html nodes");
    assert!(
        html_nodes
            .iter()
            .any(|node| node.qualified_name == "public/index.html::document")
    );
    assert!(
        html_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "app.js")
    );

    let css_nodes = store.nodes_by_file("assets/app.css").expect("css nodes");
    assert!(
        css_nodes
            .iter()
            .any(|node| node.qualified_name == "assets/app.css::rule::1")
    );
    assert!(
        css_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Class && node.name == ".button")
    );

    let bash_nodes = store
        .nodes_by_file("scripts/deploy.sh")
        .expect("bash nodes");
    assert!(
        bash_nodes
            .iter()
            .any(|node| node.qualified_name == "scripts/deploy.sh::fn::setup")
    );
    let bash_edges = store
        .edges_by_file("scripts/deploy.sh")
        .expect("bash edges");
    assert!(bash_edges.iter().any(|edge| {
        edge.kind == EdgeKind::Calls && edge.target_qn == "scripts/deploy.sh::fn::helper"
    }));

    let markdown_nodes = store.nodes_by_file("README.md").expect("markdown nodes");
    assert!(
        markdown_nodes
            .iter()
            .any(|node| node.qualified_name == "README.md::heading::document.intro")
    );
    assert!(
        markdown_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "docs/guide.md")
    );

    write_repo_file(
        repo.path(),
        "public/index.html",
        "<!doctype html><html><body><script src=\"hero.js\"></script></body></html>\n",
    );
    write_repo_file(
        repo.path(),
        "assets/app.css",
        "@import url('theme.css');\n#app { background: black; }\n",
    );
    write_repo_file(
        repo.path(),
        "scripts/deploy.sh",
        "setup() {\n  helper\n  source ./inner.sh\n}\nhelper() {\n  echo ok\n}\n",
    );
    write_repo_file(
        repo.path(),
        "README.md",
        "# Intro\n\n## Advanced\n\nSee [reference](docs/reference.md).\n",
    );

    let update = read_json_data_output(
        "update",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "update",
                "--files",
                "public/index.html",
                "--files",
                "assets/app.css",
                "--files",
                "scripts/deploy.sh",
                "--files",
                "README.md",
            ],
        ),
    );
    assert_eq!(update["parse_errors"], json!(0));
    assert_eq!(update["parsed"], json!(4));

    let store = open_store(repo.path());
    let html_nodes = store
        .nodes_by_file("public/index.html")
        .expect("updated html nodes");
    assert!(
        html_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "hero.js"),
        "updated html nodes: {html_nodes:?}"
    );
    assert!(
        !html_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "app.js")
    );

    let css_nodes = store
        .nodes_by_file("assets/app.css")
        .expect("updated css nodes");
    assert!(
        css_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "theme.css")
    );
    assert!(css_nodes.iter().any(|node| node.name == "#app"));
    assert!(!css_nodes.iter().any(|node| node.name == ".button"));

    let bash_nodes = store
        .nodes_by_file("scripts/deploy.sh")
        .expect("updated bash nodes");
    assert!(
        bash_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "./inner.sh")
    );

    let markdown_nodes = store
        .nodes_by_file("README.md")
        .expect("updated markdown nodes");
    assert!(
        markdown_nodes
            .iter()
            .any(|node| node.qualified_name == "README.md::heading::document.intro.advanced")
    );
    assert!(
        markdown_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "docs/reference.md")
    );
    assert!(
        !markdown_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "docs/guide.md")
    );
}

#[test]
fn build_and_update_replace_compiled_language_file_graphs() {
    let repo = setup_repo(&[
        (
            "src/Main.java",
            "package demo.app;\nimport java.util.List;\nclass Main { void run() { helper(); } void helper() {} }\n",
        ),
        (
            "src/App.cs",
            "using System.Text;\nnamespace Demo.App;\nclass Runner { void Run() { Helper(); } void Helper() {} }\n",
        ),
        (
            "src/index.php",
            "<?php\nnamespace Demo\\App;\nuse Demo\\Support\\Helper;\nclass Runner { public function run() { helper(); } private function helper() {} }\n",
        ),
        (
            "src/native.c",
            "#include \"util.h\"\ntypedef unsigned long size_t;\nstatic void helper(void) {}\nvoid run(void) { helper(); }\n",
        ),
        (
            "src/native.cpp",
            "#include <vector>\nnamespace demo { template <typename T> class Box {}; class Runner { public: void helper() {} void run() { helper(); } }; }\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    let build = read_json_data_output("build", run_atlas(repo.path(), &["--json", "build"]));
    assert_eq!(build["parse_errors"], json!(0));
    assert!(build["parsed"].as_u64().unwrap_or_default() >= 5);

    let store = open_store(repo.path());
    let java_nodes = store.nodes_by_file("src/Main.java").expect("java nodes");
    assert!(
        java_nodes
            .iter()
            .any(|node| node.qualified_name == "src/Main.java::package::demo.app")
    );
    assert!(
        java_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "java.util.List")
    );
    let java_edges = store.edges_by_file("src/Main.java").expect("java edges");
    assert!(java_edges.iter().any(|edge| {
        edge.kind == EdgeKind::Calls && edge.target_qn == "src/Main.java::method::Main.helper"
    }));

    let csharp_nodes = store.nodes_by_file("src/App.cs").expect("csharp nodes");
    assert!(
        csharp_nodes
            .iter()
            .any(|node| node.qualified_name == "src/App.cs::namespace::Demo.App")
    );
    assert!(
        csharp_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "System.Text")
    );

    let php_nodes = store.nodes_by_file("src/index.php").expect("php nodes");
    assert!(
        php_nodes
            .iter()
            .any(|node| node.qualified_name == "src/index.php::namespace::Demo\\App")
    );
    assert!(
        php_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "Demo\\Support\\Helper")
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

    let cpp_nodes = store.nodes_by_file("src/native.cpp").expect("cpp nodes");
    assert!(
        cpp_nodes
            .iter()
            .any(|node| node.qualified_name == "src/native.cpp::namespace::demo")
    );
    assert!(
        cpp_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "vector")
    );
    let cpp_edges = store.edges_by_file("src/native.cpp").expect("cpp edges");
    assert!(cpp_edges.iter().any(|edge| {
        edge.kind == EdgeKind::Calls && edge.target_qn == "src/native.cpp::method::Runner.helper"
    }));

    write_repo_file(
        repo.path(),
        "src/Main.java",
        "package demo.app;\nimport java.util.Map;\nclass Main { void run() { assist(); } void assist() {} }\n",
    );
    write_repo_file(
        repo.path(),
        "src/App.cs",
        "using System.IO;\nnamespace Demo.App;\nclass Runner { void Run() { Assist(); } void Assist() {} }\n",
    );
    write_repo_file(
        repo.path(),
        "src/index.php",
        "<?php\nnamespace Demo\\App;\nuse Demo\\Support\\Other;\nclass Runner { public function run() { assist(); } private function assist() {} }\n",
    );
    write_repo_file(
        repo.path(),
        "src/native.c",
        "#include \"other.h\"\ntypedef unsigned long count_t;\nstatic void assist(void) {}\nvoid run(void) { assist(); }\n",
    );
    write_repo_file(
        repo.path(),
        "src/native.cpp",
        "#include <string>\nnamespace demo { class Runner { public: void assist() {} void run() { assist(); } }; }\n",
    );

    let update = read_json_data_output(
        "update",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "update",
                "--files",
                "src/Main.java",
                "--files",
                "src/App.cs",
                "--files",
                "src/index.php",
                "--files",
                "src/native.c",
                "--files",
                "src/native.cpp",
            ],
        ),
    );
    assert_eq!(update["parse_errors"], json!(0));
    assert_eq!(update["parsed"], json!(5));

    let store = open_store(repo.path());
    let java_nodes = store
        .nodes_by_file("src/Main.java")
        .expect("updated java nodes");
    assert!(
        java_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "java.util.Map")
    );
    assert!(
        java_nodes
            .iter()
            .any(|node| node.qualified_name == "src/Main.java::method::Main.assist")
    );
    assert!(
        !java_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "java.util.List")
    );

    let csharp_nodes = store
        .nodes_by_file("src/App.cs")
        .expect("updated csharp nodes");
    assert!(
        csharp_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "System.IO")
    );
    assert!(
        csharp_nodes
            .iter()
            .any(|node| node.qualified_name == "src/App.cs::method::Runner.Assist")
    );
    assert!(
        !csharp_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "System.Text")
    );

    let php_nodes = store
        .nodes_by_file("src/index.php")
        .expect("updated php nodes");
    assert!(
        php_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "Demo\\Support\\Other")
    );
    assert!(
        php_nodes
            .iter()
            .any(|node| node.qualified_name == "src/index.php::method::Runner.assist")
    );
    assert!(
        !php_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "Demo\\Support\\Helper")
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

    let cpp_nodes = store
        .nodes_by_file("src/native.cpp")
        .expect("updated cpp nodes");
    assert!(
        cpp_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "string")
    );
    assert!(
        cpp_nodes
            .iter()
            .any(|node| node.qualified_name == "src/native.cpp::method::Runner.assist")
    );
    assert!(
        !cpp_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "vector")
    );
}

#[test]
fn build_and_update_replace_scala_ruby_file_graphs() {
    let repo = setup_repo(&[
        (
            "src/Main.scala",
            "package demo.app\nimport demo.support.Helper\nobject Runner { def helper(): Unit = () ; def run(): Unit = helper() }\ncase class Box(value: Int)\n",
        ),
        (
            "lib/app.rb",
            "require \"json\"\nmodule Demo\n  class Runner\n    include Logging\n    def helper\n    end\n    def run\n      helper()\n    end\n  end\nend\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    let build = read_json_data_output("build", run_atlas(repo.path(), &["--json", "build"]));
    assert_eq!(build["parse_errors"], json!(0));
    assert!(build["parsed"].as_u64().unwrap_or_default() >= 2);

    let store = open_store(repo.path());
    let scala_nodes = store.nodes_by_file("src/Main.scala").expect("scala nodes");
    assert!(
        scala_nodes
            .iter()
            .any(|node| node.qualified_name == "src/Main.scala::package::demo.app")
    );
    assert!(
        scala_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "demo.support.Helper")
    );
    assert!(
        scala_nodes
            .iter()
            .any(|node| node.qualified_name == "src/Main.scala::case_class::Box")
    );
    let scala_edges = store.edges_by_file("src/Main.scala").expect("scala edges");
    assert!(scala_edges.iter().any(|edge| {
        edge.kind == EdgeKind::Calls && edge.target_qn == "src/Main.scala::method::Runner.helper"
    }));

    let ruby_nodes = store.nodes_by_file("lib/app.rb").expect("ruby nodes");
    assert!(
        ruby_nodes
            .iter()
            .any(|node| node.qualified_name == "lib/app.rb::module::Demo")
    );
    assert!(
        ruby_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "json")
    );
    assert!(
        ruby_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "Logging")
    );
    let ruby_edges = store.edges_by_file("lib/app.rb").expect("ruby edges");
    assert!(ruby_edges.iter().any(|edge| {
        edge.kind == EdgeKind::Calls && edge.target_qn == "lib/app.rb::method::Runner.helper"
    }));

    write_repo_file(
        repo.path(),
        "src/Main.scala",
        "package demo.app\nimport demo.support.Other\nobject Runner { def assist(): Unit = () ; def run(): Unit = assist() }\ncase class Crate(value: Int)\n",
    );
    write_repo_file(
        repo.path(),
        "lib/app.rb",
        "require \"yaml\"\nmodule Demo\n  class Runner\n    extend Builders\n    def assist\n    end\n    def run\n      assist()\n    end\n  end\nend\n",
    );

    let update = read_json_data_output(
        "update",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "update",
                "--files",
                "src/Main.scala",
                "--files",
                "lib/app.rb",
            ],
        ),
    );
    assert_eq!(update["parse_errors"], json!(0));
    assert_eq!(update["parsed"], json!(2));

    let store = open_store(repo.path());
    let scala_nodes = store
        .nodes_by_file("src/Main.scala")
        .expect("updated scala nodes");
    assert!(
        scala_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "demo.support.Other")
    );
    assert!(
        scala_nodes
            .iter()
            .any(|node| node.qualified_name == "src/Main.scala::method::Runner.assist")
    );
    assert!(
        scala_nodes
            .iter()
            .any(|node| node.qualified_name == "src/Main.scala::case_class::Crate")
    );
    assert!(
        !scala_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "demo.support.Helper")
    );
    assert!(
        !scala_nodes
            .iter()
            .any(|node| node.qualified_name == "src/Main.scala::case_class::Box")
    );

    let ruby_nodes = store
        .nodes_by_file("lib/app.rb")
        .expect("updated ruby nodes");
    assert!(
        ruby_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "yaml")
    );
    assert!(
        ruby_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "Builders")
    );
    assert!(
        ruby_nodes
            .iter()
            .any(|node| node.qualified_name == "lib/app.rb::method::Runner.assist")
    );
    assert!(
        !ruby_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "json")
    );
    assert!(
        !ruby_nodes
            .iter()
            .any(|node| node.kind == NodeKind::Import && node.name == "Logging")
    );
}
