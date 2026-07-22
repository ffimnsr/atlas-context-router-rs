use anyhow::Result;
use serde::Serialize;
use serde_json::{Value, json};

use crate::output::{OutputFormat, render_value};
use crate::tool_result::{ToolErrorCode, ToolErrorPayload, tool_execution_error_value};

use super::manual::suggest_tool_names;
use super::registry::tool_descriptors;

const DEFAULT_SEARCH_LIMIT: usize = 10;
const MAX_SEARCH_LIMIT: usize = 50;

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

    let mut tools = inventory_entries();
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

    let mut matches = inventory_entries()
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

fn tool_inventory_response<T>(payload: &T, output_format: OutputFormat) -> Result<Value>
where
    T: Serialize,
{
    let raw = serde_json::to_value(payload)?;
    let text = match output_format {
        OutputFormat::Json => render_value(&raw, output_format)?.text,
        OutputFormat::Toon => render_inventory_text(&raw),
    };

    Ok(json!({
        "content": [{
            "type": "text",
            "text": text,
            "mimeType": output_format.mime_type(),
        }],
        "structuredContent": raw,
        "_meta": {
            "atlas:outputFormat": output_format.as_str(),
            "atlas:requestedOutputFormat": output_format.as_str(),
        },
    }))
}

fn render_inventory_text(payload: &Value) -> String {
    if let Some(tools) = payload.get("tools").and_then(Value::as_array) {
        let mut lines = vec![format!(
            "tools: {} visible exported MCP tools",
            payload
                .get("returned_tools")
                .and_then(Value::as_u64)
                .unwrap_or(tools.len() as u64)
        )];
        if let Some(category) = payload.get("applied_category").and_then(Value::as_str) {
            lines.push(format!("category: {category}"));
        }
        lines.push(String::new());
        for tool in tools {
            let name = tool.get("name").and_then(Value::as_str).unwrap_or_default();
            let description = tool
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let category = tool
                .get("category")
                .and_then(Value::as_str)
                .unwrap_or_default();
            lines.push(format!("- {name} [{category}] — {description}"));
        }
        lines.push(String::new());
        lines.push(
            "next: use tool_search for fuzzy discovery, tool_help for exact runtime docs"
                .to_owned(),
        );
        return lines.join("\n");
    }

    let matches = payload
        .get("matches")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let query = payload
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let mut lines = vec![format!(
        "tool search: '{}' → {} match(es)",
        query,
        payload
            .get("returned_matches")
            .and_then(Value::as_u64)
            .unwrap_or(matches.len() as u64)
    )];
    lines.push(String::new());
    if matches.is_empty() {
        lines.push("- no direct matches".to_owned());
        if let Some(suggestions) = payload.get("suggestions").and_then(Value::as_array) {
            let values = suggestions
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>();
            if !values.is_empty() {
                lines.push(format!("  suggestions: {}", values.join(", ")));
            }
        }
    } else {
        for item in matches {
            let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
            let description = item
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let reasons = item
                .get("match_reasons")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let score = item
                .get("score")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            let factors = item
                .get("score_factors")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .map(|value| {
                            let factor = value
                                .get("factor")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            let contribution = value
                                .get("contribution")
                                .and_then(Value::as_u64)
                                .unwrap_or_default();
                            let detail = value.get("detail").and_then(Value::as_str);
                            match detail {
                                Some(detail) if !detail.is_empty() => {
                                    format!("{factor}(+{contribution}; {detail})")
                                }
                                _ => format!("{factor}(+{contribution})"),
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            lines.push(format!("- {name} — {description}"));
            lines.push(format!("  score: {score}"));
            if !reasons.is_empty() {
                lines.push(format!("  reasons: {reasons}"));
            }
            if !factors.is_empty() {
                lines.push(format!("  factors: {factors}"));
            }
        }
    }
    lines.push(String::new());
    lines.push("next: use tool_help with exact name for full runtime docs".to_owned());
    lines.join("\n")
}

fn inventory_entries() -> Vec<ToolInventoryEntry> {
    let mut tools = tool_descriptors()
        .into_iter()
        .map(|tool| ToolInventoryEntry {
            category: tool
                .meta
                .get("atlas:category")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned(),
            result_contract: tool
                .meta
                .get("atlas:resultContract")
                .and_then(Value::as_str)
                .unwrap_or("text-only")
                .to_owned(),
            read_only: tool.annotations.read_only_hint,
            state_changing: tool.annotations.state_changing_hint,
            destructive: tool.annotations.destructive_hint,
            name: tool.name,
            title: tool.title,
            description: tool.description,
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
}

fn fuzzy_name_distance(query: &str, candidate: &str) -> Option<usize> {
    if query.len() < 3 || query == candidate || candidate.contains(query) {
        return None;
    }
    let threshold = fuzzy_distance_threshold(query, candidate);
    let distance = levenshtein(query, candidate);
    (distance <= threshold).then_some(distance)
}

fn fuzzy_name_token_match(query: &str, candidate: &str) -> Option<(String, String, usize)> {
    query_tokens(query)
        .filter(|token| token.len() >= 3)
        .flat_map(|token| {
            query_tokens(candidate).filter_map(move |candidate_token| {
                if candidate_token.contains(token) || token == candidate_token {
                    return None;
                }
                let distance = levenshtein(token, candidate_token);
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

fn levenshtein(left: &str, right: &str) -> usize {
    if left == right {
        return 0;
    }
    if left.is_empty() {
        return right.chars().count();
    }
    if right.is_empty() {
        return left.chars().count();
    }

    let right_chars = right.chars().collect::<Vec<_>>();
    let mut prev = (0..=right_chars.len()).collect::<Vec<_>>();
    let mut curr = vec![0usize; right_chars.len() + 1];

    for (left_idx, left_ch) in left.chars().enumerate() {
        curr[0] = left_idx + 1;
        for (right_idx, right_ch) in right_chars.iter().enumerate() {
            let cost = usize::from(left_ch != *right_ch);
            curr[right_idx + 1] = (prev[right_idx + 1] + 1)
                .min(curr[right_idx] + 1)
                .min(prev[right_idx] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }

    prev[right_chars.len()]
}

#[cfg(test)]
mod tests {
    use super::{inventory_entries, score_match};

    #[test]
    fn inventory_entries_are_sorted() {
        let entries = inventory_entries();
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
        let entry = inventory_entries()
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
        let entry = inventory_entries()
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
        let entry = inventory_entries()
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
}
