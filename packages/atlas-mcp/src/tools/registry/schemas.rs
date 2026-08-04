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
    start_line: u64,
    end_line: u64,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadFileExcerptSelectorSchema {
    kind: ExcerptSelectorKindSchema,
    start_line: Option<u64>,
    end_line: Option<u64>,
    line: Option<u64>,
    before: Option<u64>,
    after: Option<u64>,
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
    kind: DocsSelectorKindSchema,
    heading: Option<String>,
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
    kind: RepoScopeKindSchema,
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
    fuzzy: Option<bool>,
    hybrid: Option<bool>,
    include_files: Option<bool>,
    repo_scope: Option<RepoScopeArgsSchema>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BatchQueryGraphItemArgsSchema {
    text: Option<String>,
    kind: Option<String>,
    language: Option<String>,
    limit: Option<u64>,
    semantic: Option<bool>,
    expand: Option<bool>,
    expand_hops: Option<u64>,
    regex: Option<String>,
    subpath: Option<String>,
    fuzzy: Option<bool>,
    hybrid: Option<bool>,
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
    kind: ChangeSourceWithFilesKindSchema,
    files: Option<Vec<String>>,
    base: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChangeSourceWithoutFilesArgsSchema {
    kind: ChangeSourceWithoutFilesKindSchema,
    base: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BuildOperationKindSchema {
    Build,
    Update,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BuildOperationArgsSchema {
    kind: BuildOperationKindSchema,
    change_source: Option<ChangeSourceWithFilesArgsSchema>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct BuildOrUpdateGraphArgsSchema {
    operation: Option<BuildOperationArgsSchema>,
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
    kind: GetContextTargetKindSchema,
    query: Option<String>,
    file: Option<String>,
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
    kind: SingleRepoScopeKindSchema,
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
    text: String,
    topic: Option<String>,
    title: Option<String>,
    importance: Option<MemoryImportanceSchema>,
    scope: Option<MemoryScopeSchema>,
    frontend: Option<String>,
    source_id: Option<String>,
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
