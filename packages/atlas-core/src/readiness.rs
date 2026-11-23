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
    GraphHealthInput, GraphStoreHealthClass, classify_graph_store_error,
    graph_health_error_message, graph_health_error_suggestions, select_graph_health_error_code,
};

/// Integrity classification of the graph database.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrityState {
    /// No integrity issues detected.
    #[default]
    Clean,
    /// SQLite schema does not match this Atlas build; rebuild required.
    SchemaMismatch,
    /// SQLite storage is physically corrupt or unreadable.
    SqliteCorrupt,
    /// SQLite opened, but graph invariants failed.
    LogicalInconsistency,
}

impl IntegrityState {
    /// Machine-readable string label for this state.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Clean => "clean",
            Self::SchemaMismatch => "schema_mismatch",
            Self::SqliteCorrupt => "sqlite_corrupt",
            Self::LogicalInconsistency => "logical_inconsistency",
        }
    }

    /// Returns `true` when no integrity issues are present.
    pub fn is_clean(&self) -> bool {
        matches!(self, Self::Clean)
    }

    /// Shared graph-store health class implied by this integrity state.
    pub fn health_class(&self) -> Option<GraphStoreHealthClass> {
        match self {
            Self::Clean => None,
            Self::SchemaMismatch => Some(GraphStoreHealthClass::SchemaMismatch),
            Self::SqliteCorrupt => Some(GraphStoreHealthClass::SqliteCorrupt),
            Self::LogicalInconsistency => Some(GraphStoreHealthClass::LogicalInconsistency),
        }
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
    /// Canonical graph-store health class when the DB exists and the state is
    /// classifiable, or `None` when the store is missing or only retrieval is
    /// degraded.
    pub health_class: Option<GraphStoreHealthClass>,
    /// Short machine-readable error code for the most significant readiness
    /// issue, or `"none"` when the graph is ready.
    pub error_code: String,
    /// Recovery mode last recorded in build lifecycle metadata, when present.
    pub recovery_mode: Option<String>,
    /// Quarantine path recorded for the graph store, when present.
    pub quarantine_path: Option<String>,
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

    /// Canonical rebuild command to surface in fail-closed responses.
    pub fn recommended_rebuild_command(&self) -> &'static str {
        "atlas build"
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
    /// Recovery mode last recorded in build lifecycle metadata, when present.
    pub recovery_mode: Option<&'a str>,
    /// Quarantine path recorded in build lifecycle metadata, when present.
    pub quarantine_path: Option<&'a str>,
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

        let health_class = derive_health_class(
            input.db_exists,
            &integrity_state,
            graph_built,
            input.build_state,
            stale_index,
        );

        let error_code = select_graph_health_error_code(GraphHealthInput {
            db_exists: input.db_exists,
            graph_built,
            health_class,
            build_state: input.build_state,
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
            health_class,
            error_code: error_code.to_owned(),
            recovery_mode: input.recovery_mode.map(str::to_owned),
            quarantine_path: input.quarantine_path.map(str::to_owned),
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

    match classify_graph_store_error(err) {
        GraphStoreHealthClass::SchemaMismatch => IntegrityState::SchemaMismatch,
        GraphStoreHealthClass::SqliteCorrupt => IntegrityState::SqliteCorrupt,
        GraphStoreHealthClass::LogicalInconsistency => IntegrityState::LogicalInconsistency,
        GraphStoreHealthClass::Healthy
        | GraphStoreHealthClass::Stale
        | GraphStoreHealthClass::InterruptedBuild
        | GraphStoreHealthClass::FailedBuild => IntegrityState::SqliteCorrupt,
    }
}

fn derive_health_class(
    db_exists: bool,
    integrity_state: &IntegrityState,
    graph_built: bool,
    build_state: Option<&str>,
    stale_index: bool,
) -> Option<GraphStoreHealthClass> {
    if !db_exists {
        return None;
    }
    if let Some(health_class) = integrity_state.health_class() {
        return Some(health_class);
    }
    match build_state {
        Some("building") => return Some(GraphStoreHealthClass::InterruptedBuild),
        Some("build_failed") => return Some(GraphStoreHealthClass::FailedBuild),
        _ => {}
    }
    if stale_index {
        return Some(GraphStoreHealthClass::Stale);
    }
    if graph_built {
        return Some(GraphStoreHealthClass::Healthy);
    }
    None
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
mod tests;
