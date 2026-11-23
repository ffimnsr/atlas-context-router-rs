use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub enum InsightsLargeFunctionMode {
    Large,
    Complex,
    #[value(name = "large-or-complex")]
    LargeOrComplex,
}

impl From<InsightsLargeFunctionMode> for atlas_reasoning::LargeFunctionMode {
    fn from(value: InsightsLargeFunctionMode) -> Self {
        match value {
            InsightsLargeFunctionMode::Large => Self::Large,
            InsightsLargeFunctionMode::Complex => Self::Complex,
            InsightsLargeFunctionMode::LargeOrComplex => Self::LargeOrComplex,
        }
    }
}

/// Sub-commands for `atlas insights`.
#[derive(Debug, Subcommand)]
pub enum InsightsCommand {
    /// Analyze module-level cycles, layer violations, and coupling hotspots.
    Architecture {
        /// Cap returned findings after ranking.
        #[arg(long)]
        limit: Option<usize>,
    },

    /// Analyze graph health metrics, outliers, and complexity hotspots.
    Metrics {
        /// Cap returned findings after ranking.
        #[arg(long)]
        limit: Option<usize>,
    },

    /// Score deterministic change risk for one symbol.
    Risk {
        /// Fully-qualified symbol name or resolvable symbol identifier.
        symbol: String,
    },

    /// Detect repeated chains, isolated structures, hubs, and deep dependency paths.
    Patterns {
        /// Cap returned findings after ranking.
        #[arg(long)]
        limit: Option<usize>,
    },

    /// Find large or complex functions across the repo or within selected files.
    #[command(name = "large-functions")]
    LargeFunctions {
        /// Restrict search to one or more repo-relative files.
        #[arg(long, num_args = 1..)]
        files: Vec<String>,

        /// Override the configured LOC threshold.
        #[arg(long)]
        threshold: Option<usize>,

        /// Override the configured cyclomatic complexity threshold.
        #[arg(long = "complexity-threshold")]
        complexity_threshold: Option<usize>,

        /// Override the configured cognitive complexity threshold.
        #[arg(long = "cognitive-threshold")]
        cognitive_threshold: Option<usize>,

        /// Override the configured max nesting depth threshold.
        #[arg(long = "nesting-threshold")]
        nesting_threshold: Option<usize>,

        /// Filter to large-only, complex-only, or either category.
        #[arg(long, value_enum, default_value_t = InsightsLargeFunctionMode::LargeOrComplex)]
        mode: InsightsLargeFunctionMode,

        /// Cap returned findings after ranking.
        #[arg(long)]
        limit: Option<usize>,

        /// Include test functions and test methods in the result set.
        #[arg(long)]
        include_tests: bool,
    },

    /// Find complex functions across the repo or within selected files.
    #[command(name = "complex-functions")]
    ComplexFunctions {
        /// Restrict search to one or more repo-relative files.
        #[arg(long, num_args = 1..)]
        files: Vec<String>,

        /// Override the configured cyclomatic complexity threshold.
        #[arg(long = "complexity-threshold")]
        complexity_threshold: Option<usize>,

        /// Override the configured cognitive complexity threshold.
        #[arg(long = "cognitive-threshold")]
        cognitive_threshold: Option<usize>,

        /// Override the configured max nesting depth threshold.
        #[arg(long = "nesting-threshold")]
        nesting_threshold: Option<usize>,

        /// Cap returned findings after ranking.
        #[arg(long)]
        limit: Option<usize>,

        /// Include test functions and test methods in the result set.
        #[arg(long)]
        include_tests: bool,
    },

    /// Find semantically similar functions for one symbol.
    #[command(name = "similar-functions")]
    SimilarFunctions {
        /// Fully-qualified symbol name or resolvable callable identifier.
        symbol: String,

        /// Minimum similarity score to keep (0.0-1.0).
        #[arg(long)]
        min_score: Option<f64>,

        /// Cap returned matches after ranking.
        #[arg(long)]
        limit: Option<usize>,

        /// Keep same-file matches too.
        #[arg(long)]
        include_same_file: bool,
    },

    /// Find exact-normalized and near-duplicate callable bodies.
    Duplicates {
        /// Restrict search to one or more repo-relative files.
        #[arg(long, num_args = 1..)]
        files: Vec<String>,

        /// Minimum duplicate confidence to keep (0.0-1.0).
        #[arg(long)]
        min_score: Option<f64>,

        /// Cap returned duplicate groups after ranking.
        #[arg(long)]
        limit: Option<usize>,

        /// Include test functions and test methods in the result set.
        #[arg(long)]
        include_tests: bool,

        /// Suppress duplicate groups matching group id, normalized summary, file path, or symbol name.
        #[arg(long = "suppress", num_args = 1..)]
        suppressions: Vec<String>,
    },

    /// Infer module buckets from package ownership, paths, and dependency structure.
    #[command(name = "infer-modules")]
    InferModules {
        /// Cap returned findings after ranking.
        #[arg(long)]
        limit: Option<usize>,
    },

    /// Label files and symbols with Atlas component taxonomy.
    #[command(name = "label-components")]
    LabelComponents {
        /// Restrict file labeling to one or more repo-relative files.
        #[arg(long, num_args = 1..)]
        files: Vec<String>,

        /// Restrict symbol labeling to one or more qualified names.
        #[arg(long = "symbol", num_args = 1..)]
        symbols: Vec<String>,

        /// Cap returned assignments after ranking.
        #[arg(long)]
        limit: Option<usize>,
    },
}
