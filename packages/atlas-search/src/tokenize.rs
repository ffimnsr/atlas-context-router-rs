// ---------------------------------------------------------------------------
// Token splitting
// ---------------------------------------------------------------------------

/// Split a camelCase identifier into its component words.
///
/// Examples:
///   `"ReplaceFileGraph"` → `["Replace", "File", "Graph"]`
///   `"camelCase"` → `["camel", "Case"]`
pub(super) fn split_camel(s: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();

    let chars: Vec<char> = s.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        let prev_lower = i > 0 && chars[i - 1].is_lowercase();
        let next_lower = i + 1 < chars.len() && chars[i + 1].is_lowercase();
        if ch.is_uppercase() && i > 0 && (prev_lower || next_lower) && !current.is_empty() {
            parts.push(current.clone());
            current.clear();
        }
        current.push(ch);
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

/// Build an FTS5 query string from user input, adding token variants from
/// camelCase and snake_case splitting so that "ReplaceFileGraph" also matches
/// documents containing "replace", "file", or "graph".
///
/// The original term is always preserved as the leading token to keep it
/// highest-priority for BM25.
pub fn build_fts_query(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return trimmed.to_string();
    }

    let mut tokens: Vec<String> = vec![trimmed.to_lowercase()];

    // camelCase splitting
    let camel_parts = split_camel(trimmed);
    if camel_parts.len() > 1 {
        tokens.extend(camel_parts.iter().map(|s| s.to_lowercase()));
    }

    // snake_case splitting
    let snake_parts: Vec<&str> = trimmed.split('_').filter(|s| !s.is_empty()).collect();
    if snake_parts.len() > 1 {
        tokens.extend(snake_parts.iter().map(|s| s.to_lowercase()));
    }

    // whitespace splitting (multi-word input)
    let word_parts: Vec<&str> = trimmed.split_whitespace().collect();
    if word_parts.len() > 1 {
        tokens.extend(word_parts.iter().map(|s| s.to_lowercase()));
    }

    tokens.dedup();
    tokens.join(" OR ")
}

pub(super) fn matches_subpath(file_path: &str, subpath: Option<&str>) -> bool {
    match subpath.map(str::trim).filter(|value| !value.is_empty()) {
        Some(prefix) => file_path.starts_with(prefix),
        None => true,
    }
}

/// Build a relaxed FTS5 query for typo-tolerant lookup.
///
/// Uses short prefix wildcards derived from the original token and any
/// camelCase / snake_case splits so a typo like `greter` can still retrieve
/// `greet_twice` candidates before fuzzy ranking runs.
pub(super) fn build_relaxed_fts_query(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut prefixes: Vec<String> = Vec::new();
    let lower = trimmed.to_lowercase();
    if let Some(prefix) = relaxed_prefix(&lower) {
        prefixes.push(prefix);
    }

    let camel_parts = split_camel(trimmed);
    if camel_parts.len() > 1 {
        for part in camel_parts {
            if let Some(prefix) = relaxed_prefix(&part.to_lowercase()) {
                prefixes.push(prefix);
            }
        }
    }

    let snake_parts: Vec<&str> = trimmed.split('_').filter(|part| !part.is_empty()).collect();
    if snake_parts.len() > 1 {
        for part in snake_parts {
            if let Some(prefix) = relaxed_prefix(&part.to_lowercase()) {
                prefixes.push(prefix);
            }
        }
    }

    let word_parts: Vec<&str> = trimmed.split_whitespace().collect();
    if word_parts.len() > 1 {
        for part in word_parts {
            if let Some(prefix) = relaxed_prefix(&part.to_lowercase()) {
                prefixes.push(prefix);
            }
        }
    }

    prefixes.dedup();
    prefixes.join(" OR ")
}

pub(super) fn relaxed_prefix(token: &str) -> Option<String> {
    let len = token.chars().count();
    if len < 4 {
        return None;
    }

    let prefix_len = if len >= 6 { 3 } else { 2 };
    let prefix: String = token.chars().take(prefix_len).collect();
    Some(format!("{prefix}*"))
}
