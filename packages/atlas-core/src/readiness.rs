//! Canonical graph readiness record and execution-state safety gate.
//!
//! # Readiness record
//!
//! [`GraphReadiness`] is the single source of truth for whether the Atlas
//! graph is ready, searchable, and current enough to use.  All CLI commands,
//! MCP tools, and adapters must derive their readiness answers from this
//! record instead of computing their own readiness logic.
//!
//! # Execution safety state
//!
//! [`GraphExecutionState`] classifies the graph as `fresh`, `stale`,
//! `partial`, `corrupt`, or `missing`.  Callers use
//! [`GraphReadiness::check_tool`] to decide whether their tool class is
//! allowed to proceed under the current state and the supplied
//! [`ReadinessOverride`] flags.  The result is a [`ReadinessVerdict`].
//!
//! # Readiness dimensions
//!
//! | Dimension | Field | Meaning |
//! |---|---|---|
//! | Built vs. missing | `graph_built` | Index has been built and contains content |
//! | Queryable vs. blocked | `graph_queryable` | Graph can serve queries right now |
//! | Current vs. stale | `graph_current` | Graph is up-to-date with the working tree |
//! | Corrupt vs. merely stale | `integrity_state` | Whether the database has integrity issues |
//! | Graph vs. retrieval | `error_code` | Most significant issue across both surfaces |
//! | Safety gate | `execution_state` | Canonical safety state for feature-gating |

use serde::{Deserialize, Serialize};

use crate::health::{
    GraphHealthInput, graph_health_error_message, graph_health_error_suggestions,
    is_schema_mismatch_error, select_graph_health_error_code,
};

/// Integrity classification of the graph database.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrityState {
    /// No integrity issues detected.
    #[default]
    Clean,
    /// Persisted path rows are not canonical; rebuild required.
    NoncanonicalPaths,
    /// SQLite schema does not match this Atlas build; rebuild required.
    SchemaMismatch,
    /// Graph has SQLite integrity errors, orphan nodes, or dangling edges.
    Corrupt,
}

impl IntegrityState {
    /// Machine-readable string label for this state.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::NoncanonicalPaths => "noncanonical_paths",
            Self::SchemaMismatch => "schema_mismatch",
            Self::Corrupt => "corrupt",
        }
    }

    /// Returns `true` when no integrity issues are present.
    pub fn is_clean(&self) -> bool {
        matches!(self, Self::Clean)
    }
}

impl std::fmt::Display for IntegrityState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ── Execution safety state ────────────────────────────────────────────────────

/// Canonical graph execution safety state.
///
/// Derived from [`GraphReadiness`] to give callers one simple value to branch
/// on before performing graph-backed operations.
///
/// # Priority (worst → best)
/// `corrupt` > `missing` > `partial` > `stale` > `fresh`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphExecutionState {
    /// Graph is built, queryable, current, and integrity-clean.
    /// Full graph-backed features enabled.
    Fresh,
    /// Graph is queryable but behind graph-relevant working-tree changes.
    /// Warn and allow with freshness metadata; stale override not required
    /// by default policy unless operator opts in to strict mode.
    Stale,
    /// Graph is queryable but the build finished in degraded mode (budget hit
    /// or partial indexing).  Allow limited features only; block answers
    /// requiring complete graph facts unless `allow_partial` is set.
    Partial,
    /// Graph has SQLite integrity errors, schema mismatch, orphan nodes, or
    /// dangling edges.  Block all graph-backed answers; no override allowed.
    Corrupt,
    /// Graph has not been built yet or the database is absent.
    /// Fail with a build suggestion.
    Missing,
}

impl GraphExecutionState {
    /// Machine-readable string label for this state.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Stale => "stale",
            Self::Partial => "partial",
            Self::Corrupt => "corrupt",
            Self::Missing => "missing",
        }
    }

    /// Returns `true` for states where the graph can be queried at all.
    pub fn is_queryable(self) -> bool {
        matches!(self, Self::Fresh | Self::Stale | Self::Partial)
    }
}

impl std::fmt::Display for GraphExecutionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which class of tool is requesting a readiness check.
///
/// The class determines which execution states block the tool and which
/// produce a warning-only response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphToolRequirement {
    /// Diagnostics commands (status, doctor, db_check): always allowed in
    /// any execution state, including corrupt and missing.
    Diagnostics,
    /// Symbol lookup (query_graph, batch_query_graph, symbol_neighbors,
    /// resolve_symbol, explain_query): allowed unless corrupt or missing.
    /// Stale and partial graphs return results with a freshness/degraded
    /// warning in `safe_to_answer`.
    SymbolLookup,
    /// Graph traversal (traverse_graph, cross_file_links): blocked when
    /// the graph is partial (missing edges can make the answer unsafe) or
    /// corrupt/missing.
    Traversal,
    /// Impact, review, context, and analysis flows (get_impact_radius,
    /// get_review_context, get_minimal_context, get_context, analyze_*,
    /// explain_change): blocked in partial, corrupt, or missing states
    /// unless completeness requirements are explicitly overridden.
    Analysis,
}

/// Caller-supplied override flags.
///
/// A `default()` instance applies no overrides (strictest policy).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReadinessOverride {
    /// Allow graph-backed operations when the graph is stale.  Maps to the
    /// `--allow-stale` CLI flag and MCP `allow_stale=true` parameter.
    pub allow_stale: bool,
    /// Allow limited graph-backed operations when the graph is partial
    /// (degraded build).  Maps to `--allow-partial` / MCP `allow_partial=true`.
    pub allow_partial: bool,
}

/// Outcome of a [`GraphReadiness::check_tool`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadinessVerdict {
    /// Tool may proceed.  `safe_to_answer` indicates whether the graph state
    /// is clean enough to claim full answer fidelity.  A `warning` message is
    /// present when proceeding under stale or partial conditions.
    Allowed {
        execution_state: GraphExecutionState,
        /// `true` when the graph is fresh; `false` under stale/partial.
        safe_to_answer: bool,
        /// Human-readable caveat, present when `safe_to_answer` is `false`.
        warning: Option<String>,
    },
    /// Tool must not proceed.  Contains a reason string and suggestions.
    Blocked {
        execution_state: GraphExecutionState,
        reason: String,
        suggestions: Vec<String>,
    },
}

impl ReadinessVerdict {
    /// Returns `true` when the tool is allowed to proceed.
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allowed { .. })
    }

    /// Returns `true` when the verdict is `Blocked`.
    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::Blocked { .. })
    }

    /// Returns the execution state regardless of verdict kind.
    pub fn execution_state(&self) -> GraphExecutionState {
        match self {
            Self::Allowed {
                execution_state, ..
            } => *execution_state,
            Self::Blocked {
                execution_state, ..
            } => *execution_state,
        }
    }
}

/// Canonical graph readiness record.
///
/// Derive this once per request via [`GraphReadiness::derive`] and pass it
/// through to every subsystem that needs to decide whether to proceed.
/// Do not compute readiness from raw `Store::open` results or build state
/// strings in callers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphReadiness {
    /// Absolute path to the repo root.
    pub repo_root: String,
    /// Absolute path to the graph database file.
    pub db_path: String,
    /// Whether the database file exists on disk.
    pub db_exists: bool,
    /// Error encountered when opening the database, if any.
    pub db_open_error: Option<String>,
    /// Raw build lifecycle state: `"built"`, `"building"`, `"degraded"`,
    /// `"build_failed"`, or `None` when no build record exists.
    pub build_state: Option<String>,
    /// Last error recorded by the build lifecycle, if any.
    pub build_last_error: Option<String>,
    /// Whether the graph has been built and contains indexable content.
    ///
    /// `true` when `build_state` is `"built"`, or when `build_state` is
    /// absent but the graph contains nodes, edges, or file records.
    pub graph_built: bool,
    /// Whether the graph can currently serve queries.
    ///
    /// `true` when `graph_built` is `true`, there are no db open or query
    /// errors, and `integrity_state` is [`IntegrityState::Clean`].
    pub graph_queryable: bool,
    /// Whether graph facts are current with the working tree.
    ///
    /// `true` when `graph_queryable` is `true` and `stale_index` is `false`.
    pub graph_current: bool,
    /// Whether there are graph-relevant working-tree changes not yet indexed.
    pub stale_index: bool,
    /// File paths with pending graph-relevant changes.
    pub pending_graph_changes: Vec<String>,
    /// Integrity classification of the graph database.
    pub integrity_state: IntegrityState,
    /// Short machine-readable error code for the most significant readiness
    /// issue, or `"none"` when the graph is ready.
    pub error_code: String,
    /// Human-readable message describing the readiness state.
    pub message: String,
    /// Suggested actions to resolve any readiness issue.
    pub suggestions: Vec<String>,
    /// ISO-8601 timestamp of the last successful index run, if any.
    pub last_indexed_at: Option<String>,
    /// Number of files in the current graph index.
    pub indexed_file_count: i64,
    /// Canonical execution safety state derived from all readiness dimensions.
    ///
    /// Use this single field for feature-gating decisions instead of
    /// inspecting individual boolean flags.  Pass it to
    /// [`GraphReadiness::check_tool`] to get a [`ReadinessVerdict`].
    pub execution_state: GraphExecutionState,
}

impl GraphReadiness {
    /// Returns `true` when the graph is fully ready: built, queryable, and
    /// current with no integrity issues.
    pub fn is_ok(&self) -> bool {
        self.error_code == "none" && self.graph_built
    }

    /// Returns the [`ReadinessVerdict`] for the given tool class and override
    /// flags.
    ///
    /// Callers should check the verdict before performing any graph-backed
    /// operation.  Diagnostics tools always receive `Allowed`.  Corrupt and
    /// missing states block all non-diagnostic operations with no override
    /// path.
    pub fn check_tool(
        &self,
        requirement: GraphToolRequirement,
        overrides: ReadinessOverride,
    ) -> ReadinessVerdict {
        use GraphExecutionState as S;
        use GraphToolRequirement as R;
        let state = self.execution_state;

        // Diagnostics are always allowed, regardless of graph state.
        if requirement == R::Diagnostics {
            return ReadinessVerdict::Allowed {
                execution_state: state,
                safe_to_answer: true,
                warning: None,
            };
        }

        match state {
            S::Corrupt => ReadinessVerdict::Blocked {
                execution_state: state,
                reason: "graph has integrity errors and cannot serve queries".to_owned(),
                suggestions: self.suggestions.clone(),
            },
            S::Missing => ReadinessVerdict::Blocked {
                execution_state: state,
                reason: "graph has not been built yet".to_owned(),
                suggestions: self.suggestions.clone(),
            },
            S::Partial => match requirement {
                R::Diagnostics => unreachable!("handled above"),
                R::SymbolLookup => {
                    if overrides.allow_partial {
                        ReadinessVerdict::Allowed {
                            execution_state: state,
                            safe_to_answer: false,
                            warning: Some(
                                "graph is in partial (degraded) state; results may be incomplete"
                                    .to_owned(),
                            ),
                        }
                    } else {
                        ReadinessVerdict::Blocked {
                            execution_state: state,
                            reason:
                                "graph is in partial (degraded) state; symbol lookup results may \
                                 be incomplete — pass allow_partial=true to override"
                                    .to_owned(),
                            suggestions: self.suggestions.clone(),
                        }
                    }
                }
                R::Traversal | R::Analysis => ReadinessVerdict::Blocked {
                    execution_state: state,
                    reason: format!(
                        "graph is in partial (degraded) state; {} requires a complete graph",
                        match requirement {
                            R::Traversal => "traversal",
                            R::Analysis => "analysis",
                            _ => "this tool",
                        }
                    ),
                    suggestions: self.suggestions.clone(),
                },
            },
            S::Stale => {
                // Default policy allows stale reads with a warning.
                // Callers may enforce strict mode by treating the warning as an error.
                let warn = "graph is stale; working-tree changes are not yet indexed".to_owned();
                if overrides.allow_stale || matches!(requirement, R::SymbolLookup) {
                    ReadinessVerdict::Allowed {
                        execution_state: state,
                        safe_to_answer: false,
                        warning: Some(warn),
                    }
                } else {
                    // For traversal/analysis the default policy also allows
                    // stale reads (unlike partial) but emits a warning.
                    ReadinessVerdict::Allowed {
                        execution_state: state,
                        safe_to_answer: false,
                        warning: Some(warn),
                    }
                }
            }
            S::Fresh => ReadinessVerdict::Allowed {
                execution_state: state,
                safe_to_answer: true,
                warning: None,
            },
        }
    }
}

/// All raw inputs required to derive a [`GraphReadiness`] record.
pub struct GraphReadinessInput<'a> {
    /// Absolute repo root path.
    pub repo_root: &'a str,
    /// Absolute graph database path.
    pub db_path: &'a str,
    /// Whether the database file exists on disk.
    pub db_exists: bool,
    /// Error string from opening the database, if any.
    pub db_open_error: Option<&'a str>,
    /// Raw build lifecycle state string, if a build record exists.
    pub build_state: Option<&'a str>,
    /// Last build error string recorded in the build lifecycle, if any.
    pub build_last_error: Option<&'a str>,
    /// Error string from querying graph content (e.g. stats), if any.
    pub graph_error: Option<&'a str>,
    /// Graph-relevant changed files that have not been indexed yet.
    pub pending_graph_changes: &'a [String],
    /// Number of files in the current index.
    pub indexed_file_count: i64,
    /// Whether the graph contains any content (nodes, edges, or file records).
    ///
    /// Used to detect a built graph when no build state record exists.
    pub graph_has_content: bool,
    /// ISO-8601 timestamp of the last successful index run, if any.
    pub last_indexed_at: Option<&'a str>,
    /// Whether the retrieval/content index is unavailable.
    pub retrieval_unavailable: bool,
}

impl GraphReadiness {
    /// Derive a canonical [`GraphReadiness`] from raw inputs.
    ///
    /// This is the only function that may produce a `GraphReadiness`.  All
    /// other subsystems must call this instead of duplicating the derivation.
    pub fn derive(input: GraphReadinessInput<'_>) -> Self {
        let integrity_state = derive_integrity_state(input.db_open_error, input.graph_error);

        // A graph is considered built when the lifecycle recorded "built", or
        // when no lifecycle record exists but the graph already has content
        // (e.g. a legacy index from before build-state tracking was added).
        let graph_built = input.build_state == Some("built")
            || (input.build_state.is_none()
                && input.db_open_error.is_none()
                && input.graph_error.is_none()
                && input.graph_has_content);

        // Queryable requires: built, no open/query errors, and integrity clean.
        let graph_queryable = graph_built
            && input.db_open_error.is_none()
            && input.graph_error.is_none()
            && integrity_state.is_clean();

        let stale_index = graph_built && !input.pending_graph_changes.is_empty();
        let graph_current = graph_queryable && !stale_index;

        // Combine db open and query errors for error code selection.
        let combined_error = input.db_open_error.or(input.graph_error);

        let error_code = select_graph_health_error_code(GraphHealthInput {
            db_exists: input.db_exists,
            graph_error: combined_error,
            build_state: input.build_state,
            stale_index,
            retrieval_unavailable: input.retrieval_unavailable,
        });
        let message = graph_health_error_message(error_code).to_owned();
        let suggestions = graph_health_error_suggestions(error_code)
            .iter()
            .map(|s| (*s).to_owned())
            .collect();

        // Derive canonical execution safety state.
        // Priority (worst → best): Corrupt > Missing > Partial > Stale > Fresh.
        let execution_state = derive_execution_state(
            &integrity_state,
            graph_built,
            input.build_state,
            input.graph_has_content,
            stale_index,
        );

        GraphReadiness {
            repo_root: input.repo_root.to_owned(),
            db_path: input.db_path.to_owned(),
            db_exists: input.db_exists,
            db_open_error: input.db_open_error.map(str::to_owned),
            build_state: input.build_state.map(str::to_owned),
            build_last_error: input.build_last_error.map(str::to_owned),
            graph_built,
            graph_queryable,
            graph_current,
            stale_index,
            pending_graph_changes: input.pending_graph_changes.to_vec(),
            integrity_state,
            error_code: error_code.to_owned(),
            message,
            suggestions,
            last_indexed_at: input.last_indexed_at.map(str::to_owned),
            indexed_file_count: input.indexed_file_count,
            execution_state,
        }
    }
}

fn derive_integrity_state(
    db_open_error: Option<&str>,
    graph_error: Option<&str>,
) -> IntegrityState {
    let combined = db_open_error.or(graph_error);
    let Some(err) = combined else {
        return IntegrityState::Clean;
    };

    if is_schema_mismatch_error(err) {
        return IntegrityState::SchemaMismatch;
    }

    let lower = err.to_ascii_lowercase();
    if lower.contains("noncanonical_path") {
        return IntegrityState::NoncanonicalPaths;
    }

    IntegrityState::Corrupt
}

/// Derive the [`GraphExecutionState`] from the already-computed readiness
/// sub-values.
///
/// Priority (worst → best): Corrupt > Missing > Partial > Stale > Fresh.
///
/// A degraded build with existing content is classified as `Partial` rather
/// than `Missing` because the graph is partially answerable even though the
/// build lifecycle recorded a degraded finish.
fn derive_execution_state(
    integrity_state: &IntegrityState,
    graph_built: bool,
    build_state: Option<&str>,
    graph_has_content: bool,
    stale_index: bool,
) -> GraphExecutionState {
    if !integrity_state.is_clean() {
        return GraphExecutionState::Corrupt;
    }
    // Degraded build with content is partially queryable even though
    // `graph_built` is false in the boolean dimensions.
    if build_state == Some("degraded") && graph_has_content {
        return GraphExecutionState::Partial;
    }
    if !graph_built {
        return GraphExecutionState::Missing;
    }
    if stale_index {
        return GraphExecutionState::Stale;
    }
    GraphExecutionState::Fresh
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Construct a fully-healthy base input with no pending changes.
    fn healthy_input() -> GraphReadinessInput<'static> {
        GraphReadinessInput {
            repo_root: "/repo",
            db_path: "/repo/.atlas/worldtree.db",
            db_exists: true,
            db_open_error: None,
            build_state: Some("built"),
            build_last_error: None,
            graph_error: None,
            pending_graph_changes: &[],
            indexed_file_count: 42,
            graph_has_content: true,
            last_indexed_at: Some("2024-01-01T00:00:00Z"),
            retrieval_unavailable: false,
        }
    }

    // ── built / clean / current ─────────────────────────────────────────────

    #[test]
    fn fresh_graph_is_fully_ready() {
        let r = GraphReadiness::derive(healthy_input());
        assert!(r.graph_built, "graph_built");
        assert!(r.graph_queryable, "graph_queryable");
        assert!(r.graph_current, "graph_current");
        assert!(!r.stale_index, "stale_index");
        assert!(r.pending_graph_changes.is_empty());
        assert_eq!(r.integrity_state, IntegrityState::Clean);
        assert_eq!(r.error_code, "none");
        assert!(r.is_ok());
        assert_eq!(r.indexed_file_count, 42);
        assert_eq!(r.last_indexed_at.as_deref(), Some("2024-01-01T00:00:00Z"));
        assert_eq!(r.repo_root, "/repo");
        assert_eq!(r.db_path, "/repo/.atlas/worldtree.db");
    }

    // ── missing graph db ────────────────────────────────────────────────────

    #[test]
    fn missing_db_marks_not_built_not_queryable() {
        let r = GraphReadiness::derive(GraphReadinessInput {
            db_exists: false,
            build_state: None,
            graph_has_content: false,
            ..healthy_input()
        });
        assert!(!r.db_exists);
        assert!(!r.graph_built);
        assert!(!r.graph_queryable);
        assert!(!r.graph_current);
        assert_eq!(r.integrity_state, IntegrityState::Clean);
        assert_eq!(r.error_code, "missing_graph_db");
        assert!(!r.is_ok());
    }

    // ── stale index ─────────────────────────────────────────────────────────

    #[test]
    fn pending_changes_make_graph_stale_but_queryable() {
        let pending = vec!["src/main.rs".to_owned(), "src/lib.rs".to_owned()];
        let r = GraphReadiness::derive(GraphReadinessInput {
            pending_graph_changes: &pending,
            ..healthy_input()
        });
        assert!(r.graph_built);
        assert!(r.graph_queryable, "queryable even when stale");
        assert!(!r.graph_current, "not current when stale");
        assert!(r.stale_index);
        assert_eq!(r.pending_graph_changes.len(), 2);
        assert_eq!(r.error_code, "stale_index");
        assert!(!r.is_ok());
    }

    // ── build lifecycle states ───────────────────────────────────────────────

    #[test]
    fn interrupted_build_state_marks_not_built() {
        let r = GraphReadiness::derive(GraphReadinessInput {
            build_state: Some("building"),
            graph_has_content: false,
            ..healthy_input()
        });
        assert!(!r.graph_built, "in-progress build is not built");
        assert!(!r.graph_queryable);
        assert!(!r.graph_current);
        assert_eq!(r.error_code, "interrupted_build");
        assert_eq!(r.build_state.as_deref(), Some("building"));
    }

    #[test]
    fn degraded_build_marks_not_built() {
        let r = GraphReadiness::derive(GraphReadinessInput {
            build_state: Some("degraded"),
            graph_has_content: false,
            ..healthy_input()
        });
        assert!(!r.graph_built);
        assert!(!r.graph_queryable);
        assert_eq!(r.error_code, "degraded_build");
    }

    #[test]
    fn failed_build_marks_not_built() {
        let last_err = Some("parse error on file foo.rs");
        let r = GraphReadiness::derive(GraphReadinessInput {
            build_state: Some("build_failed"),
            build_last_error: last_err,
            graph_has_content: false,
            ..healthy_input()
        });
        assert!(!r.graph_built);
        assert!(!r.graph_queryable);
        assert_eq!(r.error_code, "failed_build");
        assert_eq!(r.build_last_error.as_deref(), last_err);
    }

    // ── graph built inferred from content (no build record) ─────────────────

    #[test]
    fn legacy_graph_with_content_but_no_build_record_is_treated_as_built() {
        let r = GraphReadiness::derive(GraphReadinessInput {
            build_state: None,
            graph_has_content: true,
            ..healthy_input()
        });
        assert!(r.graph_built, "inferred from content");
        assert!(r.graph_queryable);
        assert!(r.graph_current);
        assert_eq!(r.error_code, "none");
    }

    #[test]
    fn no_build_record_and_no_content_is_not_built() {
        // db_exists=true but nothing has been indexed yet.  The health layer
        // does not treat an empty-but-present DB as "missing_graph_db"; the
        // error_code is "none" but graph_built=false so is_ok() is false.
        let r = GraphReadiness::derive(GraphReadinessInput {
            build_state: None,
            graph_has_content: false,
            ..healthy_input()
        });
        assert!(!r.graph_built, "no content means not built");
        assert!(!r.graph_queryable);
        assert!(!r.graph_current);
        // health.rs returns "none" when db_exists=true and no error: the DB
        // file is present but unpopulated, which is distinct from missing.
        assert_eq!(r.error_code, "none");
        assert!(!r.is_ok(), "is_ok requires graph_built");
    }

    // ── integrity states ─────────────────────────────────────────────────────

    #[test]
    fn schema_mismatch_error_yields_schema_mismatch_integrity() {
        let err = "table nodes has no column named extra_json";
        let r = GraphReadiness::derive(GraphReadinessInput {
            db_open_error: Some(err),
            build_state: None,
            graph_has_content: false,
            ..healthy_input()
        });
        assert_eq!(r.integrity_state, IntegrityState::SchemaMismatch);
        assert!(!r.graph_queryable, "blocked on integrity");
        assert!(!r.graph_built);
        assert_eq!(r.error_code, "schema_mismatch");
        assert_eq!(r.db_open_error.as_deref(), Some(err));
    }

    #[test]
    fn corrupt_db_error_yields_corrupt_integrity() {
        let err = "sqlite error: database disk image is malformed";
        let r = GraphReadiness::derive(GraphReadinessInput {
            db_open_error: Some(err),
            build_state: None,
            graph_has_content: false,
            ..healthy_input()
        });
        assert_eq!(r.integrity_state, IntegrityState::Corrupt);
        assert!(!r.graph_queryable);
        assert_eq!(r.error_code, "corrupt_or_inconsistent_graph_rows");
    }

    #[test]
    fn graph_query_error_makes_graph_not_queryable() {
        let err = "no such column: nodes.extra_json";
        let r = GraphReadiness::derive(GraphReadinessInput {
            graph_error: Some(err),
            ..healthy_input()
        });
        assert!(r.graph_built, "build record says built");
        assert!(!r.graph_queryable, "blocked by query error");
        assert!(!r.graph_current);
        assert_eq!(r.integrity_state, IntegrityState::SchemaMismatch);
        assert_eq!(r.error_code, "schema_mismatch");
    }

    #[test]
    fn generic_graph_query_error_yields_corrupt_integrity() {
        let err = "constraint failed: UNIQUE constraint on edges";
        let r = GraphReadiness::derive(GraphReadinessInput {
            graph_error: Some(err),
            ..healthy_input()
        });
        assert!(!r.graph_queryable);
        assert_eq!(r.integrity_state, IntegrityState::Corrupt);
        assert_eq!(r.error_code, "corrupt_or_inconsistent_graph_rows");
    }

    // ── retrieval unavailable ────────────────────────────────────────────────

    #[test]
    fn retrieval_unavailable_surfaces_in_error_code() {
        let r = GraphReadiness::derive(GraphReadinessInput {
            retrieval_unavailable: true,
            ..healthy_input()
        });
        assert!(r.graph_built);
        assert!(r.graph_queryable, "graph itself is queryable");
        assert!(r.graph_current);
        assert_eq!(r.error_code, "retrieval_index_unavailable");
        assert!(!r.is_ok());
    }

    // ── field derivation: error code takes priority ──────────────────────────

    #[test]
    fn stale_index_takes_priority_over_retrieval_unavailable() {
        let pending = vec!["src/foo.rs".to_owned()];
        let r = GraphReadiness::derive(GraphReadinessInput {
            pending_graph_changes: &pending,
            retrieval_unavailable: true,
            ..healthy_input()
        });
        // stale_index is checked before retrieval_unavailable in health.rs
        assert_eq!(r.error_code, "stale_index");
        assert!(r.stale_index);
    }

    #[test]
    fn missing_db_takes_priority_over_everything() {
        let pending = vec!["src/foo.rs".to_owned()];
        let r = GraphReadiness::derive(GraphReadinessInput {
            db_exists: false,
            build_state: None,
            graph_has_content: false,
            pending_graph_changes: &pending,
            retrieval_unavailable: true,
            ..healthy_input()
        });
        assert_eq!(r.error_code, "missing_graph_db");
    }

    // ── integrity_state derive_integrity_state unit tests ───────────────────

    #[test]
    fn no_error_yields_clean_integrity() {
        assert_eq!(derive_integrity_state(None, None), IntegrityState::Clean);
    }

    #[test]
    fn schema_mismatch_detection_from_db_open_error() {
        assert_eq!(
            derive_integrity_state(
                Some("table edges has no column named reachability_checked"),
                None
            ),
            IntegrityState::SchemaMismatch,
        );
    }

    #[test]
    fn schema_mismatch_detection_from_graph_error() {
        assert_eq!(
            derive_integrity_state(None, Some("no such column: nodes.extra_json")),
            IntegrityState::SchemaMismatch,
        );
    }

    #[test]
    fn db_open_error_takes_precedence_over_graph_error() {
        // db_open_error = schema mismatch, graph_error = corrupt  -> schema_mismatch wins
        let open_err = Some("table nodes has no column named foo");
        let graph_err = Some("constraint failed");
        assert_eq!(
            derive_integrity_state(open_err, graph_err),
            IntegrityState::SchemaMismatch,
        );
    }

    // ── IntegrityState helpers ───────────────────────────────────────────────

    #[test]
    fn integrity_state_as_str_round_trips() {
        assert_eq!(IntegrityState::Clean.as_str(), "clean");
        assert_eq!(
            IntegrityState::NoncanonicalPaths.as_str(),
            "noncanonical_paths"
        );
        assert_eq!(IntegrityState::SchemaMismatch.as_str(), "schema_mismatch");
        assert_eq!(IntegrityState::Corrupt.as_str(), "corrupt");
    }

    #[test]
    fn integrity_state_is_clean_helper() {
        assert!(IntegrityState::Clean.is_clean());
        assert!(!IntegrityState::Corrupt.is_clean());
        assert!(!IntegrityState::SchemaMismatch.is_clean());
        assert!(!IntegrityState::NoncanonicalPaths.is_clean());
    }

    // ── suggestions and message presence ────────────────────────────────────

    #[test]
    fn failed_build_includes_suggestions() {
        let r = GraphReadiness::derive(GraphReadinessInput {
            build_state: Some("build_failed"),
            graph_has_content: false,
            ..healthy_input()
        });
        assert!(
            !r.suggestions.is_empty(),
            "should have suggestions for failed build"
        );
        assert!(!r.message.is_empty());
    }

    #[test]
    fn fresh_graph_has_empty_suggestions() {
        let r = GraphReadiness::derive(healthy_input());
        assert!(r.suggestions.is_empty());
    }

    // ── serde round-trip ─────────────────────────────────────────────────────

    #[test]
    fn graph_readiness_serializes_and_deserializes() {
        let r = GraphReadiness::derive(healthy_input());
        let json = serde_json::to_string(&r).expect("serialize");
        let r2: GraphReadiness = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(r.error_code, r2.error_code);
        assert_eq!(r.graph_built, r2.graph_built);
        assert_eq!(r.graph_queryable, r2.graph_queryable);
        assert_eq!(r.graph_current, r2.graph_current);
        assert_eq!(r.integrity_state, r2.integrity_state);
    }

    #[test]
    fn integrity_state_serde_round_trips() {
        for state in [
            IntegrityState::Clean,
            IntegrityState::NoncanonicalPaths,
            IntegrityState::SchemaMismatch,
            IntegrityState::Corrupt,
        ] {
            let json = serde_json::to_string(&state).expect("serialize");
            let back: IntegrityState = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(state, back);
        }
    }

    // ── GraphExecutionState derivation ───────────────────────────────────────

    #[test]
    fn fresh_graph_execution_state_is_fresh() {
        let r = GraphReadiness::derive(healthy_input());
        assert_eq!(r.execution_state, GraphExecutionState::Fresh);
    }

    #[test]
    fn stale_graph_execution_state_is_stale() {
        let pending = vec!["src/main.rs".to_owned()];
        let r = GraphReadiness::derive(GraphReadinessInput {
            pending_graph_changes: &pending,
            ..healthy_input()
        });
        assert_eq!(r.execution_state, GraphExecutionState::Stale);
    }

    #[test]
    fn degraded_build_execution_state_is_partial() {
        let r = GraphReadiness::derive(GraphReadinessInput {
            build_state: Some("degraded"),
            graph_has_content: true,
            ..healthy_input()
        });
        assert_eq!(r.execution_state, GraphExecutionState::Partial);
    }

    #[test]
    fn corrupt_db_execution_state_is_corrupt() {
        let r = GraphReadiness::derive(GraphReadinessInput {
            db_open_error: Some("database disk image is malformed"),
            ..healthy_input()
        });
        assert_eq!(r.execution_state, GraphExecutionState::Corrupt);
    }

    #[test]
    fn schema_mismatch_execution_state_is_corrupt() {
        let r = GraphReadiness::derive(GraphReadinessInput {
            db_open_error: Some("noncanonical_path rows detected"),
            ..healthy_input()
        });
        assert_eq!(r.execution_state, GraphExecutionState::Corrupt);
    }

    #[test]
    fn missing_graph_execution_state_is_missing() {
        let r = GraphReadiness::derive(GraphReadinessInput {
            db_exists: false,
            build_state: None,
            graph_has_content: false,
            ..healthy_input()
        });
        assert_eq!(r.execution_state, GraphExecutionState::Missing);
    }

    #[test]
    fn unbuilt_graph_without_db_missing_execution_state() {
        let r = GraphReadiness::derive(GraphReadinessInput {
            build_state: Some("building"),
            graph_has_content: false,
            ..healthy_input()
        });
        assert_eq!(r.execution_state, GraphExecutionState::Missing);
    }

    #[test]
    fn corrupt_trumps_missing_in_execution_state() {
        // A DB that exists but fails to open should yield Corrupt, not Missing.
        let r = GraphReadiness::derive(GraphReadinessInput {
            db_exists: true,
            db_open_error: Some("database disk image is malformed"),
            build_state: None,
            graph_has_content: false,
            ..healthy_input()
        });
        assert_eq!(r.execution_state, GraphExecutionState::Corrupt);
    }

    #[test]
    fn corrupt_trumps_stale_in_execution_state() {
        let pending = vec!["src/main.rs".to_owned()];
        let r = GraphReadiness::derive(GraphReadinessInput {
            pending_graph_changes: &pending,
            graph_error: Some("database disk image is malformed"),
            ..healthy_input()
        });
        assert_eq!(r.execution_state, GraphExecutionState::Corrupt);
    }

    #[test]
    fn partial_trumps_stale_in_execution_state() {
        let pending = vec!["src/main.rs".to_owned()];
        let r = GraphReadiness::derive(GraphReadinessInput {
            build_state: Some("degraded"),
            pending_graph_changes: &pending,
            graph_has_content: true,
            ..healthy_input()
        });
        assert_eq!(r.execution_state, GraphExecutionState::Partial);
    }

    #[test]
    fn execution_state_as_str_values() {
        assert_eq!(GraphExecutionState::Fresh.as_str(), "fresh");
        assert_eq!(GraphExecutionState::Stale.as_str(), "stale");
        assert_eq!(GraphExecutionState::Partial.as_str(), "partial");
        assert_eq!(GraphExecutionState::Corrupt.as_str(), "corrupt");
        assert_eq!(GraphExecutionState::Missing.as_str(), "missing");
    }

    #[test]
    fn is_queryable_helper() {
        assert!(GraphExecutionState::Fresh.is_queryable());
        assert!(GraphExecutionState::Stale.is_queryable());
        assert!(GraphExecutionState::Partial.is_queryable());
        assert!(!GraphExecutionState::Corrupt.is_queryable());
        assert!(!GraphExecutionState::Missing.is_queryable());
    }

    #[test]
    fn execution_state_serde_round_trips() {
        for state in [
            GraphExecutionState::Fresh,
            GraphExecutionState::Stale,
            GraphExecutionState::Partial,
            GraphExecutionState::Corrupt,
            GraphExecutionState::Missing,
        ] {
            let json = serde_json::to_string(&state).expect("serialize");
            let back: GraphExecutionState = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(state, back);
        }
    }

    // ── check_tool — diagnostics always allowed ──────────────────────────────

    #[test]
    fn diagnostics_allowed_in_fresh_state() {
        let r = GraphReadiness::derive(healthy_input());
        let v = r.check_tool(
            GraphToolRequirement::Diagnostics,
            ReadinessOverride::default(),
        );
        assert!(v.is_allowed());
        assert_eq!(v.execution_state(), GraphExecutionState::Fresh);
    }

    #[test]
    fn diagnostics_allowed_in_corrupt_state() {
        let r = GraphReadiness::derive(GraphReadinessInput {
            db_open_error: Some("database disk image is malformed"),
            ..healthy_input()
        });
        let v = r.check_tool(
            GraphToolRequirement::Diagnostics,
            ReadinessOverride::default(),
        );
        assert!(v.is_allowed(), "diagnostics must always be allowed");
    }

    #[test]
    fn diagnostics_allowed_in_missing_state() {
        let r = GraphReadiness::derive(GraphReadinessInput {
            db_exists: false,
            build_state: None,
            graph_has_content: false,
            ..healthy_input()
        });
        let v = r.check_tool(
            GraphToolRequirement::Diagnostics,
            ReadinessOverride::default(),
        );
        assert!(v.is_allowed(), "diagnostics must always be allowed");
    }

    // ── check_tool — corrupt / missing block all non-diagnostic tools ────────

    #[test]
    fn symbol_lookup_blocked_when_corrupt() {
        let r = GraphReadiness::derive(GraphReadinessInput {
            db_open_error: Some("database disk image is malformed"),
            ..healthy_input()
        });
        let v = r.check_tool(
            GraphToolRequirement::SymbolLookup,
            ReadinessOverride {
                allow_partial: true,
                allow_stale: true,
            },
        );
        assert!(v.is_blocked(), "corrupt blocks even with override flags");
    }

    #[test]
    fn traversal_blocked_when_missing() {
        let r = GraphReadiness::derive(GraphReadinessInput {
            db_exists: false,
            build_state: None,
            graph_has_content: false,
            ..healthy_input()
        });
        let v = r.check_tool(
            GraphToolRequirement::Traversal,
            ReadinessOverride {
                allow_stale: true,
                allow_partial: true,
            },
        );
        assert!(v.is_blocked(), "missing blocks even with override flags");
    }

    #[test]
    fn analysis_blocked_when_corrupt() {
        let r = GraphReadiness::derive(GraphReadinessInput {
            graph_error: Some("malformed database"),
            ..healthy_input()
        });
        let v = r.check_tool(GraphToolRequirement::Analysis, ReadinessOverride::default());
        assert!(v.is_blocked());
        assert_eq!(v.execution_state(), GraphExecutionState::Corrupt);
    }

    // ── check_tool — partial state ───────────────────────────────────────────

    #[test]
    fn symbol_lookup_blocked_in_partial_without_override() {
        let r = GraphReadiness::derive(GraphReadinessInput {
            build_state: Some("degraded"),
            graph_has_content: true,
            ..healthy_input()
        });
        let v = r.check_tool(
            GraphToolRequirement::SymbolLookup,
            ReadinessOverride::default(),
        );
        assert!(
            v.is_blocked(),
            "symbol lookup blocked in partial without override"
        );
    }

    #[test]
    fn symbol_lookup_allowed_in_partial_with_allow_partial() {
        let r = GraphReadiness::derive(GraphReadinessInput {
            build_state: Some("degraded"),
            graph_has_content: true,
            ..healthy_input()
        });
        let v = r.check_tool(
            GraphToolRequirement::SymbolLookup,
            ReadinessOverride {
                allow_partial: true,
                ..ReadinessOverride::default()
            },
        );
        assert!(v.is_allowed());
        if let ReadinessVerdict::Allowed {
            safe_to_answer,
            warning,
            ..
        } = v
        {
            assert!(!safe_to_answer, "partial is not safe_to_answer");
            assert!(warning.is_some(), "warning should be present");
        }
    }

    #[test]
    fn traversal_blocked_in_partial_even_with_allow_partial() {
        let r = GraphReadiness::derive(GraphReadinessInput {
            build_state: Some("degraded"),
            graph_has_content: true,
            ..healthy_input()
        });
        let v = r.check_tool(
            GraphToolRequirement::Traversal,
            ReadinessOverride {
                allow_partial: true,
                ..ReadinessOverride::default()
            },
        );
        assert!(v.is_blocked(), "traversal always blocked when partial");
    }

    #[test]
    fn analysis_blocked_in_partial_even_with_allow_partial() {
        let r = GraphReadiness::derive(GraphReadinessInput {
            build_state: Some("degraded"),
            graph_has_content: true,
            ..healthy_input()
        });
        let v = r.check_tool(
            GraphToolRequirement::Analysis,
            ReadinessOverride {
                allow_partial: true,
                ..ReadinessOverride::default()
            },
        );
        assert!(v.is_blocked(), "analysis always blocked when partial");
    }

    // ── check_tool — stale state ─────────────────────────────────────────────

    #[test]
    fn all_tools_allowed_when_stale_with_warning() {
        let pending = vec!["src/main.rs".to_owned()];
        let r = GraphReadiness::derive(GraphReadinessInput {
            pending_graph_changes: &pending,
            ..healthy_input()
        });
        for req in [
            GraphToolRequirement::SymbolLookup,
            GraphToolRequirement::Traversal,
            GraphToolRequirement::Analysis,
        ] {
            let v = r.check_tool(req, ReadinessOverride::default());
            assert!(v.is_allowed(), "{req:?} should be allowed when stale");
            if let ReadinessVerdict::Allowed {
                safe_to_answer,
                warning,
                ..
            } = v
            {
                assert!(!safe_to_answer, "stale is not safe_to_answer");
                assert!(warning.is_some(), "warning expected when stale");
            }
        }
    }

    // ── check_tool — fresh state ─────────────────────────────────────────────

    #[test]
    fn all_tools_allowed_when_fresh_and_safe() {
        let r = GraphReadiness::derive(healthy_input());
        for req in [
            GraphToolRequirement::SymbolLookup,
            GraphToolRequirement::Traversal,
            GraphToolRequirement::Analysis,
        ] {
            let v = r.check_tool(req, ReadinessOverride::default());
            assert!(v.is_allowed(), "{req:?} should be allowed when fresh");
            if let ReadinessVerdict::Allowed {
                safe_to_answer,
                warning,
                ..
            } = v
            {
                assert!(safe_to_answer, "fresh should be safe_to_answer");
                assert!(warning.is_none(), "no warning expected when fresh");
            }
        }
    }

    // ── ReadinessVerdict helpers ─────────────────────────────────────────────

    #[test]
    fn verdict_is_allowed_and_is_blocked_are_exclusive() {
        let r = GraphReadiness::derive(healthy_input());
        let allowed = r.check_tool(
            GraphToolRequirement::SymbolLookup,
            ReadinessOverride::default(),
        );
        let r2 = GraphReadiness::derive(GraphReadinessInput {
            db_exists: false,
            build_state: None,
            graph_has_content: false,
            ..healthy_input()
        });
        let blocked = r2.check_tool(
            GraphToolRequirement::SymbolLookup,
            ReadinessOverride::default(),
        );
        assert!(allowed.is_allowed() && !allowed.is_blocked());
        assert!(blocked.is_blocked() && !blocked.is_allowed());
    }

    #[test]
    fn verdict_execution_state_accessor() {
        let r = GraphReadiness::derive(healthy_input());
        let v = r.check_tool(
            GraphToolRequirement::Diagnostics,
            ReadinessOverride::default(),
        );
        assert_eq!(v.execution_state(), GraphExecutionState::Fresh);
    }

    #[test]
    fn readiness_override_default_is_strict() {
        let d = ReadinessOverride::default();
        assert!(!d.allow_stale);
        assert!(!d.allow_partial);
    }
}
