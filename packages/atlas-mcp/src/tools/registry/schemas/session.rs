use super::*;

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
    purge_bridge_files: Option<bool>,
    confirm: Option<bool>,
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
    tags: Option<Vec<String>>,
    links: Option<Vec<MemoryLinkSchema>>,
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
    tags: Option<Vec<String>>,
    shared: Option<bool>,
    limit: Option<u64>,
    output_format: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemoryLinkSchema {
    /// Target memory id.
    id: String,
    /// Relation from the memoir vocabulary plus `supersedes`.
    relation: String,
}

#[allow(dead_code)]
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemoryLinkArgsSchema {
    memory_id: String,
    target_id: String,
    relation: Option<String>,
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
