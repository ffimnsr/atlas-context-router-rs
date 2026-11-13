use anyhow::Result;
use atlas_repo::RepoRegistry;
use camino::Utf8Path;
use serde::Serialize;
use serde_json::{Value, json};

use crate::output::{OutputFormat, render_value};
use crate::tool_result::{ToolErrorCode, ToolErrorPayload, tool_execution_error_value};

use super::manual::suggest_tool_names;
use super::registry::tool_descriptors;

const DEFAULT_SEARCH_LIMIT: usize = 10;
const MAX_SEARCH_LIMIT: usize = 50;

/// Natural-language glue words carry no tool-name signal, so they are dropped
/// before token scoring. Without this an intent query like
/// "trace callers and callees" surfaces every tool whose description happens
/// to contain "and".
const QUERY_STOPWORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "how", "if", "in", "into",
    "is", "it", "of", "on", "or", "that", "the", "this", "to", "use", "using", "was", "what",
    "when", "where", "which", "with",
];

/// Minimum token length for token-level fuzzy matching. Shorter tokens produce
/// distance-1 noise (e.g. `dead` ~ `read`), so only full-name fuzzy matching is
/// allowed for them.
const MIN_FUZZY_TOKEN_CHARS: usize = 5;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ToolInventoryEntry {
    pub name: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub result_contract: String,
    pub read_only: bool,
    pub state_changing: bool,
    pub destructive: bool,
    pub idempotent: bool,
    pub open_world: bool,
    #[serde(rename = "inputSchema", skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ToolInventoryGuidance {
    pub list: String,
    pub search: String,
    pub help: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ToolListResponse {
    pub total_tools: usize,
    pub returned_tools: usize,
    pub applied_category: Option<String>,
    pub tools: Vec<ToolInventoryEntry>,
    pub guidance: ToolInventoryGuidance,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ToolSearchFactor {
    pub factor: String,
    pub contribution: u32,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ToolSearchMatch {
    pub name: String,
    pub title: String,
    pub description: String,
    pub category: String,
    pub result_contract: String,
    pub score: u32,
    pub match_reasons: Vec<String>,
    pub score_factors: Vec<ToolSearchFactor>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ToolSearchResponse {
    pub query: String,
    pub total_matches: usize,
    pub returned_matches: usize,
    pub matches: Vec<ToolSearchMatch>,
    pub suggestions: Vec<String>,
    pub guidance: ToolInventoryGuidance,
}

pub(crate) fn tool_tool_list(args: Option<&Value>, output_format: OutputFormat) -> Result<Value> {
    let category = args
        .and_then(|value| value.get("category"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let include_schemas = args
        .and_then(|value| value.get("include_schemas"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let mut tools = inventory_entries(include_schemas);
    if let Some(category_filter) = category.as_ref() {
        let category_lower = category_filter.to_ascii_lowercase();
        tools.retain(|tool| tool.category.eq_ignore_ascii_case(&category_lower));
    }

    let response = ToolListResponse {
        total_tools: tools.len(),
        returned_tools: tools.len(),
        applied_category: category,
        tools,
        guidance: guidance(),
    };

    tool_inventory_response(&response, output_format)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct RepoRegistryResponse {
    pub registry_path: String,
    pub schema_version: u32,
    pub root_repo_id: String,
    pub registration_count: usize,
    pub warning_count: usize,
    pub registrations: Vec<atlas_repo::RepoRegistration>,
    pub warnings: Vec<atlas_repo::RepoRegistryWarning>,
}

pub(crate) fn tool_repo_registry(repo_root: &str, output_format: OutputFormat) -> Result<Value> {
    let registry = RepoRegistry::load_or_bootstrap(Utf8Path::new(repo_root))?;
    let response = RepoRegistryResponse {
        registry_path: atlas_repo::registry_path(Utf8Path::new(repo_root)).to_string(),
        schema_version: registry.schema_version,
        root_repo_id: registry.root_repo_id,
        registration_count: registry.registrations.len(),
        warning_count: registry.warnings.len(),
        registrations: registry.registrations,
        warnings: registry.warnings,
    };
    tool_inventory_response(&response, output_format)
}

pub(crate) fn tool_tool_search(args: Option<&Value>, output_format: OutputFormat) -> Result<Value> {
    let query = args
        .and_then(|value| value.get("query"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    if query.is_empty() {
        let payload = ToolErrorPayload::new(
            ToolErrorCode::InvalidInput,
            "tool_search requires non-empty query",
        )
        .with_tool("tool_search")
        .with_retry_guidance("Use tool_list for full inventory, or retry with a short tool name fragment like 'query', 'context', or 'search'.")
        .with_details(json!({
            "retry_examples": [
                { "query": "query" },
                { "query": "context" },
                { "query": "search" }
            ]
        }));
        return tool_execution_error_value(output_format, &payload);
    }

    let limit = args
        .and_then(|value| value.get("limit"))
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(DEFAULT_SEARCH_LIMIT)
        .clamp(1, MAX_SEARCH_LIMIT);

    let mut matches = inventory_entries(false)
        .into_iter()
        .filter_map(|tool| score_match(query, &tool))
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.name.cmp(&right.name))
    });

    let total_matches = matches.len();
    matches.truncate(limit);
    let suggestions = if total_matches == 0 {
        suggest_tool_names(query)
    } else {
        Vec::new()
    };

    let response = ToolSearchResponse {
        query: query.to_owned(),
        total_matches,
        returned_matches: matches.len(),
        matches,
        suggestions,
        guidance: guidance(),
    };

    tool_inventory_response(&response, output_format)
}

fn tool_inventory_response<T>(payload: &T, _output_format: OutputFormat) -> Result<Value>
where
    T: Serialize,
{
    let raw = serde_json::to_value(payload)?;
    let text = render_value(&raw)?.text;

    Ok(json!({
        "content": [{
            "type": "text",
            "text": text,
        }],
        "structuredContent": raw,
        "_meta": {},
    }))
}

fn inventory_entries(include_schemas: bool) -> Vec<ToolInventoryEntry> {
    let mut tools = tool_descriptors()
        .into_iter()
        .map(|tool| {
            let read_only = tool
                .annotations
                .as_ref()
                .and_then(|annotations| annotations.read_only_hint)
                .unwrap_or(false);
            let destructive = tool
                .annotations
                .as_ref()
                .and_then(|annotations| annotations.destructive_hint)
                .unwrap_or(false);
            let idempotent = tool
                .annotations
                .as_ref()
                .and_then(|annotations| annotations.idempotent_hint)
                .unwrap_or(true);
            let open_world = tool
                .annotations
                .as_ref()
                .and_then(|annotations| annotations.open_world_hint)
                .unwrap_or(false);
            ToolInventoryEntry {
                category: tool
                    .meta
                    .as_ref()
                    .and_then(|meta| meta.get("atlas:category"))
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_owned(),
                result_contract: tool
                    .meta
                    .as_ref()
                    .and_then(|meta| meta.get("atlas:resultContract"))
                    .and_then(Value::as_str)
                    .unwrap_or("text-only")
                    .to_owned(),
                input_schema: include_schemas.then(|| Value::Object((*tool.input_schema).clone())),
                read_only,
                state_changing: !read_only,
                destructive,
                idempotent,
                open_world,
                name: tool.name.to_string(),
                title: tool.title.unwrap_or_default(),
                description: tool
                    .description
                    .map(|value| value.into_owned())
                    .unwrap_or_default(),
            }
        })
        .collect::<Vec<_>>();
    tools.sort_by(|left, right| left.name.cmp(&right.name));
    tools
}

fn guidance() -> ToolInventoryGuidance {
    ToolInventoryGuidance {
        list: "tool_list".to_owned(),
        search: "tool_search { query }".to_owned(),
        help: "tool_help { name }".to_owned(),
    }
}

fn score_match(query: &str, tool: &ToolInventoryEntry) -> Option<ToolSearchMatch> {
    let query_lower = query.to_ascii_lowercase();
    let name_lower = tool.name.to_ascii_lowercase();
    let title_lower = tool.title.to_ascii_lowercase();
    let description_lower = tool.description.to_ascii_lowercase();

    let mut score = 0u32;
    let mut reasons = Vec::new();
    let mut factors = Vec::new();

    if name_lower == query_lower {
        push_factor(
            &mut score,
            &mut reasons,
            &mut factors,
            500,
            "exact name",
            "exact_name",
            None,
        );
    }
    if name_lower.starts_with(&query_lower) {
        push_factor(
            &mut score,
            &mut reasons,
            &mut factors,
            250,
            "name prefix",
            "name_prefix",
            None,
        );
    }
    if name_lower.contains(&query_lower) {
        push_factor(
            &mut score,
            &mut reasons,
            &mut factors,
            150,
            "name contains",
            "name_contains",
            None,
        );
    }
    if title_lower.contains(&query_lower) {
        push_factor(
            &mut score,
            &mut reasons,
            &mut factors,
            100,
            "title contains",
            "title_contains",
            None,
        );
    }
    if description_lower.contains(&query_lower) {
        push_factor(
            &mut score,
            &mut reasons,
            &mut factors,
            60,
            "description contains",
            "description_contains",
            None,
        );
    }

    for token in query_tokens(&query_lower) {
        if name_lower.contains(token) {
            push_factor(
                &mut score,
                &mut reasons,
                &mut factors,
                25,
                &format!("name token:{token}"),
                "name_token",
                Some(token.to_owned()),
            );
        }
        if title_lower.contains(token) {
            push_factor(
                &mut score,
                &mut reasons,
                &mut factors,
                15,
                &format!("title token:{token}"),
                "title_token",
                Some(token.to_owned()),
            );
        }
        if description_lower.contains(token) {
            push_factor(
                &mut score,
                &mut reasons,
                &mut factors,
                10,
                &format!("description token:{token}"),
                "description_token",
                Some(token.to_owned()),
            );
        }
    }

    if let Some(distance) = fuzzy_name_distance(&query_lower, &name_lower) {
        push_factor(
            &mut score,
            &mut reasons,
            &mut factors,
            120,
            &format!("fuzzy name distance:{distance}"),
            "fuzzy_name",
            Some(format!("distance={distance}")),
        );
    }

    if let Some((token, candidate, distance)) = fuzzy_name_token_match(&query_lower, &name_lower) {
        push_factor(
            &mut score,
            &mut reasons,
            &mut factors,
            40,
            &format!("fuzzy name token:{token}~{candidate}:{distance}"),
            "fuzzy_name_token",
            Some(format!("{token}~{candidate} distance={distance}")),
        );
    }

    if score == 0 {
        return None;
    }

    reasons.sort();
    reasons.dedup();
    factors.sort_by(|left, right| {
        right
            .contribution
            .cmp(&left.contribution)
            .then_with(|| left.factor.cmp(&right.factor))
            .then_with(|| left.detail.cmp(&right.detail))
    });
    factors.dedup();

    Some(ToolSearchMatch {
        name: tool.name.clone(),
        title: tool.title.clone(),
        description: tool.description.clone(),
        category: tool.category.clone(),
        result_contract: tool.result_contract.clone(),
        score,
        match_reasons: reasons,
        score_factors: factors,
    })
}

fn push_factor(
    score: &mut u32,
    reasons: &mut Vec<String>,
    factors: &mut Vec<ToolSearchFactor>,
    contribution: u32,
    reason: &str,
    factor: &str,
    detail: Option<String>,
) {
    *score += contribution;
    reasons.push(reason.to_owned());
    factors.push(ToolSearchFactor {
        factor: factor.to_owned(),
        contribution,
        detail,
    });
}

fn query_tokens(query: &str) -> impl Iterator<Item = &str> {
    query
        .split(|ch: char| ch.is_whitespace() || matches!(ch, '.' | '_' | '-'))
        .filter(|token| token.len() >= 2)
        .filter(|token| !QUERY_STOPWORDS.contains(token))
}

fn fuzzy_name_distance(query: &str, candidate: &str) -> Option<usize> {
    if query.len() < 3 || query == candidate || candidate.contains(query) {
        return None;
    }
    let threshold = fuzzy_distance_threshold(query, candidate);
    let distance = strsim::levenshtein(query, candidate);
    (distance <= threshold).then_some(distance)
}

fn fuzzy_name_token_match(query: &str, candidate: &str) -> Option<(String, String, usize)> {
    query_tokens(query)
        .filter(|token| token.len() >= MIN_FUZZY_TOKEN_CHARS)
        .flat_map(|token| {
            query_tokens(candidate).filter_map(move |candidate_token| {
                if candidate_token.contains(token) || token == candidate_token {
                    return None;
                }
                let distance = strsim::levenshtein(token, candidate_token);
                (distance <= fuzzy_distance_threshold(token, candidate_token)).then_some((
                    token.to_owned(),
                    candidate_token.to_owned(),
                    distance,
                ))
            })
        })
        .min_by(|left, right| {
            left.2
                .cmp(&right.2)
                .then_with(|| left.1.len().cmp(&right.1.len()))
                .then_with(|| left.1.cmp(&right.1))
        })
}

fn fuzzy_distance_threshold(left: &str, right: &str) -> usize {
    match left.chars().count().min(right.chars().count()) {
        0..=4 => 1,
        5..=8 => 2,
        _ => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::{inventory_entries, score_match, tool_tool_list, tool_tool_search};
    use crate::output::OutputFormat;
    use serde_json::json;

    #[test]
    fn inventory_entries_are_sorted() {
        let entries = inventory_entries(false);
        let mut names = entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
        names.dedup();
        assert_eq!(names.len(), entries.len());
    }

    #[test]
    fn score_match_prefers_exact_name() {
        let entry = inventory_entries(false)
            .into_iter()
            .find(|tool| tool.name == "query_graph")
            .expect("query_graph inventory entry");
        let exact = score_match("query_graph", &entry).expect("exact match");
        let partial = score_match("query", &entry).expect("partial match");
        assert!(exact.score > partial.score);
        assert!(
            exact
                .match_reasons
                .iter()
                .any(|reason| reason == "exact name")
        );
        assert!(
            exact
                .score_factors
                .iter()
                .any(|factor| factor.factor == "exact_name" && factor.contribution == 500)
        );
    }

    #[test]
    fn score_match_supports_fuzzy_name_typos() {
        let entry = inventory_entries(false)
            .into_iter()
            .find(|tool| tool.name == "query_graph")
            .expect("query_graph inventory entry");
        let fuzzy = score_match("qurey_graph", &entry).expect("fuzzy match");
        assert!(
            fuzzy
                .match_reasons
                .iter()
                .any(|reason| reason.starts_with("fuzzy name distance:"))
        );
        assert!(
            fuzzy
                .score_factors
                .iter()
                .any(|factor| factor.factor == "fuzzy_name")
        );
    }

    #[test]
    fn score_match_supports_fuzzy_token_typos() {
        let entry = inventory_entries(false)
            .into_iter()
            .find(|tool| tool.name == "get_context")
            .expect("get_context inventory entry");
        let fuzzy = score_match("cntxt", &entry).expect("fuzzy token match");
        assert!(
            fuzzy
                .match_reasons
                .iter()
                .any(|reason| reason.starts_with("fuzzy name token:"))
        );
        assert!(
            fuzzy
                .score_factors
                .iter()
                .any(|factor| factor.factor == "fuzzy_name_token")
        );
    }

    #[test]
    fn score_match_ignores_stopwords() {
        let entry = inventory_entries(false)
            .into_iter()
            .find(|tool| tool.name == "read_file_excerpt")
            .expect("read_file_excerpt inventory entry");
        assert!(
            score_match("trace callers and callees", &entry).is_none(),
            "stopword/glue tokens must not surface unrelated tools"
        );
    }

    #[test]
    fn score_match_rejects_short_fuzzy_token_noise() {
        for name in [
            "read_file_excerpt",
            "read_file_around_match",
            "read_saved_context",
        ] {
            let entry = inventory_entries(false)
                .into_iter()
                .find(|tool| tool.name == name)
                .expect("inventory entry");
            assert!(
                score_match("dead code detection", &entry).is_none(),
                "distance-1 short-token fuzzy match must not surface {name}"
            );
        }
    }

    #[test]
    fn tool_search_matches_trace_calls_for_caller_callee_intent() {
        let response = tool_tool_search(
            Some(&json!({ "query": "trace callers and callees", "limit": 10 })),
            OutputFormat::Json,
        )
        .expect("tool_search response");
        let names = response["structuredContent"]["matches"]
            .as_array()
            .expect("matches array")
            .iter()
            .map(|item| item["name"].as_str().expect("match name"))
            .collect::<Vec<_>>();
        assert!(names.contains(&"trace_calls"));
        assert!(names.contains(&"symbol_neighbors"));
        assert!(!names.contains(&"read_file_excerpt"));
    }

    #[test]
    fn tool_list_only_includes_input_schemas_on_request() {
        let compact = tool_tool_list(None, OutputFormat::Json).expect("compact tool_list");
        let compact_tools = compact["structuredContent"]["tools"]
            .as_array()
            .expect("tools array");
        assert!(
            compact_tools
                .iter()
                .all(|tool| tool.get("inputSchema").is_none())
        );

        let full = tool_tool_list(
            Some(&json!({ "include_schemas": true })),
            OutputFormat::Json,
        )
        .expect("full tool_list");
        let full_tools = full["structuredContent"]["tools"]
            .as_array()
            .expect("tools array");
        assert!(
            full_tools
                .iter()
                .all(|tool| tool.get("inputSchema").is_some())
        );
    }
}
