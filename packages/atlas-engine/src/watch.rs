//! Phase 28 — Watch mode: file watcher, change detection, update pipeline.
//!
//! [`FileWatcher`] wraps the `notify` crate to recursively watch a repository
//! tree, filter ignored paths (`.git`, build dirs, …), and emit normalized
//! [`WatchEvent`]s that downstream phases can consume.
//!
//! [`WatchRunner`] owns a `FileWatcher` and drives the incremental update
//! pipeline: debounce rapid edits, coalesce duplicate paths, call
//! `update_graph` with explicit change sets, and expose per-batch results.
//!
//! Design constraints:
//! - 28.1: scope — auto-update on file changes, avoid full rebuild.
//! - 28.2: watcher — platform-recommended backend via `notify`, recursive,
//!   ignore `.git` / build dirs / `.atlas`, handle platform quirks.
//! - 28.3: change detection — detect create/modify/delete/rename; normalize
//!   duplicate event bursts; keep rename consistent with batch-update semantics.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};
use atlas_core::model::{ChangeType, ChangedFile};
use atlas_repo::{CanonicalRepoPath, DEFAULT_IGNORE_PATTERNS};
use camino::Utf8Path;
use notify::event::{ModifyKind, RenameMode};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher, recommended_watcher};

// ── Public types ─────────────────────────────────────────────────────────────

/// A normalized file-system change event emitted by [`FileWatcher`].
///
/// Paths are always repo-relative, forward-slash separated strings.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WatchEvent {
    /// A new file was created at the given repo-relative path.
    Created(String),
    /// An existing file was modified at the given repo-relative path.
    Modified(String),
    /// A file was deleted at the given repo-relative path.
    Deleted(String),
    /// A file was renamed: `(old_path, new_path)`, both repo-relative.
    Renamed(String, String),
}

// ── Extra ignore patterns added beyond DEFAULT_IGNORE_PATTERNS ───────────────

/// Paths that are always ignored by watch mode in addition to
/// [`DEFAULT_IGNORE_PATTERNS`].
const WATCH_EXTRA_IGNORE: &[&str] = &[".atlas"];

// ── FileWatcher ──────────────────────────────────────────────────────────────

/// Watches a repository directory tree for file-system changes.
///
/// Call [`FileWatcher::new`] to start watching, then call [`FileWatcher::drain`]
/// in a loop to retrieve normalized, deduplicated [`WatchEvent`]s.
pub struct FileWatcher {
    _watcher: RecommendedWatcher,
    rx: mpsc::Receiver<notify::Result<Event>>,
    repo_root: PathBuf,
    /// Prefixes that should be skipped, stored as `"dir/"` strings.
    ignore_prefixes: Vec<String>,
}

impl FileWatcher {
    /// Create a new watcher rooted at `repo_root`.
    ///
    /// Watches the repo directory recursively using the platform-recommended
    /// backend (inotify on Linux, FSEvents on macOS, ReadDirectoryChangesW on
    /// Windows). Ignores `.git`, all entries in
    /// [`DEFAULT_IGNORE_PATTERNS`], and `.atlas`.
    pub fn new(repo_root: &Path) -> Result<Self> {
        Self::with_extra_ignores(repo_root, &[])
    }

    /// Create a new watcher with additional ignore patterns beyond the defaults.
    ///
    /// `extra` entries follow the same format as [`DEFAULT_IGNORE_PATTERNS`]
    /// (plain directory/file names, not globs).
    pub fn with_extra_ignores(repo_root: &Path, extra: &[&str]) -> Result<Self> {
        let (tx, rx) = mpsc::channel();
        let mut watcher =
            recommended_watcher(tx).context("failed to create file-system watcher")?;

        watcher
            .watch(repo_root, RecursiveMode::Recursive)
            .with_context(|| format!("failed to watch '{}'", repo_root.display()))?;

        // Build prefix list: every ignored name becomes "name/" so that
        // `rel.starts_with(prefix)` correctly matches directory children.
        let mut ignore_prefixes: Vec<String> = DEFAULT_IGNORE_PATTERNS
            .iter()
            .chain(WATCH_EXTRA_IGNORE.iter())
            .chain(extra.iter())
            .map(|p| format!("{p}/"))
            .collect();
        // Also match the bare name without trailing slash for top-level files.
        for name in DEFAULT_IGNORE_PATTERNS
            .iter()
            .chain(WATCH_EXTRA_IGNORE.iter())
            .chain(extra.iter())
        {
            ignore_prefixes.push(name.to_string());
        }

        Ok(Self {
            _watcher: watcher,
            rx,
            repo_root: repo_root.to_path_buf(),
            ignore_prefixes,
        })
    }

    /// Drain pending events, blocking up to `timeout` for the first event.
    ///
    /// Returns a deduplicated list of [`WatchEvent`]s. On timeout (no events
    /// within `timeout`), returns an empty vec. Watcher errors are logged at
    /// WARN level and skipped so the caller's loop stays alive.
    pub fn drain(&self, timeout: Duration) -> Result<Vec<WatchEvent>> {
        let mut raw: Vec<Event> = Vec::new();

        // Block until first event or timeout.
        match self.rx.recv_timeout(timeout) {
            Ok(Ok(ev)) => raw.push(ev),
            Ok(Err(e)) => {
                tracing::warn!("file-watcher error: {e}");
                return Ok(Vec::new());
            }
            Err(mpsc::RecvTimeoutError::Timeout) => return Ok(Vec::new()),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(anyhow::anyhow!("file-watcher channel disconnected"));
            }
        }

        // Drain any already-buffered events without blocking for more.
        loop {
            match self.rx.try_recv() {
                Ok(Ok(ev)) => raw.push(ev),
                Ok(Err(e)) => tracing::warn!("file-watcher error: {e}"),
                Err(_) => break,
            }
        }

        self.normalize(raw)
    }

    // ── Internal helpers ─────────────────────────────────────────────────────

    /// Convert an absolute path to a repo-relative forward-slash string,
    /// returning `None` if the path is outside `repo_root` or is the root itself.
    fn repo_rel(&self, abs: &Path) -> Option<String> {
        let repo_root = Utf8Path::from_path(&self.repo_root)?;
        let abs = Utf8Path::from_path(abs)?;
        CanonicalRepoPath::from_watch_event_path(repo_root, abs)
            .ok()
            .map(|path| path.as_str().to_owned())
    }

    /// Return true if a repo-relative path should be ignored.
    fn is_ignored(&self, rel: &str) -> bool {
        self.ignore_prefixes
            .iter()
            .any(|p| rel.starts_with(p.as_str()) || rel == p.trim_end_matches('/'))
    }

    /// Normalize a batch of raw notify events into deduplicated [`WatchEvent`]s.
    ///
    /// Rename handling:
    /// - On platforms that emit a single two-path
    ///   `Modify(Name(Both))` event (Linux inotify with `IN_MOVED_FROM` +
    ///   `IN_MOVED_TO` paired by the kernel), the old and new paths are both
    ///   in `ev.paths`.
    /// - On platforms that emit two separate events (`Remove` then `Create`)
    ///   consecutive events for the same inode are paired here.
    fn normalize(&self, events: Vec<Event>) -> Result<Vec<WatchEvent>> {
        let mut out: Vec<WatchEvent> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();

        // Holds the last Remove path that might be the "from" side of a rename.
        let mut pending_remove: Option<String> = None;

        for ev in events {
            match ev.kind {
                // ── Create ────────────────────────────────────────────────────
                EventKind::Create(_) => {
                    for path in &ev.paths {
                        let Some(rel) = self.repo_rel(path) else {
                            continue;
                        };
                        if self.is_ignored(&rel) || path.is_dir() {
                            continue;
                        }
                        if let Some(old) = pending_remove.take() {
                            // Pair with pending Remove → Renamed.
                            let key = format!("R:{old}:{rel}");
                            if seen.insert(key) {
                                out.push(WatchEvent::Renamed(old, rel));
                            }
                        } else if seen.insert(format!("C:{rel}")) {
                            out.push(WatchEvent::Created(rel));
                        }
                    }
                }

                // ── Rename (single two-path event) ────────────────────────────
                EventKind::Modify(ModifyKind::Name(RenameMode::Both)) => {
                    // Flush dangling remove first.
                    if let Some(old) = pending_remove.take()
                        && seen.insert(format!("D:{old}"))
                    {
                        out.push(WatchEvent::Deleted(old));
                    }
                    if ev.paths.len() >= 2 {
                        let old = self.repo_rel(&ev.paths[0]);
                        let new_path = self.repo_rel(&ev.paths[1]);
                        if let (Some(old), Some(new_path)) = (old, new_path)
                            && !self.is_ignored(&new_path)
                        {
                            let key = format!("R:{old}:{new_path}");
                            if seen.insert(key) {
                                out.push(WatchEvent::Renamed(old, new_path));
                            }
                        }
                    }
                }

                // ── Modify ────────────────────────────────────────────────────
                EventKind::Modify(_) => {
                    // Flush dangling remove (wasn't paired with a Create).
                    if let Some(old) = pending_remove.take()
                        && seen.insert(format!("D:{old}"))
                    {
                        out.push(WatchEvent::Deleted(old));
                    }
                    for path in &ev.paths {
                        let Some(rel) = self.repo_rel(path) else {
                            continue;
                        };
                        if self.is_ignored(&rel) || path.is_dir() {
                            continue;
                        }
                        if seen.insert(format!("M:{rel}")) {
                            out.push(WatchEvent::Modified(rel));
                        }
                    }
                }

                // ── Remove ────────────────────────────────────────────────────
                EventKind::Remove(_) => {
                    // Flush any previous pending remove before accepting a new one.
                    if let Some(old) = pending_remove.take()
                        && seen.insert(format!("D:{old}"))
                    {
                        out.push(WatchEvent::Deleted(old));
                    }
                    for path in &ev.paths {
                        let Some(rel) = self.repo_rel(path) else {
                            continue;
                        };
                        if self.is_ignored(&rel) {
                            continue;
                        }
                        // Hold as pending — the next event may be a Create (rename).
                        pending_remove = Some(rel);
                    }
                }

                // ── All other event kinds (Access, Other, …) ──────────────────
                _ => {}
            }
        }

        // Flush any left-over pending remove with no matching Create.
        if let Some(old) = pending_remove.take()
            && seen.insert(format!("D:{old}"))
        {
            out.push(WatchEvent::Deleted(old));
        }

        Ok(out)
    }

    /// Drain all already-buffered events without blocking for more.
    ///
    /// Useful after a debounce sleep to collect any events that arrived during
    /// that window without waiting for new ones.
    pub fn drain_buffered(&self) -> Result<Vec<WatchEvent>> {
        let mut raw: Vec<Event> = Vec::new();
        loop {
            match self.rx.try_recv() {
                Ok(Ok(ev)) => raw.push(ev),
                Ok(Err(e)) => tracing::warn!("file-watcher error: {e}"),
                Err(_) => break,
            }
        }
        self.normalize(raw)
    }
}

// ── WatchRunner types ────────────────────────────────────────────────────────

/// Cumulative state for a running [`WatchRunner`].
#[derive(Debug, Default, Clone)]
pub struct WatchState {
    /// Number of debounced update batches processed.
    pub total_batches: u64,
    /// Cumulative count of file paths processed across all batches.
    pub total_files_updated: u64,
    /// Cumulative count of graph nodes updated across all batches.
    pub total_nodes_updated: u64,
    /// Cumulative count of errors (parse failures, I/O errors) encountered.
    pub total_errors: u64,
    /// Wall-clock time of the most recent completed batch.
    pub last_update_time: Option<SystemTime>,
}

/// Per-batch result emitted to the caller's callback after each update cycle.
#[derive(Debug, Clone)]
pub struct WatchBatchResult {
    /// Number of file paths included in this batch.
    pub files_updated: usize,
    /// Number of graph nodes written during this batch.
    pub nodes_updated: usize,
    /// Number of non-fatal errors encountered (parse failures, etc.).
    pub errors: usize,
    /// Wall time taken for the update pipeline, in milliseconds.
    pub elapsed_ms: u128,
    /// Human-readable error messages for any failures in this batch.
    pub error_messages: Vec<String>,
}

// ── WatchRunner ──────────────────────────────────────────────────────────────

/// Drives the real-time incremental update pipeline on top of [`FileWatcher`].
///
/// Call [`WatchRunner::run`] to block and process file-system changes
/// indefinitely (or until an unrecoverable error). Each debounce window
/// produces one [`WatchBatchResult`] passed to the caller's callback.
///
/// Design decisions (Phase 28.4–28.7):
/// - Debounce: wait `poll_timeout` for the first event, then sleep
///   `debounce` and drain any additional events that arrived during the
///   sleep. This batches rapid bursts without requiring a separate thread.
/// - Deduplication: tracked by destination path; a later event for the
///   same path overwrites an earlier one in the same batch.
/// - Single writer: `update_graph` is called synchronously per batch, so
///   there is exactly one SQLite writer at a time.
/// - Failure handling: parse errors and I/O errors are counted and logged at
///   WARN level; the watch loop continues after recoverable failures.
pub struct WatchRunner {
    watcher: FileWatcher,
    repo_root: camino::Utf8PathBuf,
    db_path: String,
    /// How long to sleep after the first event before draining the queue.
    pub debounce: Duration,
    /// Number of files per parallel parse batch (mirrors `UpdateOptions`).
    pub batch_size: usize,
    /// Accumulated statistics updated after every batch.
    pub state: WatchState,
}

impl WatchRunner {
    /// Create a new `WatchRunner` rooted at `repo_root`, writing to `db_path`.
    ///
    /// `debounce` controls the coalescing window (100–500 ms recommended).
    /// `batch_size` is passed through to the incremental update pipeline.
    pub fn new(
        repo_root: &Utf8Path,
        db_path: impl Into<String>,
        debounce: Duration,
        batch_size: usize,
    ) -> Result<Self> {
        let watcher = FileWatcher::new(repo_root.as_std_path())
            .context("cannot start file-system watcher")?;
        Ok(Self {
            watcher,
            repo_root: repo_root.to_owned(),
            db_path: db_path.into(),
            debounce,
            batch_size,
            state: WatchState::default(),
        })
    }

    /// Block forever, processing file-system events.
    ///
    /// After each debounce window, classifies accumulated events into a
    /// [`ChangedFile`] batch, calls `update_graph`, and invokes `on_batch`
    /// with the result. Returns only on an unrecoverable watcher channel error.
    pub fn run<F>(&mut self, mut on_batch: F) -> Result<()>
    where
        F: FnMut(&WatchBatchResult),
    {
        // Poll interval: block up to 1 s waiting for the first event. This
        // keeps the loop responsive without burning CPU when idle.
        let poll_timeout = Duration::from_secs(1);

        loop {
            // Block until the first event or poll timeout.
            let first_events = self.watcher.drain(poll_timeout)?;
            if first_events.is_empty() {
                continue;
            }

            // Debounce: wait for rapid follow-up changes within the window.
            std::thread::sleep(self.debounce);

            // Drain anything that arrived during the debounce sleep.
            let mut more_events = self.watcher.drain_buffered()?;
            let mut all_events = first_events;
            all_events.append(&mut more_events);

            // Convert to classified ChangedFile batch, deduplicating by key.
            let changes = events_to_changes(all_events);
            if changes.is_empty() {
                continue;
            }

            tracing::debug!("watch: processing {} change(s) in batch", changes.len());

            let result = self.apply_batch(changes);
            self.state.total_batches += 1;
            self.state.total_files_updated += result.files_updated as u64;
            self.state.total_nodes_updated += result.nodes_updated as u64;
            self.state.total_errors += result.errors as u64;
            self.state.last_update_time = Some(SystemTime::now());

            on_batch(&result);
        }
    }

    // ── Internals ─────────────────────────────────────────────────────────────

    /// Run `update_graph` for `changes`, returning a per-batch result.
    fn apply_batch(&self, changes: Vec<ChangedFile>) -> WatchBatchResult {
        let started = Instant::now();
        let files_updated = changes.len();
        let mut error_messages: Vec<String> = Vec::new();

        let opts = crate::update::UpdateOptions {
            fail_fast: false,
            batch_size: self.batch_size,
            target: crate::update::UpdateTarget::Batch(changes),
        };

        let (nodes_updated, errors) =
            match crate::update::update_graph(&self.repo_root, &self.db_path, &opts) {
                Ok(summary) => {
                    if summary.parse_errors > 0 {
                        tracing::warn!("watch: {} parse error(s) in batch", summary.parse_errors);
                    }
                    (summary.nodes_updated, summary.parse_errors)
                }
                Err(err) => {
                    let msg = format!("watch: update_graph failed: {err:#}");
                    tracing::warn!("{}", msg);
                    error_messages.push(msg);
                    (0, 1)
                }
            };

        WatchBatchResult {
            files_updated,
            nodes_updated,
            errors,
            elapsed_ms: started.elapsed().as_millis(),
            error_messages,
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Convert a batch of [`WatchEvent`]s to a deduplicated list of [`ChangedFile`]s.
///
/// Rules:
/// - Created / Modified → `ChangeType::Modified` keyed on destination path.
/// - Deleted            → `ChangeType::Deleted` keyed on path.
/// - Renamed(old, new)  → `ChangeType::Renamed` keyed on `new`; any pending
///   event for `old` is removed to avoid a spurious delete.
///
/// Later events for the same key overwrite earlier ones, implementing
/// deduplication of rapid-burst edits.
pub(crate) fn events_to_changes(events: Vec<WatchEvent>) -> Vec<ChangedFile> {
    // Map destination path → ChangedFile.
    let mut map: HashMap<String, ChangedFile> = HashMap::new();

    for ev in events {
        match ev {
            WatchEvent::Created(path) | WatchEvent::Modified(path) => {
                map.insert(
                    path.clone(),
                    ChangedFile {
                        path,
                        change_type: ChangeType::Modified,
                        old_path: None,
                    },
                );
            }
            WatchEvent::Deleted(path) => {
                map.insert(
                    path.clone(),
                    ChangedFile {
                        path,
                        change_type: ChangeType::Deleted,
                        old_path: None,
                    },
                );
            }
            WatchEvent::Renamed(old, new) => {
                // Drop any pending event that was keyed on the old path.
                map.remove(&old);
                map.insert(
                    new.clone(),
                    ChangedFile {
                        path: new,
                        change_type: ChangeType::Renamed,
                        old_path: Some(old),
                    },
                );
            }
        }
    }

    map.into_values().collect()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{CreateKind, ModifyKind, RemoveKind};

    fn make_event(kind: EventKind, paths: Vec<PathBuf>) -> Event {
        Event {
            kind,
            paths,
            attrs: Default::default(),
        }
    }

    fn watcher_for_root(root: &Path) -> FileWatcher {
        // Build a FileWatcher without actually starting the OS watcher so we
        // can hand-craft events for the normalize() method.
        let (_, rx) = mpsc::channel::<notify::Result<Event>>();
        FileWatcher {
            _watcher: recommended_watcher(|_: notify::Result<Event>| {})
                .expect("watcher creation failed in test"),
            rx,
            repo_root: root.to_path_buf(),
            ignore_prefixes: vec![
                ".git/".into(),
                ".git".into(),
                "target/".into(),
                "target".into(),
                ".atlas/".into(),
                ".atlas".into(),
            ],
        }
    }

    #[test]
    fn test_normalize_create() {
        let root = PathBuf::from("/repo");
        let watcher = watcher_for_root(&root);
        let events = vec![make_event(
            EventKind::Create(CreateKind::File),
            vec![root.join("src/lib.rs")],
        )];
        let result = watcher.normalize(events).unwrap();
        assert_eq!(result, vec![WatchEvent::Created("src/lib.rs".into())]);
    }

    #[test]
    fn test_normalize_modify() {
        let root = PathBuf::from("/repo");
        let watcher = watcher_for_root(&root);
        let events = vec![make_event(
            EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
            vec![root.join("src/main.rs")],
        )];
        let result = watcher.normalize(events).unwrap();
        assert_eq!(result, vec![WatchEvent::Modified("src/main.rs".into())]);
    }

    #[test]
    fn test_normalize_delete() {
        let root = PathBuf::from("/repo");
        let watcher = watcher_for_root(&root);
        let events = vec![make_event(
            EventKind::Remove(RemoveKind::File),
            vec![root.join("src/old.rs")],
        )];
        let result = watcher.normalize(events).unwrap();
        assert_eq!(result, vec![WatchEvent::Deleted("src/old.rs".into())]);
    }

    #[test]
    fn test_normalize_rename_two_path_event() {
        let root = PathBuf::from("/repo");
        let watcher = watcher_for_root(&root);
        let events = vec![make_event(
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            vec![root.join("src/old.rs"), root.join("src/new.rs")],
        )];
        let result = watcher.normalize(events).unwrap();
        assert_eq!(
            result,
            vec![WatchEvent::Renamed(
                "src/old.rs".into(),
                "src/new.rs".into()
            )]
        );
    }

    #[test]
    fn test_normalize_rename_two_event_sequence() {
        let root = PathBuf::from("/repo");
        let watcher = watcher_for_root(&root);
        // Platform emits Remove then Create for renames.
        let events = vec![
            make_event(
                EventKind::Remove(RemoveKind::File),
                vec![root.join("src/old.rs")],
            ),
            make_event(
                EventKind::Create(CreateKind::File),
                vec![root.join("src/new.rs")],
            ),
        ];
        let result = watcher.normalize(events).unwrap();
        assert_eq!(
            result,
            vec![WatchEvent::Renamed(
                "src/old.rs".into(),
                "src/new.rs".into()
            )]
        );
    }

    #[test]
    fn test_ignored_git_path_skipped() {
        let root = PathBuf::from("/repo");
        let watcher = watcher_for_root(&root);
        let events = vec![make_event(
            EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
            vec![root.join(".git/COMMIT_EDITMSG")],
        )];
        let result = watcher.normalize(events).unwrap();
        assert!(result.is_empty(), "events under .git must be ignored");
    }

    #[test]
    fn test_ignored_target_path_skipped() {
        let root = PathBuf::from("/repo");
        let watcher = watcher_for_root(&root);
        let events = vec![make_event(
            EventKind::Create(CreateKind::File),
            vec![root.join("target/debug/app")],
        )];
        let result = watcher.normalize(events).unwrap();
        assert!(result.is_empty(), "events under target/ must be ignored");
    }

    #[test]
    fn test_deduplicate_same_modify() {
        let root = PathBuf::from("/repo");
        let watcher = watcher_for_root(&root);
        // Two Modify events for the same file (rapid-save burst).
        let events = vec![
            make_event(
                EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
                vec![root.join("src/lib.rs")],
            ),
            make_event(
                EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Content)),
                vec![root.join("src/lib.rs")],
            ),
        ];
        let result = watcher.normalize(events).unwrap();
        assert_eq!(result.len(), 1, "duplicate modifies must be collapsed");
        assert_eq!(result[0], WatchEvent::Modified("src/lib.rs".into()));
    }

    // ── events_to_changes tests ───────────────────────────────────────────────

    #[test]
    fn events_to_changes_created_maps_to_modified() {
        let changes = events_to_changes(vec![WatchEvent::Created("src/foo.rs".into())]);
        assert_eq!(changes.len(), 1);
        assert!(matches!(changes[0].change_type, ChangeType::Modified));
        assert_eq!(changes[0].path, "src/foo.rs");
    }

    #[test]
    fn events_to_changes_deleted() {
        let changes = events_to_changes(vec![WatchEvent::Deleted("src/gone.rs".into())]);
        assert_eq!(changes.len(), 1);
        assert!(matches!(changes[0].change_type, ChangeType::Deleted));
    }

    #[test]
    fn events_to_changes_renamed_includes_old_path() {
        let changes = events_to_changes(vec![WatchEvent::Renamed(
            "src/old.rs".into(),
            "src/new.rs".into(),
        )]);
        assert_eq!(changes.len(), 1);
        assert!(matches!(changes[0].change_type, ChangeType::Renamed));
        assert_eq!(changes[0].path, "src/new.rs");
        assert_eq!(changes[0].old_path.as_deref(), Some("src/old.rs"));
    }

    #[test]
    fn events_to_changes_deduplicates_same_path() {
        let changes = events_to_changes(vec![
            WatchEvent::Modified("src/lib.rs".into()),
            WatchEvent::Modified("src/lib.rs".into()),
        ]);
        assert_eq!(changes.len(), 1, "duplicate modify events must collapse");
    }

    #[test]
    fn events_to_changes_rename_drops_stale_old_event() {
        // First event is a modify on "old", then a rename old→new in the same batch.
        // The old path's modify should be dropped; only the rename survives.
        let changes = events_to_changes(vec![
            WatchEvent::Modified("src/old.rs".into()),
            WatchEvent::Renamed("src/old.rs".into(), "src/new.rs".into()),
        ]);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].path, "src/new.rs");
        assert!(matches!(changes[0].change_type, ChangeType::Renamed));
    }

    #[test]
    fn repo_rel_canonicalizes_dot_segments() {
        let root = Path::new("/repo");
        let watcher = watcher_for_root(root);
        let abs = Path::new("/repo/src/./nested/../lib.rs");

        assert_eq!(watcher.repo_rel(abs).as_deref(), Some("src/lib.rs"));
    }
}
