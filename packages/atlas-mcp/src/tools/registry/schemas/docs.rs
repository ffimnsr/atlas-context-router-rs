use super::*;

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct HealthOutputFormatArgs {
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GetMetricsArgsSchema {
    verbose: Option<bool>,
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
    max_results: Option<u64>,
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
