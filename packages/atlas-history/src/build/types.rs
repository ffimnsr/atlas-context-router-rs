use super::EdgeKey;
use super::*;

#[derive(Debug, Clone)]
pub(super) struct CachedFileGraph {
    pub(super) language: Option<String>,
    pub(super) size: Option<i64>,
    pub(super) node_count: usize,
    pub(super) edge_count: usize,
    pub(super) qualified_names: Vec<String>,
    pub(super) edge_keys: Vec<EdgeKey>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct BuildRunCache {
    pub(super) blob_bytes: BTreeMap<String, Vec<u8>>,
    pub(super) blob_graphs: BTreeMap<String, CachedFileGraph>,
    pub(super) binary_blobs: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub(super) struct SnapshotFileMembership {
    pub(super) file_path: String,
    pub(super) file_hash: String,
    pub(super) qualified_names: Vec<String>,
    pub(super) edge_keys: Vec<EdgeKey>,
}

/// Work item for the parallel parse phase.  Carries everything needed to call
/// `registry.parse()` without touching the SQLite store.
pub(super) struct ParseWork {
    pub(super) file_index: usize,
    pub(super) file_hash: String,
    pub(super) rel_path: String,
    pub(super) bytes: Vec<u8>,
    pub(super) orig_language: Option<String>,
}

/// Result of one parallel `registry.parse()` call.
pub(super) struct ParseOutcome {
    pub(super) file_index: usize,
    pub(super) file_hash: String,
    pub(super) rel_path: String,
    pub(super) orig_language: Option<String>,
    pub(super) size: i64,
    /// `None` means the parser returned no output (SkippedNoParserOutput).
    pub(super) parsed: Option<ParsedFile>,
}

pub(super) struct BuildContext<'a, P: FnMut(BuildProgressEvent)> {
    pub(super) repo: &'a Path,
    pub(super) repo_id: i64,
    pub(super) store: &'a Store,
    pub(super) registry: &'a ParserRegistry,
    pub(super) indexed_ref: Option<&'a str>,
    pub(super) cache: &'a mut BuildRunCache,
    pub(super) summary: &'a mut BuildSummary,
    pub(super) progress: &'a mut P,
    pub(super) commit_index: usize,
    pub(super) total_commits: usize,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BuildFileProgressKind {
    Reused,
    Parsed,
    SkippedUnsupported,
    SkippedBinary,
    SkippedMissing,
    SkippedNoParserOutput,
    PersistError,
}

impl fmt::Display for BuildFileProgressKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Reused => "reused",
            Self::Parsed => "parsed",
            Self::SkippedUnsupported => "skipped unsupported",
            Self::SkippedBinary => "skipped binary",
            Self::SkippedMissing => "skipped missing",
            Self::SkippedNoParserOutput => "skipped empty-parse",
            Self::PersistError => "persist error",
        })
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum BuildProgressEvent {
    RunStarted {
        total_commits: usize,
    },
    RunPhaseChanged {
        message: String,
    },
    CommitStarted {
        commit_index: usize,
        total_commits: usize,
        commit_sha: String,
        total_files: usize,
    },
    CommitSkipped {
        commit_index: usize,
        total_commits: usize,
        commit_sha: String,
    },
    FileProcessed {
        commit_index: usize,
        total_commits: usize,
        commit_sha: String,
        file_index: usize,
        total_files: usize,
        file_path: String,
        outcome: BuildFileProgressKind,
    },
    CommitPersistStepStarted {
        commit_index: usize,
        total_commits: usize,
        commit_sha: String,
        step_index: usize,
        total_steps: usize,
        step: BuildPersistProgressKind,
    },
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum BuildPersistProgressKind {
    CommitMetadata,
    SnapshotRows,
    SnapshotMembership,
    TransactionCommit,
}

impl BuildPersistProgressKind {
    pub const fn total_steps() -> usize {
        4
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::CommitMetadata => "saving commit metadata",
            Self::SnapshotRows => "saving snapshot rows",
            Self::SnapshotMembership => "storing snapshot membership",
            Self::TransactionCommit => "committing database transaction",
        }
    }

    pub(crate) const fn step_index(self) -> usize {
        match self {
            Self::CommitMetadata => 1,
            Self::SnapshotRows => 2,
            Self::SnapshotMembership => 3,
            Self::TransactionCommit => 4,
        }
    }
}

/// Summary produced by [`build_historical_graph`].
#[derive(Debug, Clone, Default, Serialize)]
pub struct BuildSummary {
    /// Total commits enumerated from the selector.
    pub commits_processed: usize,
    /// Commits skipped because a snapshot already existed.
    pub commits_already_indexed: usize,
    /// Files whose graph was loaded from existing storage (no parse needed).
    pub files_reused: usize,
    /// Files newly parsed during this build run.
    pub files_parsed: usize,
    /// Nodes loaded from already-indexed blobs (reuse count).
    pub nodes_reused: usize,
    /// Nodes written from newly parsed blobs.
    pub nodes_written: usize,
    /// Edges written from newly parsed blobs.
    pub edges_written: usize,
    /// Files skipped as binary or unsupported extension.
    pub files_skipped: usize,
    /// Per-commit or per-file errors that did not abort the run.
    pub errors: Vec<IngestError>,
    /// Non-fatal diagnostics the caller should surface to the user.
    pub warnings: Vec<String>,
    /// Wall-clock duration for the entire build.
    pub elapsed_secs: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct SnapshotRebuildSummary {
    pub commit_sha: String,
    pub replaced_snapshot_id: i64,
    pub rebuilt_snapshot_id: i64,
    pub reclaimed_file_hashes: u64,
    pub reclaimed_historical_nodes: u64,
    pub reclaimed_historical_edges: u64,
    pub build: BuildSummary,
    pub lifecycle: LifecycleSummary,
}
