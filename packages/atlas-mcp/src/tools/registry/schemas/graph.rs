use super::*;

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
pub(crate) struct ListCommunitiesArgsSchema {
    include_nodes: Option<bool>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListFlowsArgsSchema {
    name: Option<String>,
    node_qn: Option<String>,
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
