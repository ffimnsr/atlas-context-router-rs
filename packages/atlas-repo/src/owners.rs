use anyhow::{Context, Result};
use atlas_core::{PackageOwner, PackageOwnerKind};
use camino::{Utf8Path, Utf8PathBuf};

use crate::{collect_files, glob_match};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRoot {
    pub workspace_id: String,
    pub kind: PackageOwnerKind,
    pub root: String,
    pub manifest_path: String,
    pub member_roots: Vec<String>,
    pub member_owner_ids: Vec<String>,
}

#[derive(Debug, Clone)]
struct WorkspaceDefinition {
    kind: PackageOwnerKind,
    manifest_path: String,
    member_manifest_paths: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct PackageOwners {
    owners: Vec<PackageOwner>,
    workspaces: Vec<WorkspaceRoot>,
}

impl PackageOwners {
    pub fn all(&self) -> &[PackageOwner] {
        &self.owners
    }

    pub fn workspaces(&self) -> &[WorkspaceRoot] {
        &self.workspaces
    }

    pub fn owner_for_path(&self, path: &str) -> Option<&PackageOwner> {
        self.owners
            .iter()
            .filter(|owner| path_is_within_owner(path, &owner.root))
            .max_by(|left, right| {
                left.root
                    .len()
                    .cmp(&right.root.len())
                    .then_with(|| left.manifest_path.cmp(&right.manifest_path))
            })
    }
}

pub fn discover_package_owners(repo_root: &Utf8Path) -> Result<PackageOwners> {
    let tracked = collect_files(repo_root, None).context("cannot collect tracked files")?;
    let cargo_manifests = tracked_manifests(&tracked, "Cargo.toml");
    let npm_manifests = tracked_manifests(&tracked, "package.json");
    let go_manifests = tracked_manifests(&tracked, "go.mod");
    let mut owners = Vec::new();
    let mut workspace_defs = Vec::new();

    if let Some(root_cargo) = cargo_manifests
        .iter()
        .find(|path| path.as_str() == "Cargo.toml")
        && let Some(workspace) = parse_cargo_workspace(repo_root, root_cargo, &cargo_manifests)?
    {
        workspace_defs.push(workspace);
    }

    if let Some(root_package_json) = npm_manifests
        .iter()
        .find(|path| path.as_str() == "package.json")
        && let Some(workspace) = parse_npm_workspace(repo_root, root_package_json, &npm_manifests)?
    {
        workspace_defs.push(workspace);
    }

    let root_go_work = Utf8Path::new("go.work");
    if tracked.iter().any(|path| path == root_go_work)
        && let Some(workspace) = parse_go_workspace(repo_root, root_go_work, &go_manifests)?
    {
        workspace_defs.push(workspace);
    }

    for rel_path in tracked {
        let Some(file_name) = rel_path.file_name() else {
            continue;
        };
        let owner = match file_name {
            "Cargo.toml" => parse_cargo_owner(repo_root, &rel_path)?,
            "package.json" => parse_npm_owner(repo_root, &rel_path)?,
            "go.mod" => parse_go_owner(repo_root, &rel_path)?,
            _ => None,
        };
        if let Some(owner) = owner {
            owners.push(owner);
        }
    }

    owners.sort_by(|left, right| {
        left.root
            .len()
            .cmp(&right.root.len())
            .reverse()
            .then_with(|| left.manifest_path.cmp(&right.manifest_path))
    });
    owners.dedup_by(|left, right| left.owner_id == right.owner_id);

    let mut workspaces: Vec<WorkspaceRoot> = workspace_defs
        .into_iter()
        .map(|workspace| make_workspace(workspace, &owners))
        .filter(|workspace| !workspace.member_owner_ids.is_empty())
        .collect();
    workspaces.sort_by(|left, right| left.manifest_path.cmp(&right.manifest_path));
    workspaces.dedup_by(|left, right| left.workspace_id == right.workspace_id);

    Ok(PackageOwners { owners, workspaces })
}

fn tracked_manifests(tracked: &[Utf8PathBuf], file_name: &str) -> Vec<Utf8PathBuf> {
    tracked
        .iter()
        .filter(|path| path.file_name() == Some(file_name))
        .cloned()
        .collect()
}

fn parse_cargo_workspace(
    repo_root: &Utf8Path,
    manifest_path: &Utf8Path,
    cargo_manifests: &[Utf8PathBuf],
) -> Result<Option<WorkspaceDefinition>> {
    let contents = std::fs::read_to_string(repo_root.join(manifest_path).as_std_path())
        .with_context(|| format!("cannot read {}", manifest_path))?;
    let value = contents
        .parse::<toml::Value>()
        .with_context(|| format!("cannot parse {}", manifest_path))?;
    let Some(workspace) = value.get("workspace").and_then(|value| value.as_table()) else {
        return Ok(None);
    };
    let member_patterns = workspace_patterns_from_toml(workspace.get("members"));
    let exclude_patterns = workspace_patterns_from_toml(workspace.get("exclude"));
    let member_manifest_paths =
        resolve_member_manifest_paths(&member_patterns, &exclude_patterns, cargo_manifests);

    Ok(Some(WorkspaceDefinition {
        kind: PackageOwnerKind::Cargo,
        manifest_path: manifest_path.to_string(),
        member_manifest_paths,
    }))
}

fn parse_npm_workspace(
    repo_root: &Utf8Path,
    manifest_path: &Utf8Path,
    npm_manifests: &[Utf8PathBuf],
) -> Result<Option<WorkspaceDefinition>> {
    let contents = std::fs::read_to_string(repo_root.join(manifest_path).as_std_path())
        .with_context(|| format!("cannot read {}", manifest_path))?;
    let value: serde_json::Value = serde_json::from_str(&contents)
        .with_context(|| format!("cannot parse {}", manifest_path))?;
    let member_patterns = workspace_patterns_from_json(value.get("workspaces"));
    if member_patterns.is_empty() {
        return Ok(None);
    }

    let member_manifest_paths = resolve_member_manifest_paths(&member_patterns, &[], npm_manifests);

    Ok(Some(WorkspaceDefinition {
        kind: PackageOwnerKind::Npm,
        manifest_path: manifest_path.to_string(),
        member_manifest_paths,
    }))
}

fn parse_go_workspace(
    repo_root: &Utf8Path,
    manifest_path: &Utf8Path,
    go_manifests: &[Utf8PathBuf],
) -> Result<Option<WorkspaceDefinition>> {
    let contents = std::fs::read_to_string(repo_root.join(manifest_path).as_std_path())
        .with_context(|| format!("cannot read {}", manifest_path))?;
    let member_patterns = parse_go_work_use_entries(&contents);
    if member_patterns.is_empty() {
        return Ok(None);
    }

    let member_manifest_paths = resolve_member_manifest_paths(&member_patterns, &[], go_manifests);

    Ok(Some(WorkspaceDefinition {
        kind: PackageOwnerKind::Go,
        manifest_path: manifest_path.to_string(),
        member_manifest_paths,
    }))
}

fn workspace_patterns_from_toml(value: Option<&toml::Value>) -> Vec<String> {
    value
        .and_then(|value| value.as_array())
        .into_iter()
        .flatten()
        .filter_map(|value| value.as_str())
        .map(normalize_workspace_pattern)
        .filter(|pattern| !pattern.is_empty())
        .collect()
}

fn workspace_patterns_from_json(value: Option<&serde_json::Value>) -> Vec<String> {
    match value {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str())
            .map(normalize_workspace_pattern)
            .filter(|pattern| !pattern.is_empty())
            .collect(),
        Some(serde_json::Value::Object(object)) => object
            .get("packages")
            .and_then(|value| value.as_array())
            .into_iter()
            .flatten()
            .filter_map(|item| item.as_str())
            .map(normalize_workspace_pattern)
            .filter(|pattern| !pattern.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_go_work_use_entries(contents: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut in_block = false;

    for line in contents.lines() {
        let trimmed = line.split("//").next().unwrap_or("").trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "use (" {
            in_block = true;
            continue;
        }
        if in_block && trimmed == ")" {
            in_block = false;
            continue;
        }
        if let Some(path) = trimmed.strip_prefix("use ") {
            let entry = normalize_workspace_pattern(path);
            if !entry.is_empty() {
                entries.push(entry);
            }
            continue;
        }
        if in_block {
            let entry = normalize_workspace_pattern(trimmed);
            if !entry.is_empty() {
                entries.push(entry);
            }
        }
    }

    entries
}

fn resolve_member_manifest_paths(
    include_patterns: &[String],
    exclude_patterns: &[String],
    manifests: &[Utf8PathBuf],
) -> Vec<String> {
    manifests
        .iter()
        .filter(|manifest_path| {
            let root = manifest_root(manifest_path);
            include_patterns
                .iter()
                .any(|pattern| workspace_pattern_matches(pattern, root))
                && !exclude_patterns
                    .iter()
                    .any(|pattern| workspace_pattern_matches(pattern, root))
        })
        .map(|manifest_path| manifest_path.to_string())
        .collect()
}

fn normalize_workspace_pattern(pattern: &str) -> String {
    let trimmed = pattern.trim().trim_matches('"');
    let trimmed = trimmed.strip_prefix("./").unwrap_or(trimmed);
    trimmed.trim_end_matches('/').to_string()
}

fn workspace_pattern_matches(pattern: &str, candidate_root: &str) -> bool {
    glob_match(pattern, candidate_root)
        || glob_match(&format!("{pattern}/"), &format!("{candidate_root}/"))
}

fn manifest_root(manifest_path: &Utf8Path) -> &str {
    manifest_path
        .parent()
        .unwrap_or_else(|| Utf8Path::new(""))
        .as_str()
}

fn make_workspace(workspace: WorkspaceDefinition, owners: &[PackageOwner]) -> WorkspaceRoot {
    let mut member_roots = Vec::new();
    let mut member_owner_ids = Vec::new();

    for manifest_path in workspace.member_manifest_paths {
        if let Some(owner) = owners
            .iter()
            .find(|owner| owner.manifest_path == manifest_path)
        {
            member_roots.push(owner.root.clone());
            member_owner_ids.push(owner.owner_id.clone());
        }
    }

    WorkspaceRoot {
        workspace_id: format!(
            "workspace:{}:{}",
            workspace.kind.as_str(),
            workspace.manifest_path
        ),
        kind: workspace.kind,
        root: manifest_root(Utf8Path::new(&workspace.manifest_path)).to_string(),
        manifest_path: workspace.manifest_path,
        member_roots,
        member_owner_ids,
    }
}

fn parse_cargo_owner(
    repo_root: &Utf8Path,
    manifest_path: &Utf8Path,
) -> Result<Option<PackageOwner>> {
    let contents = std::fs::read_to_string(repo_root.join(manifest_path).as_std_path())
        .with_context(|| format!("cannot read {}", manifest_path))?;
    let value = contents
        .parse::<toml::Value>()
        .with_context(|| format!("cannot parse {}", manifest_path))?;
    let Some(package) = value.get("package").and_then(|value| value.as_table()) else {
        return Ok(None);
    };
    let package_name = package
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::to_owned);
    Ok(Some(make_owner(
        PackageOwnerKind::Cargo,
        manifest_path,
        package_name,
    )))
}

fn parse_npm_owner(repo_root: &Utf8Path, manifest_path: &Utf8Path) -> Result<Option<PackageOwner>> {
    let contents = std::fs::read_to_string(repo_root.join(manifest_path).as_std_path())
        .with_context(|| format!("cannot read {}", manifest_path))?;
    let value: serde_json::Value = serde_json::from_str(&contents)
        .with_context(|| format!("cannot parse {}", manifest_path))?;
    let Some(object) = value.as_object() else {
        return Ok(None);
    };

    let has_package_signal = [
        "name",
        "version",
        "private",
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "optionalDependencies",
        "scripts",
        "main",
        "module",
        "exports",
        "bin",
    ]
    .iter()
    .any(|key| object.contains_key(*key));

    if !has_package_signal {
        return Ok(None);
    }

    let package_name = object
        .get("name")
        .and_then(|value| value.as_str())
        .map(str::to_owned);
    Ok(Some(make_owner(
        PackageOwnerKind::Npm,
        manifest_path,
        package_name,
    )))
}

fn parse_go_owner(repo_root: &Utf8Path, manifest_path: &Utf8Path) -> Result<Option<PackageOwner>> {
    let contents = std::fs::read_to_string(repo_root.join(manifest_path).as_std_path())
        .with_context(|| format!("cannot read {}", manifest_path))?;
    let package_name = contents.lines().find_map(|line| {
        line.trim()
            .strip_prefix("module ")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    });
    Ok(Some(make_owner(
        PackageOwnerKind::Go,
        manifest_path,
        package_name,
    )))
}

fn make_owner(
    kind: PackageOwnerKind,
    manifest_path: &Utf8Path,
    package_name: Option<String>,
) -> PackageOwner {
    let root = manifest_path
        .parent()
        .unwrap_or_else(|| Utf8Path::new(""))
        .to_string();
    PackageOwner {
        owner_id: format!("{}:{}", kind.as_str(), manifest_path),
        kind,
        root,
        manifest_path: manifest_path.to_string(),
        package_name,
    }
}

fn path_is_within_owner(path: &str, root: &str) -> bool {
    if root.is_empty() {
        return true;
    }
    path == root || path.starts_with(&format!("{root}/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn setup_repo(files: &[(&str, &str)]) -> tempfile::TempDir {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        for (relative_path, content) in files {
            let path = temp_dir.path().join(relative_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create dir");
            }
            fs::write(path, content).expect("write file");
        }
        std::process::Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(temp_dir.path())
            .output()
            .expect("git init");
        std::process::Command::new("git")
            .args(["config", "user.name", "Atlas Tests"])
            .current_dir(temp_dir.path())
            .output()
            .expect("git config name");
        std::process::Command::new("git")
            .args(["config", "user.email", "atlas-tests@example.com"])
            .current_dir(temp_dir.path())
            .output()
            .expect("git config email");
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(temp_dir.path())
            .output()
            .expect("git add");
        std::process::Command::new("git")
            .args(["commit", "--quiet", "-m", "fixture"])
            .current_dir(temp_dir.path())
            .output()
            .expect("git commit");
        temp_dir
    }

    #[test]
    fn discovers_standalone_cargo_package_roots() {
        let repo = setup_repo(&[
            (
                "crates/foo/Cargo.toml",
                "[package]\nname='foo'\nversion='0.1.0'\n",
            ),
            ("crates/foo/src/lib.rs", "pub fn foo() {}\n"),
            (
                "libs/bar/Cargo.toml",
                "[package]\nname='bar'\nversion='0.1.0'\n",
            ),
            ("libs/bar/src/lib.rs", "pub fn bar() {}\n"),
        ]);
        let repo_root = Utf8Path::from_path(repo.path()).expect("utf8 repo path");

        let owners = discover_package_owners(repo_root).expect("owners");

        assert_eq!(
            owners
                .owner_for_path("crates/foo/src/lib.rs")
                .expect("foo owner")
                .owner_id,
            "cargo:crates/foo/Cargo.toml"
        );
        assert_eq!(
            owners
                .owner_for_path("libs/bar/src/lib.rs")
                .expect("bar owner")
                .owner_id,
            "cargo:libs/bar/Cargo.toml"
        );
    }

    #[test]
    fn nearest_owner_wins_for_nested_package_roots() {
        let repo = setup_repo(&[
            ("Cargo.toml", "[package]\nname='root'\nversion='0.1.0'\n"),
            ("src/lib.rs", "pub fn root() {}\n"),
            (
                "tools/gen/Cargo.toml",
                "[package]\nname='gen'\nversion='0.1.0'\n",
            ),
            ("tools/gen/src/main.rs", "fn main() {}\n"),
        ]);
        let repo_root = Utf8Path::from_path(repo.path()).expect("utf8 repo path");

        let owners = discover_package_owners(repo_root).expect("owners");

        assert_eq!(
            owners
                .owner_for_path("tools/gen/src/main.rs")
                .expect("nested owner")
                .owner_id,
            "cargo:tools/gen/Cargo.toml"
        );
        // Root package still owns files not under the nested root.
        assert_eq!(
            owners
                .owner_for_path("src/lib.rs")
                .expect("root owner")
                .owner_id,
            "cargo:Cargo.toml"
        );
    }

    #[test]
    fn discovers_standalone_npm_package_roots() {
        let repo = setup_repo(&[
            (
                "apps/web/package.json",
                r#"{"name":"web","version":"0.1.0","scripts":{}}"#,
            ),
            ("apps/web/index.js", "export default {};\n"),
            (
                "packages/ui/package.json",
                r#"{"name":"ui","version":"0.1.0","main":"index.js"}"#,
            ),
            ("packages/ui/index.js", "export default {};\n"),
        ]);
        let repo_root = Utf8Path::from_path(repo.path()).expect("utf8 repo path");

        let owners = discover_package_owners(repo_root).expect("owners");

        assert_eq!(
            owners
                .owner_for_path("apps/web/index.js")
                .expect("web owner")
                .owner_id,
            "npm:apps/web/package.json"
        );
        assert_eq!(
            owners
                .owner_for_path("packages/ui/index.js")
                .expect("ui owner")
                .owner_id,
            "npm:packages/ui/package.json"
        );
    }

    #[test]
    fn discovers_standalone_go_module_roots() {
        let repo = setup_repo(&[
            ("cmd/tool/go.mod", "module example.com/tool\n\ngo 1.22\n"),
            ("cmd/tool/main.go", "package main\n"),
            ("lib/core/go.mod", "module example.com/core\n\ngo 1.22\n"),
            ("lib/core/core.go", "package core\n"),
        ]);
        let repo_root = Utf8Path::from_path(repo.path()).expect("utf8 repo path");

        let owners = discover_package_owners(repo_root).expect("owners");

        assert_eq!(
            owners
                .owner_for_path("cmd/tool/main.go")
                .expect("tool owner")
                .owner_id,
            "go:cmd/tool/go.mod"
        );
        assert_eq!(
            owners
                .owner_for_path("lib/core/core.go")
                .expect("core owner")
                .owner_id,
            "go:lib/core/go.mod"
        );
    }

    #[test]
    fn files_outside_any_package_root_return_none() {
        let repo = setup_repo(&[
            (
                "packages/foo/Cargo.toml",
                "[package]\nname='foo'\nversion='0.1.0'\n",
            ),
            ("packages/foo/src/lib.rs", "pub fn foo() {}\n"),
            ("scripts/build.sh", "#!/bin/sh\n"),
            ("docs/README.md", "# Docs\n"),
        ]);
        let repo_root = Utf8Path::from_path(repo.path()).expect("utf8 repo path");

        let owners = discover_package_owners(repo_root).expect("owners");

        assert!(owners.owner_for_path("scripts/build.sh").is_none());
        assert!(owners.owner_for_path("docs/README.md").is_none());
        // Still resolves for the package itself.
        assert!(owners.owner_for_path("packages/foo/src/lib.rs").is_some());
    }

    #[test]
    fn workspace_only_cargo_toml_is_not_an_owner() {
        // A root Cargo.toml with [workspace] but no [package] should not
        // be treated as a standalone package root.
        let repo = setup_repo(&[
            ("Cargo.toml", "[workspace]\nmembers=[\"crates/foo\"]\n"),
            (
                "crates/foo/Cargo.toml",
                "[package]\nname='foo'\nversion='0.1.0'\n",
            ),
            ("crates/foo/src/lib.rs", "pub fn foo() {}\n"),
        ]);
        let repo_root = Utf8Path::from_path(repo.path()).expect("utf8 repo path");

        let owners = discover_package_owners(repo_root).expect("owners");

        // Root Cargo.toml has no [package] → no owner at repo root level.
        assert_eq!(owners.all().len(), 1);
        assert_eq!(owners.all()[0].owner_id, "cargo:crates/foo/Cargo.toml");
    }

    #[test]
    fn cargo_workspace_members_globs_resolve_to_member_roots() {
        let repo = setup_repo(&[
            (
                "Cargo.toml",
                "[workspace]\nmembers = ['packages/*', 'tools/gen']\nexclude = ['packages/skip']\n",
            ),
            (
                "packages/foo/Cargo.toml",
                "[package]\nname='foo'\nversion='0.1.0'\n",
            ),
            ("packages/foo/src/lib.rs", "pub fn foo() {}\n"),
            (
                "packages/skip/Cargo.toml",
                "[package]\nname='skip'\nversion='0.1.0'\n",
            ),
            ("packages/skip/src/lib.rs", "pub fn skip() {}\n"),
            (
                "tools/gen/Cargo.toml",
                "[package]\nname='gen'\nversion='0.1.0'\n",
            ),
            ("tools/gen/src/main.rs", "fn main() {}\n"),
        ]);
        let repo_root = Utf8Path::from_path(repo.path()).expect("utf8 repo path");

        let owners = discover_package_owners(repo_root).expect("owners");

        assert_eq!(owners.workspaces().len(), 1);
        assert_eq!(
            owners.workspaces()[0].member_owner_ids,
            vec![
                "cargo:packages/foo/Cargo.toml".to_string(),
                "cargo:tools/gen/Cargo.toml".to_string(),
            ]
        );
        assert!(
            owners.workspaces()[0]
                .member_owner_ids
                .iter()
                .all(|owner_id| owner_id != "cargo:packages/skip/Cargo.toml")
        );
    }

    #[test]
    fn npm_workspaces_resolve_object_packages_entries() {
        let repo = setup_repo(&[
            (
                "package.json",
                r#"{"private":true,"workspaces":{"packages":["apps/*","packages/*"]}}"#,
            ),
            (
                "apps/web/package.json",
                r#"{"name":"web","version":"0.1.0","main":"index.js"}"#,
            ),
            ("apps/web/index.js", "export default {};\n"),
            (
                "packages/ui/package.json",
                r#"{"name":"ui","version":"0.1.0","exports":"./index.js"}"#,
            ),
            ("packages/ui/index.js", "export default {};\n"),
        ]);
        let repo_root = Utf8Path::from_path(repo.path()).expect("utf8 repo path");

        let owners = discover_package_owners(repo_root).expect("owners");

        assert_eq!(owners.workspaces().len(), 1);
        assert_eq!(
            owners.workspaces()[0].member_owner_ids,
            vec![
                "npm:apps/web/package.json".to_string(),
                "npm:packages/ui/package.json".to_string(),
            ]
        );
    }

    #[test]
    fn go_work_resolves_multiple_module_roots() {
        let repo = setup_repo(&[
            (
                "go.work",
                "go 1.22\n\nuse (\n    ./services/api\n    ./libs/shared\n)\n",
            ),
            (
                "services/api/go.mod",
                "module example.com/services/api\n\ngo 1.22\n",
            ),
            ("services/api/main.go", "package main\n"),
            (
                "libs/shared/go.mod",
                "module example.com/libs/shared\n\ngo 1.22\n",
            ),
            ("libs/shared/shared.go", "package shared\n"),
        ]);
        let repo_root = Utf8Path::from_path(repo.path()).expect("utf8 repo path");

        let owners = discover_package_owners(repo_root).expect("owners");

        assert_eq!(owners.workspaces().len(), 1);
        assert_eq!(
            owners.workspaces()[0].member_owner_ids,
            vec![
                "go:libs/shared/go.mod".to_string(),
                "go:services/api/go.mod".to_string(),
            ]
        );
    }
}
