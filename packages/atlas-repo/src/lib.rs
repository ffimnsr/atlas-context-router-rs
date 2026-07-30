//! Repository discovery and path identity utilities for Atlas.
//!
//! This crate keeps repo-relative file handling deterministic across graph
//! build, history, persistence, and retrieval flows.
//!
//! Primary surfaces:
//! - canonical path conversion via [`CanonicalRepoPath`]
//! - repository root discovery and repo-relative conversion
//! - supported-file collection and ignore handling
//! - changed-file detection and content hashing

mod diff;
mod files;
mod hash;
mod owners;
mod path;
mod registry;
mod root;

pub use diff::{DiffTarget, changed_files};
pub use files::{
    CollectFilesStats, DEFAULT_IGNORE_PATTERNS, DEFAULT_MAX_FILE_BYTES, collect_files,
    collect_supported_files, collect_supported_files_with_stats, glob_match, load_atlasignore,
    should_ignore,
};
pub use hash::hash_file;
pub use owners::{PackageOwners, WorkspaceRoot, discover_package_owners};
pub use path::{
    CanonicalRepoPath, RepoPathError, canonical_absolute_path, canonical_filesystem_path,
    normalize_case, normalize_unicode, repo_relative, to_forward_slashes,
};
pub use registry::{
    REPO_REGISTRY_FILE_NAME, REPO_REGISTRY_SCHEMA_VERSION, RepoDependency, RepoRegistration,
    RepoRegistry, RepoRegistryWarning, RepoRelationship, RepoRelationshipKind, TrustState,
    VcsMetadata, add_manual_repo, bootstrap_registry, discover_initialized_submodules,
    phase1_multi_repo_supported, registry_path, stable_repo_id,
};
pub use root::find_repo_root;
