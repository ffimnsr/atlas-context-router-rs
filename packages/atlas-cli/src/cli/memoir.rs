//! ICM-F2 — `atlas memoir` CLI surface.
//!
//! Commands are thin parsers over the shared memoir service layer in
//! `atlas-session`; all vocabulary, direction, duplicate, and opt-in rules
//! live there so CLI and MCP cannot drift.

use clap::Subcommand;

/// Sub-commands for `atlas memoir`.
#[derive(Debug, Subcommand)]
pub enum MemoirCommand {
    /// Create a memoir concept graph for the current repo.
    Create {
        /// Graph name; unique per repo and scope (case-insensitive).
        name: String,

        /// Optional graph description.
        #[arg(long)]
        description: Option<String>,

        /// Visibility scope: project, session, frontend, or global (default: project).
        #[arg(long)]
        scope: Option<String>,
    },

    /// Add a concept to an existing memoir graph.
    AddConcept {
        /// Graph name.
        graph: String,

        /// Concept name; unique inside the graph (case-insensitive).
        name: String,

        /// Concept description or summary (optional).
        description: Option<String>,

        /// Concept kind label (default: concept).
        #[arg(long)]
        kind: Option<String>,

        /// Source id linking this concept to saved-context evidence.
        #[arg(long)]
        source_id: Option<String>,

        /// Graph scope: project, session, frontend, or global (default: project).
        #[arg(long)]
        scope: Option<String>,
    },

    /// Link two memoir concepts with a controlled relation type.
    Link {
        /// Graph name.
        graph: String,

        /// Source concept name.
        source: String,

        /// Target concept name.
        target: String,

        /// Relation type flag form.
        #[arg(long = "relation")]
        relation: Option<String>,

        /// Relation type positional form (alias for `--relation`).
        #[arg(value_name = "RELATION")]
        relation_positional: Option<String>,

        /// Relation confidence between 0.0 and 1.0 (default: 1.0).
        #[arg(long, default_value_t = 1.0)]
        confidence: f64,

        /// Source id linking this relation to saved-context evidence.
        #[arg(long)]
        source_id: Option<String>,

        /// Graph scope: project, session, frontend, or global (default: project).
        #[arg(long)]
        scope: Option<String>,

        /// Explicitly opt in to creating missing concept endpoints.
        #[arg(long)]
        allow_missing_concepts: bool,
    },

    /// Inspect a concept's bounded neighbourhood with relation direction.
    ///
    /// Accepts either `inspect <CONCEPT> --graph <GRAPH>` or the guide form
    /// `inspect <GRAPH> <CONCEPT>`.
    Inspect {
        /// Concept name, or graph name when two positionals are supplied.
        #[arg(value_name = "GRAPH|CONCEPT", num_args = 1..=2)]
        targets: Vec<String>,

        /// Graph name; omit to resolve across graphs (ambiguous names fail).
        #[arg(long)]
        graph: Option<String>,

        /// Traversal depth (clamped to 1..=5, default: 1).
        #[arg(long, default_value_t = 1)]
        depth: u32,

        /// Restrict traversal to one relation type.
        #[arg(long)]
        relation: Option<String>,

        /// Graph scope used when --graph is provided (default: project).
        #[arg(long)]
        scope: Option<String>,
    },

    /// Delete a memoir graph and all of its concepts and relations.
    Delete {
        /// Graph name.
        graph: String,

        /// Report counts without deleting.
        #[arg(long)]
        dry_run: bool,

        /// Graph scope: project, session, frontend, or global (default: project).
        #[arg(long)]
        scope: Option<String>,
    },
}
