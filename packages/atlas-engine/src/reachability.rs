//! Rust module reachability guard for weak `same_package` call edges.
//!
//! The guard reconstructs Cargo crate-root module trees from persisted module
//! facts. It is intentionally conservative:
//!
//! - incomplete indexes always return `Unknown`, never `Unreachable`;
//! - direct import-backed and parser-proven edges bypass the guard entirely;
//! - only weak `same_package` fallback candidates are checked.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::time::{Duration, Instant};

use anyhow::Result;
use atlas_core::{NodeKind, Result as AtlasResult};
use atlas_store_sqlite::Store;
use serde::{Deserialize, Serialize};

use crate::config::ReachabilityConfig;

/// Reachability status persisted in `Edge.extra_json.reachability`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReachabilityStatus {
    /// The candidate is reachable from the caller's crate root.
    Verified,
    /// The candidate is not reachable from the caller's crate root.
    Unreachable,
    /// Reachability could not be decided without guessing.
    Unknown,
    /// The language or provider does not apply.
    NotApplicable,
}

impl ReachabilityStatus {
    /// Stable snake-case persistence label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Unreachable => "unreachable",
            Self::Unknown => "unknown",
            Self::NotApplicable => "not_applicable",
        }
    }

    /// Parse a persisted label, rejecting unknown values.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "verified" => Some(Self::Verified),
            "unreachable" => Some(Self::Unreachable),
            "unknown" => Some(Self::Unknown),
            "not_applicable" => Some(Self::NotApplicable),
            _ => None,
        }
    }
}

/// Guard decision for one weak candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReachabilityResult {
    /// Candidate is reachable from the caller's crate root.
    Reachable,
    /// Candidate cannot be reached from the caller's crate root.
    Unreachable,
    /// Decision is unavailable without guessing.
    Unknown {
        /// Stable machine-readable reason.
        reason: String,
    },
}

impl ReachabilityResult {
    /// Convert the decision to persisted provenance.
    pub fn status(&self) -> ReachabilityStatus {
        match self {
            Self::Reachable => ReachabilityStatus::Verified,
            Self::Unreachable => ReachabilityStatus::Unreachable,
            Self::Unknown { .. } => ReachabilityStatus::Unknown,
        }
    }

    /// Stable reason label for diagnostics.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Unknown { reason } => Some(reason.as_str()),
            _ => None,
        }
    }
}

/// Cargo crate-root kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrateRootKind {
    /// `src/lib.rs`.
    Lib,
    /// `src/main.rs` or `src/bin/*.rs`.
    Bin,
    /// `examples/*.rs`.
    Example,
    /// `tests/*.rs`.
    Test,
    /// `benches/*.rs`.
    Bench,
}

impl CrateRootKind {
    /// Stable snake-case label.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lib => "lib",
            Self::Bin => "bin",
            Self::Example => "example",
            Self::Test => "test",
            Self::Bench => "bench",
        }
    }
}

/// One reconstructed crate root and the files reachable from it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrateRoot {
    /// Root file path (repo-relative).
    pub root_file: String,
    /// Root kind.
    pub kind: CrateRootKind,
    /// All files reachable from this root through module declarations.
    pub reachable_files: BTreeSet<String>,
    /// True when a module could not be resolved or lacks persisted facts.
    pub has_unknown_modules: bool,
    /// True when a `#[cfg(...)]` module was encountered.
    pub has_cfg_modules: bool,
}

/// Reachability index for one package owner.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RustReachabilityIndex {
    /// Package owner id (e.g. `cargo:Cargo.toml`).
    pub owner_id: String,
    /// Reconstructed crate roots, sorted by root file.
    pub roots: Vec<CrateRoot>,
    /// Module declarations that could not be resolved.
    pub unresolved_mods: Vec<String>,
    /// False when any module fact is missing, configured, or unresolved.
    pub index_complete: bool,
}

#[derive(Debug, Clone, Default)]
struct ModuleFacts {
    qn: String,
    fact_present: bool,
    inline: bool,
    module_file: Option<String>,
    has_cfg: bool,
    has_path_attr: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct FileFacts {
    has_include_macro: bool,
    has_macro_generated_modules: bool,
}

impl FileFacts {
    fn is_uncertain(self) -> bool {
        self.has_include_macro || self.has_macro_generated_modules
    }
}

impl RustReachabilityIndex {
    /// Reconstruct a Rust reachability index for one owner from the store.
    ///
    /// Reads persisted module nodes and file nodes only; incremental update
    /// never depends on in-memory `ParsedFile` output.
    pub fn from_store(store: &Store, source_repo_id: &str, owner_id: &str) -> AtlasResult<Self> {
        let files = store.files_for_owner_for_repo(source_repo_id, owner_id)?;
        let file_paths: BTreeSet<String> = files
            .iter()
            .filter(|file| file.path.ends_with(".rs"))
            .map(|file| file.path.clone())
            .collect();

        let nodes = store.module_and_file_nodes_for_owner_for_repo(source_repo_id, owner_id)?;
        let mut file_facts: HashMap<String, FileFacts> = HashMap::new();
        let mut modules_by_file: BTreeMap<String, Vec<ModuleFacts>> = BTreeMap::new();

        for node in nodes {
            match node.kind {
                NodeKind::File => {
                    let extra = node.extra_json.as_object();
                    let flags = extra
                        .map(|extra| FileFacts {
                            has_include_macro: extra
                                .get("has_include_macro")
                                .and_then(|value| value.as_bool())
                                .unwrap_or(false),
                            has_macro_generated_modules: extra
                                .get("has_macro_generated_modules")
                                .and_then(|value| value.as_bool())
                                .unwrap_or(false),
                        })
                        .unwrap_or_default();
                    file_facts.insert(node.file_path, flags);
                }
                NodeKind::Module => {
                    let extra = node.extra_json.as_object();
                    let fact_present = extra.is_some_and(|extra| extra.contains_key("inline"));
                    let facts = ModuleFacts {
                        qn: node.qualified_name,
                        fact_present,
                        inline: extra
                            .and_then(|extra| extra.get("inline"))
                            .and_then(|value| value.as_bool())
                            .unwrap_or(false),
                        module_file: extra
                            .and_then(|extra| extra.get("module_file"))
                            .and_then(|value| value.as_str())
                            .map(str::to_owned),
                        has_cfg: extra
                            .and_then(|extra| extra.get("has_cfg"))
                            .and_then(|value| value.as_bool())
                            .unwrap_or(false),
                        has_path_attr: extra
                            .and_then(|extra| extra.get("has_path_attr"))
                            .and_then(|value| value.as_bool())
                            .unwrap_or(false),
                    };
                    modules_by_file
                        .entry(node.file_path)
                        .or_default()
                        .push(facts);
                }
                _ => {}
            }
        }

        let mut roots = Vec::new();
        let mut unresolved_mods = BTreeSet::new();
        let mut index_complete = true;

        for (root_file, kind) in file_paths
            .iter()
            .filter_map(|path| crate_root_kind(path).map(|kind| (path.clone(), kind)))
        {
            let mut reachable_files = BTreeSet::new();
            let mut has_unknown_modules = false;
            let mut has_cfg_modules = false;
            let mut queue = VecDeque::from([root_file.clone()]);

            while let Some(file) = queue.pop_front() {
                if !reachable_files.insert(file.clone()) {
                    continue;
                }
                let facts = file_facts.get(&file);
                let Some(facts) = facts else {
                    has_unknown_modules = true;
                    index_complete = false;
                    continue;
                };
                if facts.is_uncertain() {
                    has_unknown_modules = true;
                    index_complete = false;
                    continue;
                }
                let modules = modules_by_file.get(&file).cloned().unwrap_or_default();
                for module in modules {
                    if !module.fact_present {
                        has_unknown_modules = true;
                        index_complete = false;
                        unresolved_mods.insert(format!("{} (missing facts)", module.qn));
                        continue;
                    }
                    if module.has_cfg {
                        has_cfg_modules = true;
                        index_complete = false;
                    }
                    if module.has_path_attr {
                        has_unknown_modules = true;
                        index_complete = false;
                    }
                    if module.inline {
                        continue;
                    }
                    let Some(module_file) = module.module_file.as_deref() else {
                        has_unknown_modules = true;
                        index_complete = false;
                        unresolved_mods.insert(format!("{} (no module file)", module.qn));
                        continue;
                    };
                    match resolve_module_path(module_file, &file_paths) {
                        Some(resolved) => {
                            if !reachable_files.contains(&resolved) {
                                queue.push_back(resolved);
                            }
                        }
                        None => {
                            has_unknown_modules = true;
                            index_complete = false;
                            unresolved_mods.insert(format!("{} -> {module_file}", module.qn));
                        }
                    }
                }
            }

            roots.push(CrateRoot {
                root_file,
                kind,
                reachable_files,
                has_unknown_modules,
                has_cfg_modules,
            });
        }

        if roots.is_empty() {
            // No recognizable crate root means the module tree is unknown, not
            // empty. Keep the index incomplete so the guard never rejects.
            index_complete = false;
        }

        Ok(Self {
            owner_id: owner_id.to_owned(),
            roots,
            unresolved_mods: unresolved_mods.into_iter().collect(),
            index_complete,
        })
    }

    /// Root containing `file`, if any.
    fn root_for(&self, file: &str) -> Option<&CrateRoot> {
        self.roots
            .iter()
            .find(|root| root.reachable_files.contains(file) || root.root_file == file)
    }
}

fn resolve_module_path(module_file: &str, file_paths: &BTreeSet<String>) -> Option<String> {
    if file_paths.contains(module_file) {
        return Some(module_file.to_owned());
    }
    if let Some(stem) = module_file.strip_suffix(".rs")
        && let Some((parent, name)) = stem.rsplit_once('/')
    {
        let candidate = format!("{parent}/{name}/mod.rs");
        if file_paths.contains(&candidate) {
            return Some(candidate);
        }
        if parent.is_empty() {
            let candidate = format!("{name}/mod.rs");
            if file_paths.contains(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// Classify a standard Cargo crate root path, if it is one.
pub fn crate_root_kind(path: &str) -> Option<CrateRootKind> {
    let normalized = path.replace('\\', "/");
    let normalized = normalized.trim_start_matches("./");
    let segments: Vec<&str> = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let file = *segments.last()?;
    if !file.ends_with(".rs") {
        return None;
    }

    if file == "lib.rs" && segments.len() >= 2 && segments[segments.len() - 2] == "src" {
        return Some(CrateRootKind::Lib);
    }
    if file == "main.rs" && segments.len() >= 2 && segments[segments.len() - 2] == "src" {
        return Some(CrateRootKind::Bin);
    }

    for (dir, kind) in [
        ("bin", CrateRootKind::Bin),
        ("tests", CrateRootKind::Test),
        ("examples", CrateRootKind::Example),
        ("benches", CrateRootKind::Bench),
    ] {
        let Some(index) = segments.iter().position(|segment| *segment == dir) else {
            continue;
        };
        let tail = &segments[index + 1..];
        match tail {
            [name] if name.ends_with(".rs") && *name != "mod.rs" => return Some(kind),
            [name, "main.rs"] if !name.is_empty() => return Some(kind),
            _ => {}
        }
    }
    None
}

/// Reachability guard keyed by `(language, owner_id)`.
#[derive(Debug, Default)]
pub struct ReachabilityGuard {
    enabled: bool,
    indexes: HashMap<(String, String), RustReachabilityIndex>,
    owners_skipped: usize,
    unknown_reasons: BTreeMap<String, usize>,
    weak_edges_accepted: usize,
    weak_edges_rejected: usize,
    budget_exhausted: bool,
}

impl ReachabilityGuard {
    /// Create an empty guard with the configured enable flag.
    pub fn new(config: &ReachabilityConfig) -> Self {
        Self {
            enabled: config.enabled,
            ..Self::default()
        }
    }

    /// Build indexes for the owners touched by one reconcile run.
    ///
    /// Applies `max_owners_per_run` and `budget_ms`; skipped owners stay
    /// `Unknown` so the guard never rejects from a partial index.
    pub fn prepare(
        &mut self,
        store: &Store,
        source_repo_id: &str,
        owners: impl IntoIterator<Item = (String, String)>,
        config: &ReachabilityConfig,
    ) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let mut owners: Vec<(String, String)> = owners.into_iter().collect();
        owners.sort();
        owners.dedup();

        let started = Instant::now();
        let budget = Duration::from_millis(config.budget_ms);
        let mut indexed = 0usize;
        for (language, owner_id) in owners {
            if language != "rust" {
                continue;
            }
            if indexed >= config.max_owners_per_run {
                self.owners_skipped += 1;
                self.record_unknown_reason("owner_limit");
                continue;
            }
            if started.elapsed() >= budget {
                self.owners_skipped += 1;
                self.budget_exhausted = true;
                self.record_unknown_reason("budget_exhausted");
                continue;
            }
            let index = RustReachabilityIndex::from_store(store, source_repo_id, &owner_id)?;
            self.indexes.insert((language, owner_id), index);
            indexed += 1;
        }
        Ok(())
    }

    /// Decide whether `candidate_file` is reachable from `caller_file` inside
    /// the same crate root. Never returns `Unreachable` for an incomplete index.
    pub fn is_reachable_from_same_root(
        &self,
        language: &str,
        owner_id: &str,
        caller_file: &str,
        candidate_file: &str,
    ) -> ReachabilityResult {
        if !self.enabled {
            return ReachabilityResult::Unknown {
                reason: "provider_disabled".to_owned(),
            };
        }
        if language != "rust" {
            return ReachabilityResult::Unknown {
                reason: "provider_absent".to_owned(),
            };
        }
        let key = (language.to_owned(), owner_id.to_owned());
        let Some(index) = self.indexes.get(&key) else {
            return ReachabilityResult::Unknown {
                reason: "index_absent".to_owned(),
            };
        };
        if !index.index_complete {
            return ReachabilityResult::Unknown {
                reason: "index_incomplete".to_owned(),
            };
        }
        let Some(root) = index.root_for(caller_file) else {
            return ReachabilityResult::Unknown {
                reason: "caller_not_in_crate_root".to_owned(),
            };
        };
        if root.has_unknown_modules {
            return ReachabilityResult::Unknown {
                reason: "unknown_modules".to_owned(),
            };
        }
        if root.has_cfg_modules {
            return ReachabilityResult::Unknown {
                reason: "cfg_modules".to_owned(),
            };
        }
        if root.reachable_files.contains(candidate_file) {
            return ReachabilityResult::Reachable;
        }
        // Package targets can depend on the library target without a module
        // declaration. Treat any lib-root file as reachable from a non-lib
        // crate root; never the reverse.
        if let Some(candidate_root) = index.root_for(candidate_file) {
            if candidate_root.kind == CrateRootKind::Lib && root.kind != CrateRootKind::Lib {
                return ReachabilityResult::Reachable;
            }
            return ReachabilityResult::Unreachable;
        }
        ReachabilityResult::Unreachable
    }

    /// Resolve one weak edge, recording acceptance/rejection counters.
    pub fn resolve_weak_edge(
        &mut self,
        language: &str,
        owner_id: &str,
        caller_file: &str,
        candidate_file: &str,
    ) -> ReachabilityResult {
        let result =
            self.is_reachable_from_same_root(language, owner_id, caller_file, candidate_file);
        match &result {
            ReachabilityResult::Reachable => self.weak_edges_accepted += 1,
            ReachabilityResult::Unreachable => self.weak_edges_rejected += 1,
            ReachabilityResult::Unknown { reason } => {
                self.weak_edges_accepted += 1;
                self.record_unknown_reason(reason);
            }
        }
        result
    }

    fn record_unknown_reason(&mut self, reason: &str) {
        *self.unknown_reasons.entry(reason.to_owned()).or_insert(0) += 1;
    }

    /// Record a weak edge rejected outside the normal guard decision path
    /// (for example stale-target cleanup after a file deletion).
    pub fn record_weak_edge_rejected(&mut self) {
        self.weak_edges_rejected += 1;
    }

    /// Snapshot guard statistics for build/update/doctor output.
    pub fn summary(&self) -> ReachabilitySummary {
        ReachabilitySummary {
            enabled: self.enabled,
            owners_indexed: self.indexes.len(),
            owners_skipped: self.owners_skipped,
            unknown_reasons: self.unknown_reasons.clone(),
            unresolved_mods: self
                .indexes
                .values()
                .map(|index| index.unresolved_mods.len())
                .sum(),
            weak_edges_accepted: self.weak_edges_accepted,
            weak_edges_rejected: self.weak_edges_rejected,
            budget_exhausted: self.budget_exhausted,
        }
    }
}

/// Reachability summary emitted by build, update, and doctor surfaces.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReachabilitySummary {
    /// Whether the guard was enabled for the run.
    pub enabled: bool,
    /// Number of owners indexed.
    pub owners_indexed: usize,
    /// Number of owners skipped by budget or owner limit.
    pub owners_skipped: usize,
    /// Unknown-reason counts keyed by reason label.
    pub unknown_reasons: BTreeMap<String, usize>,
    /// Total unresolved module declarations across indexed owners.
    pub unresolved_mods: usize,
    /// Weak edges retained with verified or unknown provenance.
    pub weak_edges_accepted: usize,
    /// Weak edges rejected as unreachable.
    pub weak_edges_rejected: usize,
    /// True when `budget_ms` was exceeded.
    pub budget_exhausted: bool,
}

/// Build a reachability summary for diagnostics surfaces.
///
/// Indexes all owner ids recorded for the repo and combines guard statistics
/// with persisted provenance counters. Never mutates graph state.
pub fn diagnose_reachability(
    store: &Store,
    source_repo_id: &str,
    config: &ReachabilityConfig,
) -> Result<ReachabilitySummary> {
    let owners = store.owner_ids_for_repo(source_repo_id)?;
    let mut guard = ReachabilityGuard::new(config);
    guard.prepare(
        store,
        source_repo_id,
        owners
            .into_iter()
            .map(|owner_id| ("rust".to_owned(), owner_id)),
        config,
    )?;
    let mut summary = guard.summary();
    summary.weak_edges_accepted = ["verified", "unknown", "not_applicable"]
        .into_iter()
        .map(|status| store.count_edges_with_reachability(source_repo_id, status))
        .collect::<AtlasResult<Vec<_>>>()?
        .into_iter()
        .sum();
    summary.weak_edges_rejected = store.count_unresolved_weak_edges(source_repo_id)?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_root_classification_is_conservative() {
        assert_eq!(crate_root_kind("src/lib.rs"), Some(CrateRootKind::Lib));
        assert_eq!(
            crate_root_kind("crates/foo/src/lib.rs"),
            Some(CrateRootKind::Lib)
        );
        assert_eq!(crate_root_kind("src/main.rs"), Some(CrateRootKind::Bin));
        assert_eq!(crate_root_kind("src/bin/tool.rs"), Some(CrateRootKind::Bin));
        assert_eq!(
            crate_root_kind("src/bin/tool/main.rs"),
            Some(CrateRootKind::Bin)
        );
        assert_eq!(crate_root_kind("src/bin/tool/helper.rs"), None);
        assert_eq!(
            crate_root_kind("tests/integration.rs"),
            Some(CrateRootKind::Test)
        );
        assert_eq!(crate_root_kind("tests/common/helper.rs"), None);
        assert_eq!(
            crate_root_kind("examples/demo.rs"),
            Some(CrateRootKind::Example)
        );
        assert_eq!(
            crate_root_kind("benches/bench.rs"),
            Some(CrateRootKind::Bench)
        );
        assert_eq!(crate_root_kind("src/orphan.rs"), None);
    }

    #[test]
    fn module_path_resolution_prefers_file_then_mod_rs() {
        let files = ["src/foo.rs".to_owned(), "src/bar/mod.rs".to_owned()]
            .into_iter()
            .collect();
        assert_eq!(
            resolve_module_path("src/foo.rs", &files).as_deref(),
            Some("src/foo.rs")
        );
        assert_eq!(
            resolve_module_path("src/bar.rs", &files).as_deref(),
            Some("src/bar/mod.rs")
        );
        assert_eq!(resolve_module_path("src/missing.rs", &files), None);
    }

    const REPO: &str = "reachability-test-repo";
    const OWNER: &str = "cargo:Cargo.toml";

    fn test_node(
        kind: NodeKind,
        file_path: &str,
        qualified_name: &str,
        extra_json: serde_json::Value,
    ) -> atlas_core::Node {
        atlas_core::Node {
            id: atlas_core::NodeId::UNSET,
            kind,
            name: qualified_name
                .rsplit("::")
                .next()
                .unwrap_or("node")
                .to_owned(),
            qualified_name: qualified_name.to_owned(),
            file_path: file_path.to_owned(),
            line_start: 1,
            line_end: 1,
            language: "rust".to_owned(),
            parent_name: None,
            params: None,
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: format!("hash-{file_path}"),
            extra_json,
            repo_provenance: None,
        }
    }

    fn file_node(file_path: &str, extra_json: serde_json::Value) -> atlas_core::Node {
        test_node(NodeKind::File, file_path, file_path, extra_json)
    }

    fn module_node(
        file_path: &str,
        qualified_name: &str,
        inline: bool,
        module_file: Option<&str>,
        has_cfg: bool,
        has_path_attr: bool,
    ) -> atlas_core::Node {
        test_node(
            NodeKind::Module,
            file_path,
            qualified_name,
            serde_json::json!({
                "inline": inline,
                "module_file": module_file,
                "has_cfg": has_cfg,
                "has_path_attr": has_path_attr,
                "path_attr_value": has_path_attr.then_some("custom.rs"),
            }),
        )
    }

    fn write_file(store: &mut Store, path: &str, nodes: Vec<atlas_core::Node>, owner_id: &str) {
        store
            .replace_file_graph_for_repo(
                REPO,
                path,
                &format!("hash-{path}"),
                Some("rust"),
                None,
                &nodes,
                &[],
            )
            .unwrap();
        let owner = atlas_core::PackageOwner {
            owner_id: owner_id.to_owned(),
            kind: atlas_core::PackageOwnerKind::Cargo,
            root: ".".to_owned(),
            manifest_path: "Cargo.toml".to_owned(),
            package_name: Some("fixture".to_owned()),
        };
        store
            .upsert_file_owner_for_repo(REPO, path, Some(&owner))
            .unwrap();
    }

    fn seeded_store() -> Store {
        let mut store = Store::open(":memory:").unwrap();
        store.migrate().unwrap();
        write_file(
            &mut store,
            "src/lib.rs",
            vec![
                file_node("src/lib.rs", serde_json::json!({})),
                module_node(
                    "src/lib.rs",
                    "src/lib.rs::module::helper",
                    false,
                    Some("src/helper.rs"),
                    false,
                    false,
                ),
            ],
            OWNER,
        );
        write_file(
            &mut store,
            "src/helper.rs",
            vec![file_node("src/helper.rs", serde_json::json!({}))],
            OWNER,
        );
        write_file(
            &mut store,
            "src/orphan.rs",
            vec![file_node("src/orphan.rs", serde_json::json!({}))],
            OWNER,
        );
        store
    }

    #[test]
    fn index_reconstructs_module_tree_from_store() {
        let store = seeded_store();
        let index = RustReachabilityIndex::from_store(&store, REPO, OWNER).unwrap();
        assert!(index.index_complete);
        assert_eq!(index.roots.len(), 1);
        let root = &index.roots[0];
        assert_eq!(root.kind, CrateRootKind::Lib);
        assert!(root.reachable_files.contains("src/lib.rs"));
        assert!(root.reachable_files.contains("src/helper.rs"));
        assert!(!root.reachable_files.contains("src/orphan.rs"));
    }

    #[test]
    fn guard_separates_reachable_and_orphan_files() {
        let store = seeded_store();
        let mut guard = ReachabilityGuard::new(&ReachabilityConfig::default());
        guard
            .prepare(
                &store,
                REPO,
                [("rust".to_owned(), OWNER.to_owned())],
                &ReachabilityConfig::default(),
            )
            .unwrap();
        assert_eq!(
            guard.is_reachable_from_same_root("rust", OWNER, "src/lib.rs", "src/helper.rs"),
            ReachabilityResult::Reachable
        );
        assert_eq!(
            guard.is_reachable_from_same_root("rust", OWNER, "src/lib.rs", "src/orphan.rs"),
            ReachabilityResult::Unreachable
        );
    }

    #[test]
    fn cfg_module_keeps_index_incomplete_and_never_unreachable() {
        let mut store = Store::open(":memory:").unwrap();
        store.migrate().unwrap();
        write_file(
            &mut store,
            "src/lib.rs",
            vec![
                file_node("src/lib.rs", serde_json::json!({})),
                module_node(
                    "src/lib.rs",
                    "src/lib.rs::module::platform",
                    true,
                    None,
                    true,
                    false,
                ),
            ],
            OWNER,
        );
        write_file(
            &mut store,
            "src/orphan.rs",
            vec![file_node("src/orphan.rs", serde_json::json!({}))],
            OWNER,
        );
        let index = RustReachabilityIndex::from_store(&store, REPO, OWNER).unwrap();
        assert!(!index.index_complete);
        assert!(index.roots[0].has_cfg_modules);

        let mut guard = ReachabilityGuard::new(&ReachabilityConfig::default());
        guard
            .prepare(
                &store,
                REPO,
                [("rust".to_owned(), OWNER.to_owned())],
                &ReachabilityConfig::default(),
            )
            .unwrap();
        assert!(matches!(
            guard.is_reachable_from_same_root("rust", OWNER, "src/lib.rs", "src/orphan.rs"),
            ReachabilityResult::Unknown { .. }
        ));
    }

    #[test]
    fn path_attr_and_missing_facts_mark_index_incomplete() {
        let mut store = Store::open(":memory:").unwrap();
        store.migrate().unwrap();
        write_file(
            &mut store,
            "src/lib.rs",
            vec![
                file_node("src/lib.rs", serde_json::json!({})),
                module_node(
                    "src/lib.rs",
                    "src/lib.rs::module::custom",
                    false,
                    Some("src/custom.rs"),
                    false,
                    true,
                ),
                test_node(
                    NodeKind::Module,
                    "src/lib.rs",
                    "src/lib.rs::module::legacy",
                    serde_json::Value::Null,
                ),
            ],
            OWNER,
        );
        let index = RustReachabilityIndex::from_store(&store, REPO, OWNER).unwrap();
        assert!(!index.index_complete);
        assert!(index.roots[0].has_unknown_modules);
        assert!(
            index
                .unresolved_mods
                .iter()
                .any(|entry| entry.contains("missing facts"))
        );
    }

    #[test]
    fn zero_budget_marks_indexes_unknown() {
        let store = seeded_store();
        let config = ReachabilityConfig {
            enabled: true,
            budget_ms: 0,
            max_owners_per_run: 64,
        };
        let mut guard = ReachabilityGuard::new(&config);
        guard
            .prepare(
                &store,
                REPO,
                [("rust".to_owned(), OWNER.to_owned())],
                &config,
            )
            .unwrap();
        let summary = guard.summary();
        assert!(summary.budget_exhausted);
        assert_eq!(summary.owners_indexed, 0);
        assert_eq!(summary.owners_skipped, 1);
        assert_eq!(summary.unknown_reasons.get("budget_exhausted"), Some(&1));
    }

    #[test]
    fn owner_limit_skips_remaining_owners() {
        let store = seeded_store();
        let config = ReachabilityConfig {
            enabled: true,
            budget_ms: 250,
            max_owners_per_run: 1,
        };
        let mut guard = ReachabilityGuard::new(&config);
        guard
            .prepare(
                &store,
                REPO,
                [
                    ("rust".to_owned(), OWNER.to_owned()),
                    ("rust".to_owned(), "cargo:other/Cargo.toml".to_owned()),
                ],
                &config,
            )
            .unwrap();
        let summary = guard.summary();
        assert_eq!(summary.owners_indexed, 1);
        assert_eq!(summary.owners_skipped, 1);
        assert_eq!(summary.unknown_reasons.get("owner_limit"), Some(&1));
    }
}
