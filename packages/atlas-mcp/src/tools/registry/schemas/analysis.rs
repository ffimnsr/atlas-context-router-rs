use super::*;

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct EmbedChunksArgsSchema {
    /// Maximum number of chunks to embed in this run (default 1000).
    limit: Option<u64>,
    /// Count pending chunks without calling the provider or writing vectors.
    dry_run: Option<bool>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BuildGraphArgsSchema {
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateGraphArgsSchema {
    change_source: ChangeSourceWithFilesArgsSchema,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PostprocessStageSchema {
    Flows,
    Communities,
    ArchitectureMetrics,
    QueryHints,
    LargeFunctionSummaries,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PostprocessGraphArgsSchema {
    changed_only: Option<bool>,
    stage: Option<PostprocessStageSchema>,
    dry_run: Option<bool>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DetectChangesArgsSchema {
    change_source: ChangeSourceWithoutFilesArgsSchema,
    repo_scope: Option<RepoScopeArgsSchema>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GetImpactRadiusArgsSchema {
    change_source: ChangeSourceWithFilesArgsSchema,
    max_depth: Option<u64>,
    max_nodes: Option<u64>,
    repo_scope: Option<RepoScopeArgsSchema>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GetReviewContextArgsSchema {
    change_source: ChangeSourceWithFilesArgsSchema,
    max_depth: Option<u64>,
    max_nodes: Option<u64>,
    token_budget: Option<u64>,
    exact_dir: Option<String>,
    exclude_subpaths: Option<Vec<String>>,
    include_telemetry: Option<bool>,
    include_ranking_evidence: Option<bool>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GetMinimalContextArgsSchema {
    change_source: ChangeSourceWithoutFilesArgsSchema,
    max_depth: Option<u64>,
    max_nodes: Option<u64>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExplainChangeArgsSchema {
    change_source: ChangeSourceWithFilesArgsSchema,
    max_depth: Option<u64>,
    max_nodes: Option<u64>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GetContextTargetKindSchema {
    Query,
    File,
    Files,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GetContextTargetArgsSchema {
    /// Target kind: query, file, or files.
    kind: GetContextTargetKindSchema,
    /// Query grammar: identifier, qualified name, 'who calls <symbol>', 'what breaks <symbol>', or 'tests for <symbol>'.
    query: Option<String>,
    /// Single repo-relative file path when kind='file'.
    file: Option<String>,
    /// Repo-relative file paths when kind='files'.
    files: Option<Vec<String>>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GetContextIntentSchema {
    Symbol,
    File,
    Review,
    Impact,
    UsageLookup,
    RefactorSafety,
    DeadCodeCheck,
    RenamePreview,
    DependencyRemoval,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GetContextArgsSchema {
    target: GetContextTargetArgsSchema,
    intent: Option<GetContextIntentSchema>,
    max_nodes: Option<u64>,
    max_edges: Option<u64>,
    max_files: Option<u64>,
    max_depth: Option<u64>,
    code_spans: Option<bool>,
    tests: Option<bool>,
    imports: Option<bool>,
    neighbors: Option<bool>,
    semantic: Option<bool>,
    include_saved_context: Option<bool>,
    include_content_assets: Option<bool>,
    session_id: Option<String>,
    agent_id: Option<String>,
    merge_agent_partitions: Option<bool>,
    token_budget: Option<u64>,
    exact_dir: Option<String>,
    exclude_subpaths: Option<Vec<String>>,
    include_telemetry: Option<bool>,
    include_ranking_evidence: Option<bool>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum LargeFunctionModeSchema {
    Large,
    Complex,
    LargeOrComplex,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FindLargeFunctionsArgsSchema {
    files: Option<Vec<String>>,
    threshold: Option<u64>,
    complexity_threshold: Option<u64>,
    cognitive_threshold: Option<u64>,
    nesting_threshold: Option<u64>,
    mode: Option<LargeFunctionModeSchema>,
    limit: Option<u64>,
    include_tests: Option<bool>,
    verbose: Option<bool>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FindComplexFunctionsArgsSchema {
    files: Option<Vec<String>>,
    complexity_threshold: Option<u64>,
    cognitive_threshold: Option<u64>,
    nesting_threshold: Option<u64>,
    limit: Option<u64>,
    include_tests: Option<bool>,
    verbose: Option<bool>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FindSimilarFunctionsArgsSchema {
    symbol: String,
    min_score: Option<f64>,
    limit: Option<u64>,
    include_same_file: Option<bool>,
    verbose: Option<bool>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FindDuplicatesArgsSchema {
    files: Option<Vec<String>>,
    min_score: Option<f64>,
    limit: Option<u64>,
    include_tests: Option<bool>,
    suppressions: Option<Vec<String>>,
    verbose: Option<bool>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct InferModulesArgsSchema {
    limit: Option<u64>,
    verbose: Option<bool>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct LabelComponentsArgsSchema {
    files: Option<Vec<String>>,
    symbols: Option<Vec<String>>,
    limit: Option<u64>,
    verbose: Option<bool>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AnalyzeSafetyArgsSchema {
    symbol: String,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AnalyzeRemoveArgsSchema {
    symbols: Vec<String>,
    max_depth: Option<u64>,
    max_nodes: Option<u64>,
    max_files: Option<u64>,
    max_edges: Option<u64>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DeadCodeExcludeKindSchema {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    Interface,
    Class,
    Constant,
    Variable,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AnalyzeDeadCodeArgsSchema {
    allowlist: Option<Vec<String>>,
    subpath: Option<String>,
    limit: Option<u64>,
    summary: Option<bool>,
    exclude_kind: Option<Vec<DeadCodeExcludeKindSchema>>,
    code_only: Option<bool>,
    max_files: Option<u64>,
    max_edges: Option<u64>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct AnalyzeDependencyArgsSchema {
    symbol: String,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SingleRepoScopeKindSchema {
    Current,
    RepoId,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SingleRepoScopeArgsSchema {
    /// Repo scope kind: current or repo_id. Multi-repo scopes are rejected.
    kind: SingleRepoScopeKindSchema,
    /// Required when kind='repo_id'. Must be registered and enabled.
    repo_id: Option<String>,
}
