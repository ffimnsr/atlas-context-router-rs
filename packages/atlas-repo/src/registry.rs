use std::collections::{BTreeMap, BTreeSet};
use std::fs;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::path::{canonical_filesystem_path, git_cmd, repo_relative, to_forward_slashes};
use crate::root::find_repo_root;

pub const REPO_REGISTRY_SCHEMA_VERSION: u32 = 1;
pub const REPO_REGISTRY_FILE_NAME: &str = "repo-registry.toml";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoRegistry {
    pub schema_version: u32,
    pub root_repo_id: String,
    #[serde(default)]
    pub registrations: Vec<RepoRegistration>,
    #[serde(default)]
    pub warnings: Vec<RepoRegistryWarning>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoRegistration {
    pub repo_id: String,
    pub root: Utf8PathBuf,
    pub display_alias: String,
    pub vcs: VcsMetadata,
    pub relationship: RepoRelationship,
    pub trust_state: TrustState,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_globs: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_globs: Option<Vec<String>>,
    #[serde(default)]
    pub dependencies: Vec<RepoDependency>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VcsMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_branch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoDependency {
    pub repo_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoRegistryWarning {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RepoRelationshipKind {
    Root,
    Submodule,
    WorkspaceMember,
    Manual,
}

pub fn phase1_multi_repo_supported(kind: RepoRelationshipKind) -> bool {
    matches!(
        kind,
        RepoRelationshipKind::Root | RepoRelationshipKind::Submodule
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoRelationship {
    pub kind: RepoRelationshipKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_repo_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrustState {
    Trusted,
    Untrusted,
    Missing,
    Stale,
    Unauthorized,
}

impl RepoRegistry {
    pub fn new(root_repo_id: String) -> Self {
        Self {
            schema_version: REPO_REGISTRY_SCHEMA_VERSION,
            root_repo_id,
            registrations: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub fn load(registry_root: &Utf8Path) -> Result<Self> {
        let path = registry_path(registry_root);
        let raw = fs::read_to_string(path.as_std_path())
            .with_context(|| format!("cannot read repo registry at {path}"))?;
        let registry: Self = toml::from_str(&raw).context("cannot parse repo registry TOML")?;
        anyhow::ensure!(
            registry.schema_version == REPO_REGISTRY_SCHEMA_VERSION,
            "unsupported repo registry schema version {}",
            registry.schema_version
        );
        Ok(registry)
    }

    pub fn load_or_bootstrap(registry_root: &Utf8Path) -> Result<Self> {
        if registry_path(registry_root).exists() {
            Self::load(registry_root)
        } else {
            bootstrap_registry(registry_root)
        }
    }

    pub fn save(&self, registry_root: &Utf8Path) -> Result<()> {
        let atlas_dir = registry_root.join(".atlas");
        fs::create_dir_all(atlas_dir.as_std_path())
            .with_context(|| format!("cannot create {atlas_dir}"))?;
        let encoded = toml::to_string_pretty(self).context("cannot encode repo registry TOML")?;
        let path = registry_path(registry_root);
        fs::write(path.as_std_path(), encoded)
            .with_context(|| format!("cannot write repo registry at {path}"))
    }

    pub fn upsert(&mut self, registration: RepoRegistration) {
        if let Some(existing) = self
            .registrations
            .iter_mut()
            .find(|entry| entry.repo_id == registration.repo_id)
        {
            *existing = registration;
        } else {
            self.registrations.push(registration);
        }
        self.sort_and_dedupe();
    }

    pub fn remove(&mut self, repo_id: &str) -> Option<RepoRegistration> {
        let index = self
            .registrations
            .iter()
            .position(|entry| entry.repo_id == repo_id)?;
        let removed = self.registrations.remove(index);
        for entry in &mut self.registrations {
            entry
                .dependencies
                .retain(|dependency| dependency.repo_id != repo_id);
        }
        Some(removed)
    }

    pub fn sync(&mut self, registry_root: &Utf8Path) -> Result<()> {
        self.warnings.clear();
        let mut next = Vec::with_capacity(self.registrations.len());
        for mut registration in std::mem::take(&mut self.registrations) {
            if registration.root.exists() {
                registration.trust_state = TrustState::Trusted;
                registration.vcs = read_vcs_metadata(registration.root.as_path());
            } else {
                registration.trust_state = TrustState::Missing;
                registration.enabled = false;
                self.warnings.push(RepoRegistryWarning {
                    code: "repo_missing".to_owned(),
                    message: format!("registered repo '{}' is missing", registration.repo_id),
                    path: Some(registration.root.to_string()),
                });
            }
            next.push(registration);
        }
        self.registrations = next;
        discover_initialized_submodules(registry_root, self)?;
        self.sort_and_dedupe();
        Ok(())
    }

    fn sort_and_dedupe(&mut self) {
        self.registrations.sort_by(|left, right| {
            (
                relationship_rank(left.relationship.kind),
                &left.display_alias,
                &left.repo_id,
            )
                .cmp(&(
                    relationship_rank(right.relationship.kind),
                    &right.display_alias,
                    &right.repo_id,
                ))
        });
        let mut seen = BTreeSet::new();
        self.registrations
            .retain(|entry| seen.insert(entry.repo_id.clone()));
    }
}

pub fn registry_path(registry_root: &Utf8Path) -> Utf8PathBuf {
    registry_root.join(".atlas").join(REPO_REGISTRY_FILE_NAME)
}

pub fn bootstrap_registry(registry_root: &Utf8Path) -> Result<RepoRegistry> {
    let root = canonical_filesystem_path(registry_root)
        .with_context(|| format!("cannot canonicalize registry root '{registry_root}'"))?;
    let root_repo_id = stable_repo_id(root.as_path());
    let mut registry = RepoRegistry::new(root_repo_id.clone());
    registry.upsert(registration_for_repo(
        root.as_path(),
        root.as_path(),
        RepoRelationship {
            kind: RepoRelationshipKind::Root,
            parent_repo_id: None,
            parent_path: None,
        },
        Some(".".to_owned()),
    )?);
    discover_initialized_submodules(root.as_path(), &mut registry)?;
    Ok(registry)
}

pub fn add_manual_repo(registry_root: &Utf8Path, repo_path: &Utf8Path) -> Result<RepoRegistration> {
    let input = if repo_path.is_absolute() {
        repo_path.to_path_buf()
    } else {
        registry_root.join(repo_path)
    };
    let repo_root = find_repo_root(input.as_path())
        .with_context(|| format!("cannot find git repo root for '{repo_path}'"))?;
    let root = canonical_filesystem_path(repo_root.as_path())
        .with_context(|| format!("cannot canonicalize repo root '{repo_root}'"))?;
    registration_for_repo(
        root.as_path(),
        registry_root,
        RepoRelationship {
            kind: RepoRelationshipKind::Manual,
            parent_repo_id: None,
            parent_path: None,
        },
        None,
    )
}

pub fn discover_initialized_submodules(
    registry_root: &Utf8Path,
    registry: &mut RepoRegistry,
) -> Result<()> {
    let output = git_cmd()
        .args(["submodule", "status", "--recursive"])
        .current_dir(registry_root)
        .output();
    let Ok(output) = output else {
        return Ok(());
    };
    if !output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut dependencies: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let root_repo_id = registry.root_repo_id.clone();
    for raw_line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let Some((state, commit, rel_path)) = parse_submodule_status_line(raw_line) else {
            continue;
        };
        if state == '-' {
            registry.warnings.push(RepoRegistryWarning {
                code: "submodule_uninitialized".to_owned(),
                message: format!("submodule '{rel_path}' is not initialized"),
                path: Some(rel_path.clone()),
            });
            continue;
        }
        if state == 'U' {
            registry.warnings.push(RepoRegistryWarning {
                code: "submodule_conflict".to_owned(),
                message: format!("submodule '{rel_path}' has unresolved conflicts"),
                path: Some(rel_path.clone()),
            });
            continue;
        }
        let absolute = registry_root.join(&rel_path);
        if !absolute.exists() {
            registry.warnings.push(RepoRegistryWarning {
                code: "submodule_missing".to_owned(),
                message: format!("submodule '{rel_path}' path is missing"),
                path: Some(rel_path.clone()),
            });
            continue;
        }
        let submodule_root = match canonical_filesystem_path(absolute.as_path()) {
            Ok(root) => root,
            Err(error) => {
                registry.warnings.push(RepoRegistryWarning {
                    code: "submodule_uncanonical".to_owned(),
                    message: format!("submodule '{rel_path}' cannot be canonicalized: {error}"),
                    path: Some(rel_path.clone()),
                });
                continue;
            }
        };
        let parent_repo_id = parent_repo_id_for_submodule(registry_root, registry, &rel_path)
            .unwrap_or_else(|| root_repo_id.clone());
        let mut registration = registration_for_repo(
            submodule_root.as_path(),
            registry_root,
            RepoRelationship {
                kind: RepoRelationshipKind::Submodule,
                parent_repo_id: Some(parent_repo_id.clone()),
                parent_path: Some(rel_path.clone()),
            },
            Some(rel_path.clone()),
        )?;
        if registration.vcs.head.is_none() && !commit.is_empty() {
            registration.vcs.head = Some(commit.to_owned());
        }
        dependencies
            .entry(parent_repo_id)
            .or_default()
            .insert(registration.repo_id.clone());
        registry.upsert(registration);
    }

    for (parent, children) in dependencies {
        if let Some(entry) = registry
            .registrations
            .iter_mut()
            .find(|entry| entry.repo_id == parent)
        {
            let existing: BTreeSet<String> = entry
                .dependencies
                .iter()
                .map(|dependency| dependency.repo_id.clone())
                .collect();
            for child in children.difference(&existing) {
                entry.dependencies.push(RepoDependency {
                    repo_id: child.clone(),
                    kind: Some("submodule".to_owned()),
                });
            }
            entry
                .dependencies
                .sort_by(|left, right| left.repo_id.cmp(&right.repo_id));
        }
    }
    Ok(())
}

pub fn registration_for_repo(
    repo_root: &Utf8Path,
    registry_root: &Utf8Path,
    relationship: RepoRelationship,
    display_alias: Option<String>,
) -> Result<RepoRegistration> {
    let root = canonical_filesystem_path(repo_root)
        .with_context(|| format!("cannot canonicalize repo root '{repo_root}'"))?;
    Ok(RepoRegistration {
        repo_id: stable_repo_id(root.as_path()),
        root,
        display_alias: display_alias.unwrap_or_else(|| display_alias_for(registry_root, repo_root)),
        vcs: read_vcs_metadata(repo_root),
        relationship,
        trust_state: TrustState::Trusted,
        enabled: true,
        include_globs: None,
        exclude_globs: None,
        dependencies: Vec::new(),
    })
}

pub fn stable_repo_id(repo_root: &Utf8Path) -> String {
    let normalized = to_forward_slashes(repo_root.as_str());
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    let digest = hasher.finalize();
    format!("repo_{:x}", digest)[..21].to_owned()
}

fn read_vcs_metadata(repo_root: &Utf8Path) -> VcsMetadata {
    VcsMetadata {
        head: git_output(repo_root, &["rev-parse", "HEAD"]),
        default_branch: default_branch(repo_root),
        remote_url: git_output(repo_root, &["config", "--get", "remote.origin.url"]),
    }
}

fn default_branch(repo_root: &Utf8Path) -> Option<String> {
    git_output(
        repo_root,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
    )
    .map(|branch| branch.strip_prefix("origin/").unwrap_or(&branch).to_owned())
    .or_else(|| git_output(repo_root, &["branch", "--show-current"]))
}

fn git_output(repo_root: &Utf8Path, args: &[&str]) -> Option<String> {
    let output = git_cmd().args(args).current_dir(repo_root).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (!value.is_empty()).then_some(value)
}

fn parse_submodule_status_line(line: &str) -> Option<(char, String, String)> {
    let trimmed = line.trim_start();
    let state = if line.starts_with(' ') {
        ' '
    } else {
        trimmed.chars().next()?
    };
    let without_state = if state == ' ' { trimmed } else { &trimmed[1..] }.trim_start();
    let mut parts = without_state.split_whitespace();
    let commit = parts.next()?.trim_start_matches(['+', '-']).to_owned();
    let rel_path = parts.next()?.to_owned();
    Some((state, commit, rel_path))
}

fn parent_repo_id_for_submodule(
    registry_root: &Utf8Path,
    registry: &RepoRegistry,
    rel_path: &str,
) -> Option<String> {
    let path = Utf8Path::new(rel_path);
    let parent_path = path.parent().unwrap_or_else(|| Utf8Path::new(""));
    registry
        .registrations
        .iter()
        .filter(|entry| entry.relationship.kind == RepoRelationshipKind::Submodule)
        .filter_map(|entry| {
            let alias = Utf8Path::new(&entry.display_alias);
            parent_path
                .strip_prefix(alias)
                .ok()
                .map(|suffix| (suffix.components().count(), entry.repo_id.clone()))
        })
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, repo_id)| repo_id)
        .or_else(|| {
            repo_relative(registry_root, registry_root.join(parent_path).as_path())
                .ok()
                .and_then(|parent| {
                    registry
                        .registrations
                        .iter()
                        .find(|entry| entry.display_alias == parent.as_str())
                        .map(|entry| entry.repo_id.clone())
                })
        })
}

fn display_alias_for(registry_root: &Utf8Path, repo_root: &Utf8Path) -> String {
    if let Ok(relative) = repo_relative(registry_root, repo_root) {
        return relative.to_string();
    }
    repo_root
        .file_name()
        .map(str::to_owned)
        .unwrap_or_else(|| repo_root.to_string())
}

fn relationship_rank(kind: RepoRelationshipKind) -> u8 {
    match kind {
        RepoRelationshipKind::Root => 0,
        RepoRelationshipKind::Submodule => 1,
        RepoRelationshipKind::WorkspaceMember => 2,
        RepoRelationshipKind::Manual => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    fn git(dir: &std::path::Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Atlas Test")
            .env("GIT_AUTHOR_EMAIL", "test@atlas")
            .env("GIT_COMMITTER_NAME", "Atlas Test")
            .env("GIT_COMMITTER_EMAIL", "test@atlas")
            .status()
            .expect("git command");
        assert!(status.success(), "git {args:?} failed");
    }

    fn init_git_repo(path: &std::path::Path) {
        fs::create_dir_all(path).unwrap();
        git(path, &["init", "--quiet"]);
        git(path, &["config", "user.name", "Atlas Tests"]);
        git(path, &["config", "user.email", "atlas-tests@example.com"]);
    }

    #[test]
    fn stable_repo_id_uses_canonical_root_text() {
        let first = stable_repo_id(Utf8Path::new("/tmp/example"));
        let second = stable_repo_id(Utf8Path::new("/tmp/example"));

        assert_eq!(first, second);
        assert!(first.starts_with("repo_"));
    }

    #[test]
    fn registry_round_trips_human_editable_toml() {
        let temp = TempDir::new().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap();
        let registration = registration_for_repo(
            root,
            root,
            RepoRelationship {
                kind: RepoRelationshipKind::Root,
                parent_repo_id: None,
                parent_path: None,
            },
            Some(".".to_owned()),
        )
        .unwrap();
        let mut registry = RepoRegistry::new(registration.repo_id.clone());
        registry.upsert(registration);
        registry.save(root).unwrap();

        let encoded = fs::read_to_string(registry_path(root).as_std_path()).unwrap();
        assert!(encoded.contains("schema_version = 1"));
        assert!(encoded.contains("display_alias = \".\""));

        let loaded = RepoRegistry::load(root).unwrap();
        assert_eq!(loaded.schema_version, REPO_REGISTRY_SCHEMA_VERSION);
        assert_eq!(loaded.registrations.len(), 1);
    }

    #[test]
    fn remove_drops_registration_and_dependencies_only() {
        let mut registry = RepoRegistry::new("root".to_owned());
        registry.registrations.push(RepoRegistration {
            repo_id: "root".to_owned(),
            root: Utf8PathBuf::from("/tmp/root"),
            display_alias: ".".to_owned(),
            vcs: VcsMetadata {
                head: None,
                default_branch: None,
                remote_url: None,
            },
            relationship: RepoRelationship {
                kind: RepoRelationshipKind::Root,
                parent_repo_id: None,
                parent_path: None,
            },
            trust_state: TrustState::Trusted,
            enabled: true,
            include_globs: None,
            exclude_globs: None,
            dependencies: vec![RepoDependency {
                repo_id: "child".to_owned(),
                kind: Some("submodule".to_owned()),
            }],
        });
        registry.registrations.push(RepoRegistration {
            repo_id: "child".to_owned(),
            root: Utf8PathBuf::from("/tmp/root/vendor/child"),
            display_alias: "vendor/child".to_owned(),
            vcs: VcsMetadata {
                head: None,
                default_branch: None,
                remote_url: None,
            },
            relationship: RepoRelationship {
                kind: RepoRelationshipKind::Submodule,
                parent_repo_id: Some("root".to_owned()),
                parent_path: Some("vendor/child".to_owned()),
            },
            trust_state: TrustState::Trusted,
            enabled: true,
            include_globs: None,
            exclude_globs: None,
            dependencies: Vec::new(),
        });

        let removed = registry.remove("child");

        assert!(removed.is_some());
        assert_eq!(registry.registrations.len(), 1);
        assert!(registry.registrations[0].dependencies.is_empty());
    }

    #[test]
    fn phase1_multi_repo_support_only_includes_root_and_submodule() {
        assert!(phase1_multi_repo_supported(RepoRelationshipKind::Root));
        assert!(phase1_multi_repo_supported(RepoRelationshipKind::Submodule));
        assert!(!phase1_multi_repo_supported(
            RepoRelationshipKind::WorkspaceMember
        ));
        assert!(!phase1_multi_repo_supported(RepoRelationshipKind::Manual));
    }

    #[test]
    fn bootstrap_registry_auto_registers_initialized_submodule() {
        let root_dir = TempDir::new().unwrap();
        let root = root_dir.path();
        init_git_repo(root);
        fs::write(root.join("README.md"), "root\n").unwrap();
        git(root, &["add", "README.md"]);
        git(root, &["commit", "--quiet", "-m", "root"]);

        let sub_dir = TempDir::new().unwrap();
        let sub = sub_dir.path();
        init_git_repo(sub);
        fs::write(sub.join("lib.rs"), "pub fn dep_helper() {}\n").unwrap();
        git(sub, &["add", "lib.rs"]);
        git(sub, &["commit", "--quiet", "-m", "sub"]);

        git(
            root,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                sub.to_str().unwrap(),
                "vendor/dep",
            ],
        );
        git(root, &["commit", "--quiet", "-am", "add submodule"]);

        let registry = bootstrap_registry(Utf8Path::from_path(root).unwrap()).unwrap();

        assert_eq!(registry.registrations.len(), 2);
        let root_entry = registry
            .registrations
            .iter()
            .find(|entry| entry.relationship.kind == RepoRelationshipKind::Root)
            .unwrap();
        let sub_entry = registry
            .registrations
            .iter()
            .find(|entry| entry.relationship.kind == RepoRelationshipKind::Submodule)
            .unwrap();
        assert_eq!(sub_entry.display_alias, "vendor/dep");
        assert_eq!(
            sub_entry.relationship.parent_repo_id.as_deref(),
            Some(root_entry.repo_id.as_str())
        );
        assert_eq!(
            sub_entry.relationship.parent_path.as_deref(),
            Some("vendor/dep")
        );
        assert!(
            root_entry
                .dependencies
                .iter()
                .any(|dependency| dependency.repo_id == sub_entry.repo_id)
        );
    }

    #[test]
    fn add_manual_repo_registers_sibling_repo() {
        let root_dir = TempDir::new().unwrap();
        let root = root_dir.path();
        init_git_repo(root);
        fs::write(root.join("README.md"), "root\n").unwrap();
        git(root, &["add", "README.md"]);
        git(root, &["commit", "--quiet", "-m", "root"]);

        let sibling_dir = TempDir::new().unwrap();
        let sibling = sibling_dir.path();
        init_git_repo(sibling);
        fs::create_dir_all(sibling.join("src")).unwrap();
        fs::write(sibling.join("src/lib.rs"), "pub fn sibling() {}\n").unwrap();
        git(sibling, &["add", "."]);
        git(sibling, &["commit", "--quiet", "-m", "sibling"]);

        let registration = add_manual_repo(
            Utf8Path::from_path(root).unwrap(),
            Utf8Path::from_path(sibling).unwrap(),
        )
        .unwrap();

        assert_eq!(registration.relationship.kind, RepoRelationshipKind::Manual);
        assert_eq!(
            registration.display_alias,
            sibling.file_name().unwrap().to_string_lossy()
        );
        assert_eq!(
            registration.repo_id,
            stable_repo_id(Utf8Path::from_path(sibling).unwrap())
        );
    }

    #[test]
    fn bootstrap_registry_keeps_repo_id_stable_across_rebuilds() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        init_git_repo(root);
        fs::write(root.join("src.rs"), "pub fn a() {}\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "--quiet", "-m", "first"]);

        let first = bootstrap_registry(Utf8Path::from_path(root).unwrap()).unwrap();

        fs::write(root.join("src.rs"), "pub fn a() {}\npub fn b() {}\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "--quiet", "-m", "second"]);

        let second = bootstrap_registry(Utf8Path::from_path(root).unwrap()).unwrap();

        assert_eq!(first.root_repo_id, second.root_repo_id);
        assert_eq!(
            first.registrations[0].repo_id,
            second.registrations[0].repo_id
        );
    }
}
