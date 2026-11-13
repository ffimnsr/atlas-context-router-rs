//! Typed argument-schema structs (schemars) used by the typed-input
//! builders. One file per registry split; structs are shared across
//! tool families, so they are re-exported through `super::schemas::*`.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct HealthOutputFormatArgs {
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DbCheckArgsSchema {
    limit: Option<u64>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct DebugGraphArgsSchema {
    limit: Option<u64>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchFilesArgsSchema {
    pattern: String,
    globs: Option<Vec<String>>,
    exclude_globs: Option<Vec<String>>,
    subpath: Option<String>,
    case_sensitive: Option<bool>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchContentArgsSchema {
    query: String,
    globs: Option<Vec<String>>,
    exclude_globs: Option<Vec<String>>,
    exclude_generated: Option<bool>,
    is_regex: Option<bool>,
    context_lines: Option<u64>,
    rich_snippets: Option<bool>,
    snippet_context_lines: Option<u64>,
    max_results: Option<u64>,
    subpath: Option<String>,
    exact_dir: Option<String>,
    exclude_subpaths: Option<Vec<String>>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExcerptSelectorKindSchema {
    Range,
    Ranges,
    Context,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExcerptLineRangeSchema {
    /// 1-based inclusive start line.
    start_line: u64,
    /// 1-based inclusive end line.
    end_line: u64,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadFileExcerptSelectorSchema {
    /// Selector kind: range, ranges, or context.
    kind: ExcerptSelectorKindSchema,
    /// 1-based start line for kind='range'.
    start_line: Option<u64>,
    /// Inclusive end line for kind='range'.
    end_line: Option<u64>,
    /// 1-based anchor line for kind='context'.
    line: Option<u64>,
    /// Context lines before the anchor line (default 2).
    before: Option<u64>,
    /// Context lines after the anchor line (default 2).
    after: Option<u64>,
    /// Explicit line ranges for kind='ranges'.
    line_ranges: Option<Vec<ExcerptLineRangeSchema>>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadFileExcerptArgsSchema {
    file: String,
    selector: ReadFileExcerptSelectorSchema,
    max_lines: Option<u64>,
    repo_root: Option<String>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DocsSelectorKindSchema {
    Heading,
    Line,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GetDocsSectionSelectorSchema {
    /// Selector kind: heading or line.
    kind: DocsSelectorKindSchema,
    /// Heading path or slug when kind='heading'.
    heading: Option<String>,
    /// 1-based line number when kind='line'.
    line: Option<u64>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GetDocsSectionArgsSchema {
    file: String,
    selector: GetDocsSectionSelectorSchema,
    max_bytes: Option<u64>,
    repo_root: Option<String>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadFileAroundMatchArgsSchema {
    file: String,
    query: String,
    is_regex: Option<bool>,
    case_sensitive: Option<bool>,
    before: Option<u64>,
    after: Option<u64>,
    max_matches: Option<u64>,
    max_lines: Option<u64>,
    repo_root: Option<String>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TemplateKindSchema {
    Html,
    Jinja,
    Handlebars,
    Tera,
    Mako,
    Mustache,
    Twig,
    Liquid,
    Erb,
    Haml,
    Pug,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchTemplatesArgsSchema {
    kind: Option<TemplateKindSchema>,
    globs: Option<Vec<String>>,
    exclude_globs: Option<Vec<String>>,
    subpath: Option<String>,
    case_sensitive: Option<bool>,
    max_results: Option<u64>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TextAssetKindSchema {
    Sql,
    Config,
    Env,
    Prompt,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchTextAssetsArgsSchema {
    kind: Option<TextAssetKindSchema>,
    globs: Option<Vec<String>>,
    exclude_globs: Option<Vec<String>>,
    subpath: Option<String>,
    case_sensitive: Option<bool>,
    max_results: Option<u64>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RepoScopeKindSchema {
    Current,
    RepoId,
    All,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RepoScopeArgsSchema {
    /// Repo scope kind: current, repo_id, or all.
    kind: RepoScopeKindSchema,
    /// Required when kind='repo_id'. Must be registered and enabled.
    repo_id: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct QueryGraphArgsSchema {
    text: Option<String>,
    kind: Option<String>,
    language: Option<String>,
    limit: Option<u64>,
    semantic: Option<bool>,
    expand: Option<bool>,
    expand_hops: Option<u64>,
    regex: Option<String>,
    subpath: Option<String>,
    exact_dir: Option<String>,
    exclude_subpaths: Option<Vec<String>>,
    fuzzy: Option<bool>,
    hybrid: Option<bool>,
    include_files: Option<bool>,
    repo_scope: Option<RepoScopeArgsSchema>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListSymbolsArgsSchema {
    kind: Option<String>,
    language: Option<String>,
    subpath: Option<String>,
    exact_dir: Option<String>,
    exclude_subpaths: Option<Vec<String>>,
    repo_id: Option<String>,
    limit: Option<u64>,
    offset: Option<u64>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BatchQueryGraphItemArgsSchema {
    /// Symbol name or identifier to search (e.g. 'BalancesTab'). Required unless 'regex' is set.
    text: Option<String>,
    /// Filter by node kind (e.g. 'function', 'struct').
    kind: Option<String>,
    /// Filter by language (e.g. 'rust', 'typescript').
    language: Option<String>,
    /// Maximum results for this query (default 20).
    limit: Option<u64>,
    /// Graph-neighbour expansion on top of FTS (default false). Requires FTS to find at least one hit first.
    semantic: Option<bool>,
    /// Expand results through graph edges after ranking (default false).
    expand: Option<bool>,
    /// Max edge hops when expand=true (default 1).
    expand_hops: Option<u64>,
    /// Regex pattern matched against name and qualified_name via SQL UDF. Must be valid regex crate syntax.
    regex: Option<String>,
    /// Restrict results to nodes whose file_path starts with this prefix.
    subpath: Option<String>,
    /// Enable fuzzy typo recovery (default false).
    fuzzy: Option<bool>,
    /// Enable hybrid FTS + vector retrieval (default false). Requires search.embedding.url in .atlas/config.toml.
    hybrid: Option<bool>,
    /// Include file nodes in the result set (default false).
    include_files: Option<bool>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BatchQueryGraphArgsSchema {
    #[schemars(length(min = 1, max = 20))]
    items: Vec<BatchQueryGraphItemArgsSchema>,
    repo_scope: Option<RepoScopeArgsSchema>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExplainQueryArgsSchema {
    text: Option<String>,
    kind: Option<String>,
    language: Option<String>,
    limit: Option<u64>,
    semantic: Option<bool>,
    regex: Option<String>,
    subpath: Option<String>,
    fuzzy: Option<bool>,
    hybrid: Option<bool>,
    include_files: Option<bool>,
    repo_scope: Option<RepoScopeArgsSchema>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraverseGraphArgsSchema {
    from_qn: String,
    max_depth: Option<u64>,
    max_nodes: Option<u64>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct TraceCallsArgsSchema {
    from: String,
    to: Option<String>,
    direction: Option<String>,
    edge_kinds: Option<Vec<String>>,
    max_depth: Option<u64>,
    max_nodes: Option<u64>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SymbolNeighborsArgsSchema {
    qname: String,
    limit: Option<u64>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CrossFileLinksArgsSchema {
    file: String,
    limit: Option<u64>,
    include_weak: Option<bool>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConceptClustersArgsSchema {
    files: Vec<String>,
    limit: Option<u64>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResolveSymbolArgsSchema {
    name: String,
    kind: Option<String>,
    file: Option<String>,
    language: Option<String>,
    limit: Option<u64>,
    fuzzy: Option<bool>,
    repo_scope: Option<RepoScopeArgsSchema>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ChangeSourceWithFilesKindSchema {
    Files,
    Base,
    Staged,
    WorkingTree,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ChangeSourceWithoutFilesKindSchema {
    Base,
    Staged,
    WorkingTree,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChangeSourceWithFilesArgsSchema {
    /// Change source kind: files, base, staged, or working_tree.
    kind: ChangeSourceWithFilesKindSchema,
    /// Explicit repo-relative files when kind='files'.
    files: Option<Vec<String>>,
    /// Base git ref when kind='base' (e.g. origin/main).
    base: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChangeSourceWithoutFilesArgsSchema {
    /// Change source kind: base, staged, or working_tree.
    kind: ChangeSourceWithoutFilesKindSchema,
    /// Base git ref when kind='base' (e.g. origin/main).
    base: Option<String>,
}

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

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GetSessionStatusArgsSchema {
    session_id: Option<String>,
    agent_id: Option<String>,
    merge_agent_partitions: Option<bool>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordSessionEventArgsSchema {
    event: String,
    payload: Option<serde_json::Map<String, Value>>,
    frontend: Option<String>,
    session_id: Option<String>,
    agent_id: Option<String>,
    repo_scope: Option<SingleRepoScopeArgsSchema>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct WakeUpArgsSchema {
    topic: Option<String>,
    session_id: Option<String>,
    frontend: Option<String>,
    agent_id: Option<String>,
    max_items: Option<u64>,
    repo_scope: Option<SingleRepoScopeArgsSchema>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CompactSessionArgsSchema {
    session_id: Option<String>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResumeSessionArgsSchema {
    session_id: Option<String>,
    agent_id: Option<String>,
    merge_agent_partitions: Option<bool>,
    mark_consumed: Option<bool>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchSavedContextArgsSchema {
    query: String,
    repo_scope: Option<RepoScopeArgsSchema>,
    session_id: Option<String>,
    agent_id: Option<String>,
    merge_agent_partitions: Option<bool>,
    source_type: Option<String>,
    limit: Option<u64>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SearchDecisionsArgsSchema {
    query: String,
    session_id: Option<String>,
    limit: Option<u64>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadSavedContextArgsSchema {
    source_id: String,
    session_id: Option<String>,
    agent_id: Option<String>,
    merge_agent_partitions: Option<bool>,
    chunk_offset: Option<u64>,
    max_bytes: Option<u64>,
    repo_scope: Option<RepoScopeArgsSchema>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct SaveContextArtifactArgsSchema {
    content: String,
    label: String,
    source_type: Option<String>,
    session_id: Option<String>,
    agent_id: Option<String>,
    repo_scope: Option<RepoScopeArgsSchema>,
    content_type: Option<String>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct PurgeSavedContextArgsSchema {
    session_id: Option<String>,
    agent_id: Option<String>,
    keep_days: Option<u64>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CrossSessionSearchArgsSchema {
    query: String,
    source_type: Option<String>,
    agent_id: Option<String>,
    merge_agent_partitions: Option<bool>,
    limit: Option<u64>,
    repo_scope: Option<RepoScopeArgsSchema>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GetGlobalMemoryArgsSchema {
    limit: Option<u64>,
    focus_symbols: Option<Vec<String>>,
    focus_files: Option<Vec<String>>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MemoryImportanceSchema {
    Critical,
    High,
    Normal,
    Low,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MemoryScopeSchema {
    Project,
    Session,
    Frontend,
    Global,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemoryStoreArgsSchema {
    text: Option<String>,
    topic: Option<String>,
    title: Option<String>,
    importance: Option<MemoryImportanceSchema>,
    scope: Option<MemoryScopeSchema>,
    frontend: Option<String>,
    source_id: Option<String>,
    patch_id: Option<String>,
    patch_file: Option<String>,
    patch_text: Option<String>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemoryRecallArgsSchema {
    query: String,
    topic: Option<String>,
    importance: Option<MemoryImportanceSchema>,
    scope: Option<MemoryScopeSchema>,
    shared: Option<bool>,
    limit: Option<u64>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemoryOpsArgsSchema {
    memory_id: String,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FeedbackRecordArgsSchema {
    predicted: String,
    actual: String,
    correction: Option<String>,
    tool: Option<String>,
    analysis_kind: Option<String>,
    symbol: Option<String>,
    file: Option<String>,
    source_id: Option<String>,
    output_format: Option<String>,
}
