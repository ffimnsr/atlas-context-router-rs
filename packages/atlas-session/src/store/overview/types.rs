//! ICM-G1 — overview domain model and subject identity.
//!
//! Overview memory is evidence-linked code documentation. It is stored in the
//! continuity-owned session database and is never merged into code graph
//! `nodes`/`edges`. Atlas stays non-LLM here: these types only describe
//! externally produced analysis and bounded analysis packets.

use std::str::FromStr;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use atlas_core::{AtlasError, Result};
use atlas_repo::CanonicalRepoPath;
use camino::Utf8Path;

/// First stable overview schema. Breaking changes must ship a migration or an
/// explicit import rejection; no compatibility shims before this ships.
pub const OVERVIEW_SCHEMA_VERSION: u32 = 1;

/// Exact subject kinds accepted by the overview layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverviewSubjectKind {
    Project,
    Package,
    Module,
    File,
    Symbol,
    Function,
    Method,
}

impl OverviewSubjectKind {
    pub const ALL: [Self; 7] = [
        Self::Project,
        Self::Package,
        Self::Module,
        Self::File,
        Self::Symbol,
        Self::Function,
        Self::Method,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Package => "package",
            Self::Module => "module",
            Self::File => "file",
            Self::Symbol => "symbol",
            Self::Function => "function",
            Self::Method => "method",
        }
    }

    /// Higher values are more specific subjects. Used by ranking so a symbol
    /// overview outranks a project overview when both match.
    pub fn specificity(self) -> u8 {
        match self {
            Self::Project => 0,
            Self::Package => 1,
            Self::Module => 2,
            Self::File => 3,
            Self::Symbol => 4,
            Self::Function => 5,
            Self::Method => 6,
        }
    }
}

impl std::fmt::Display for OverviewSubjectKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for OverviewSubjectKind {
    type Err = AtlasError;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "project" => Ok(Self::Project),
            "package" => Ok(Self::Package),
            "module" => Ok(Self::Module),
            "file" => Ok(Self::File),
            "symbol" => Ok(Self::Symbol),
            "function" => Ok(Self::Function),
            "method" => Ok(Self::Method),
            other => Err(AtlasError::Other(format!(
                "unknown overview subject kind: {other}; expected project, package, module, file, symbol, function, or method"
            ))),
        }
    }
}

/// Freshness states for overview records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverviewFreshness {
    Fresh,
    PossiblyStale,
    Stale,
    Orphaned,
    Unverified,
}

impl OverviewFreshness {
    pub const ALL: [Self; 5] = [
        Self::Fresh,
        Self::PossiblyStale,
        Self::Stale,
        Self::Orphaned,
        Self::Unverified,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::PossiblyStale => "possibly_stale",
            Self::Stale => "stale",
            Self::Orphaned => "orphaned",
            Self::Unverified => "unverified",
        }
    }

    pub fn is_stale(self) -> bool {
        matches!(self, Self::PossiblyStale | Self::Stale)
    }

    /// Freshness rank used by search ranking: fresher is better.
    pub fn rank(self) -> u8 {
        match self {
            Self::Fresh => 3,
            Self::Unverified => 2,
            Self::PossiblyStale | Self::Stale => 1,
            Self::Orphaned => 0,
        }
    }
}

impl std::fmt::Display for OverviewFreshness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for OverviewFreshness {
    type Err = AtlasError;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "fresh" => Ok(Self::Fresh),
            "possibly_stale" => Ok(Self::PossiblyStale),
            "stale" => Ok(Self::Stale),
            "orphaned" => Ok(Self::Orphaned),
            "unverified" => Ok(Self::Unverified),
            other => Err(AtlasError::Other(format!(
                "unknown overview freshness: {other}; expected fresh, possibly_stale, stale, orphaned, or unverified"
            ))),
        }
    }
}

/// Provenance of an overview record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverviewSourceKind {
    ExternalLlm,
    #[default]
    Manual,
    Imported,
}

impl OverviewSourceKind {
    pub const ALL: [Self; 3] = [Self::ExternalLlm, Self::Manual, Self::Imported];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExternalLlm => "external_llm",
            Self::Manual => "manual",
            Self::Imported => "imported",
        }
    }
}

impl std::fmt::Display for OverviewSourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for OverviewSourceKind {
    type Err = AtlasError;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "external_llm" | "external-llm" => Ok(Self::ExternalLlm),
            "manual" => Ok(Self::Manual),
            "imported" => Ok(Self::Imported),
            other => Err(AtlasError::Other(format!(
                "unknown overview source kind: {other}; expected external_llm, manual, or imported"
            ))),
        }
    }
}

/// Canonical subject identity for an overview record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverviewSubject {
    pub kind: OverviewSubjectKind,
    pub repo_root: String,
    pub commit_sha: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualified_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_start: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_end: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
}

impl OverviewSubject {
    /// Validate required fields per subject kind and canonicalize any path
    /// through [`CanonicalRepoPath`]. All path-derived identity must pass
    /// through this function before hashing or persistence.
    pub fn validated(mut self) -> Result<Self> {
        if self.repo_root.trim().is_empty() {
            return Err(AtlasError::Other(
                "overview subject repo_root must not be empty".to_owned(),
            ));
        }
        if self.commit_sha.trim().is_empty() {
            return Err(AtlasError::Other(
                "overview subject commit_sha must not be empty".to_owned(),
            ));
        }

        if let Some(path) = self.canonical_file_path.as_deref() {
            self.canonical_file_path = Some(canonicalize_subject_path(&self.repo_root, path)?);
        }

        match self.kind {
            OverviewSubjectKind::Project => {}
            OverviewSubjectKind::Package => {
                require_field(&self.package_name, "package_name", self.kind)?;
            }
            OverviewSubjectKind::Module => {
                require_field(&self.module_path, "module_path", self.kind)?;
            }
            OverviewSubjectKind::File => {
                require_field(&self.canonical_file_path, "canonical_file_path", self.kind)?;
            }
            OverviewSubjectKind::Symbol
            | OverviewSubjectKind::Function
            | OverviewSubjectKind::Method => {
                require_field(&self.qualified_name, "qualified_name", self.kind)?;
                require_field(&self.canonical_file_path, "canonical_file_path", self.kind)?;
                let (Some(line_start), Some(line_end)) = (self.line_start, self.line_end) else {
                    return Err(AtlasError::Other(format!(
                        "overview {} subject requires line_start and line_end",
                        self.kind
                    )));
                };
                if line_start == 0 {
                    return Err(AtlasError::Other(
                        "overview subject line_start must be greater than 0".to_owned(),
                    ));
                }
                if line_end < line_start {
                    return Err(AtlasError::Other(
                        "overview subject line_end must be greater than or equal to line_start"
                            .to_owned(),
                    ));
                }
            }
        }

        Ok(self)
    }
}

fn require_field(value: &Option<String>, field: &str, kind: OverviewSubjectKind) -> Result<()> {
    if value.as_deref().is_none_or(|value| value.trim().is_empty()) {
        return Err(AtlasError::Other(format!(
            "overview {kind} subject requires {field}"
        )));
    }
    Ok(())
}

/// Canonicalize a repo-relative overview subject path.
///
/// Uses the shared [`CanonicalRepoPath`] boundary helper so `./src/lib.rs` and
/// `src/lib.rs` resolve to the same identity. Local normalization helpers are
/// explicitly forbidden by the path identity invariant.
pub fn canonicalize_subject_path(repo_root: &str, path: &str) -> Result<String> {
    let root = Utf8Path::new(repo_root);
    CanonicalRepoPath::from_cli_argument(root, Utf8Path::new(path))
        .map(|path| path.as_str().to_owned())
        .map_err(|error| AtlasError::Other(format!("invalid overview subject path: {error}")))
}

/// Deterministic, versioned subject id.
pub fn subject_id(subject: &OverviewSubject) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(b"overview_subject");
    hasher.update(b"\x00");
    hasher.update(OVERVIEW_SCHEMA_VERSION.to_string().as_bytes());
    hasher.update(b"\x00");
    hasher.update(subject.repo_root.trim().as_bytes());
    hasher.update(b"\x00");
    hasher.update(subject.kind.as_str().as_bytes());
    hasher.update(b"\x00");
    hasher.update(subject.package_name.as_deref().unwrap_or("").as_bytes());
    hasher.update(b"\x00");
    hasher.update(subject.module_path.as_deref().unwrap_or("").as_bytes());
    hasher.update(b"\x00");
    hasher.update(
        subject
            .canonical_file_path
            .as_deref()
            .unwrap_or("")
            .as_bytes(),
    );
    hasher.update(b"\x00");
    hasher.update(subject.qualified_name.as_deref().unwrap_or("").as_bytes());
    hasher.update(b"\x00");
    hasher.update(subject.node_kind.as_deref().unwrap_or("").as_bytes());
    hasher.update(b"\x00");
    hasher.update(
        subject
            .line_start
            .map(|value| value.to_string())
            .unwrap_or_default()
            .as_bytes(),
    );
    hasher.update(b"\x00");
    hasher.update(
        subject
            .line_end
            .map(|value| value.to_string())
            .unwrap_or_default()
            .as_bytes(),
    );
    hasher.update(b"\x00");
    hasher.update(subject.content_hash.as_deref().unwrap_or("").as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Stored overview record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverviewRecord {
    pub id: String,
    pub repo_root: String,
    pub subject_id: String,
    pub subject_kind: OverviewSubjectKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub module_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub canonical_file_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qualified_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub node_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_start: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_end: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    pub title: String,
    pub summary: String,
    pub description_preview: String,
    pub responsibilities: Vec<String>,
    pub flow: Vec<String>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub invariants: Vec<String>,
    pub gotchas: Vec<String>,
    pub examples: Vec<String>,
    pub tags: Vec<String>,
    pub source_kind: OverviewSourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analyzer_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analyzer_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    pub commit_sha: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_last_indexed_at: Option<String>,
    pub freshness: OverviewFreshness,
    pub confidence: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub metadata: Value,
}

impl OverviewRecord {
    pub fn subject(&self) -> OverviewSubject {
        OverviewSubject {
            kind: self.subject_kind,
            repo_root: self.repo_root.clone(),
            commit_sha: self.commit_sha.clone(),
            package_name: self.package_name.clone(),
            module_path: self.module_path.clone(),
            canonical_file_path: self.canonical_file_path.clone(),
            qualified_name: self.qualified_name.clone(),
            node_kind: self.node_kind.clone(),
            line_start: self.line_start,
            line_end: self.line_end,
            content_hash: self.content_hash.clone(),
        }
    }
}

/// New overview record accepted by the storage API. Identity, supersession,
/// and timestamps are derived by the service layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewOverviewRecord {
    pub subject: OverviewSubject,
    #[serde(default)]
    pub title: String,
    pub summary: String,
    #[serde(default)]
    pub description_preview: String,
    #[serde(default)]
    pub responsibilities: Vec<String>,
    #[serde(default)]
    pub flow: Vec<String>,
    #[serde(default)]
    pub inputs: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub invariants: Vec<String>,
    #[serde(default)]
    pub gotchas: Vec<String>,
    #[serde(default)]
    pub examples: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub source_kind: OverviewSourceKind,
    #[serde(default)]
    pub analyzer_name: Option<String>,
    #[serde(default)]
    pub analyzer_version: Option<String>,
    #[serde(default)]
    pub model_name: Option<String>,
    #[serde(default)]
    pub graph_last_indexed_at: Option<String>,
    #[serde(default)]
    pub freshness: Option<OverviewFreshness>,
    #[serde(default = "default_overview_confidence")]
    pub confidence: f64,
    #[serde(default)]
    pub source_id: Option<String>,
    /// Explicit supersession target. When omitted the service finds the
    /// previous active record for the same repo/subject/source kind.
    #[serde(default)]
    pub supersedes_id: Option<String>,
    #[serde(default = "default_metadata")]
    pub metadata: Value,
}

fn default_overview_confidence() -> f64 {
    0.5
}

fn default_metadata() -> Value {
    Value::Object(Default::default())
}

/// External analysis input accepted by `atlas overview ingest`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OverviewAnalysisInput {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub packet_id: Option<String>,
    /// Evidence ids exported with the packet. Echoed back so `packet_id` can be
    /// validated deterministically without storing raw packet bodies.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_ids: Vec<String>,
    pub subject: OverviewSubject,
    #[serde(default)]
    pub title: String,
    pub summary: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub responsibilities: Vec<String>,
    #[serde(default)]
    pub flow: Vec<String>,
    #[serde(default)]
    pub inputs: Vec<String>,
    #[serde(default)]
    pub outputs: Vec<String>,
    #[serde(default)]
    pub invariants: Vec<String>,
    #[serde(default)]
    pub gotchas: Vec<String>,
    #[serde(default)]
    pub examples: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_overview_confidence")]
    pub confidence: f64,
    #[serde(default)]
    pub source_kind: OverviewSourceKind,
    #[serde(default)]
    pub analyzer_name: Option<String>,
    #[serde(default)]
    pub analyzer_version: Option<String>,
    #[serde(default)]
    pub model_name: Option<String>,
    /// Optional saved-context artifact ids backing this analysis.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_ids: Vec<String>,
    #[serde(default = "default_metadata")]
    pub metadata: Value,
}

/// Search/list ranking evidence for one overview hit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverviewRanking {
    pub exact_subject_match: bool,
    pub exact_title_match: bool,
    pub exact_tag_match: bool,
    pub fts_score: f64,
    pub freshness_rank: u8,
    pub confidence: f64,
    pub recency_rank: f64,
    pub subject_specificity: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverviewSearchHit {
    pub record: OverviewRecord,
    pub score: f64,
    pub ranking: OverviewRanking,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverviewSearchFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_kind: Option<OverviewSubjectKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness: Option<OverviewFreshness>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<OverviewSourceKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverviewListFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_kind: Option<OverviewSubjectKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness: Option<OverviewFreshness>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_kind: Option<OverviewSourceKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub older_than: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newer_than: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverviewStaleFilter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_kind: Option<OverviewSubjectKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_file_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverviewDeleteResult {
    pub overview_id: String,
    pub found: bool,
    pub deleted: bool,
    pub dry_run: bool,
}

/// Bounded query for overview context selection.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverviewContextQuery {
    pub repo_root: String,
    #[serde(default)]
    pub qualified_names: Vec<String>,
    #[serde(default)]
    pub file_paths: Vec<String>,
    #[serde(default)]
    pub modules: Vec<String>,
    #[serde(default)]
    pub packages: Vec<String>,
    #[serde(default)]
    pub changed_files: Vec<String>,
    #[serde(default)]
    pub freshness: Vec<OverviewFreshness>,
    #[serde(default)]
    pub subject_kinds: Vec<OverviewSubjectKind>,
    #[serde(default)]
    pub include_stale: bool,
    #[serde(default = "default_context_limit")]
    pub limit: usize,
}

fn default_context_limit() -> usize {
    10
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverviewContextHit {
    pub record: OverviewRecord,
    pub selection_reason: String,
    pub score: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_reason: Option<String>,
}

/// Analysis export scopes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverviewExportScope {
    Project,
    Packages,
    Modules,
    Files,
    Symbols,
    Functions,
    Methods,
    Changed,
}

impl OverviewExportScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Packages => "packages",
            Self::Modules => "modules",
            Self::Files => "files",
            Self::Symbols => "symbols",
            Self::Functions => "functions",
            Self::Methods => "methods",
            Self::Changed => "changed",
        }
    }
}

impl FromStr for OverviewExportScope {
    type Err = AtlasError;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "project" => Ok(Self::Project),
            "packages" | "package" => Ok(Self::Packages),
            "modules" | "module" => Ok(Self::Modules),
            "files" | "file" => Ok(Self::Files),
            "symbols" | "symbol" => Ok(Self::Symbols),
            "functions" | "function" => Ok(Self::Functions),
            "methods" | "method" => Ok(Self::Methods),
            "changed" => Ok(Self::Changed),
            other => Err(AtlasError::Other(format!(
                "unknown overview export scope: {other}; expected project, packages, modules, files, symbols, functions, methods, or changed"
            ))),
        }
    }
}

/// Export request shared by CLI and MCP so defaults cannot drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverviewExportRequest {
    pub scope: OverviewExportScope,
    #[serde(default)]
    pub subjects: Vec<String>,
    #[serde(default)]
    pub changed_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default)]
    pub include_code_spans: bool,
    #[serde(default)]
    pub include_callers: bool,
    #[serde(default)]
    pub include_callees: bool,
    #[serde(default)]
    pub include_tests: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<usize>,
    #[serde(default)]
    pub json: bool,
}

impl Default for OverviewExportRequest {
    fn default() -> Self {
        Self {
            scope: OverviewExportScope::Project,
            subjects: Vec::new(),
            changed_only: false,
            since: None,
            limit: None,
            include_code_spans: false,
            include_callers: false,
            include_callees: false,
            include_tests: false,
            max_tokens: None,
            json: false,
        }
    }
}

/// One bounded analysis packet. Atlas never calls a model; it only emits this
/// evidence packet for an external analyzer or human to fill in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverviewAnalysisPacket {
    pub schema_version: u32,
    pub packet_id: String,
    pub repo_root: String,
    pub commit_sha: String,
    pub graph: OverviewGraphProvenance,
    pub freshness: OverviewPacketFreshness,
    pub subject: OverviewSubject,
    pub subject_id: String,
    pub evidence: OverviewPacketEvidence,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code_excerpt: Option<OverviewCodeExcerpt>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub callers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub callees: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_files: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tests: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retrieval_hints: Vec<String>,
    pub evidence_ids: Vec<String>,
    pub safe_to_answer: bool,
    pub omitted_subject_count: usize,
    pub budget: OverviewPacketBudget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverviewGraphProvenance {
    pub graph_db_path: String,
    pub graph_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_indexed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_indexed_ref: Option<String>,
    pub indexed_file_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverviewPacketFreshness {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stale_reason: Option<String>,
    pub safe_to_answer: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverviewPacketEvidence {
    pub node_count: usize,
    pub edge_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub symbols: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverviewCodeExcerpt {
    pub path: String,
    pub line_start: u32,
    pub line_end: u32,
    pub text: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverviewPacketBudget {
    pub max_tokens: usize,
    pub budget_hit: bool,
    pub omitted_bytes: usize,
}

/// Packet id hash. Evidence ids are sorted so callers cannot change the order
/// and accidentally invalidate a packet.
pub fn packet_id(subject_id: &str, commit_sha: &str, evidence_ids: &[String]) -> String {
    use sha2::{Digest, Sha256};

    let mut evidence = evidence_ids.to_vec();
    evidence.sort();
    evidence.dedup();

    let mut hasher = Sha256::new();
    hasher.update(b"overview_packet");
    hasher.update(b"\x00");
    hasher.update(OVERVIEW_SCHEMA_VERSION.to_string().as_bytes());
    hasher.update(b"\x00");
    hasher.update(subject_id.as_bytes());
    hasher.update(b"\x00");
    hasher.update(commit_sha.as_bytes());
    for id in evidence {
        hasher.update(b"\x00");
        hasher.update(id.as_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
