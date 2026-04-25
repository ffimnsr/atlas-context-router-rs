use std::collections::HashMap;

use tracing::{debug, info};

use atlas_core::{AtlasError, Clock, SystemClock, format_rfc3339};

use super::ChunkResult;

/// RRF constant k (typical value 60 from the original paper).
const RRF_K: f64 = 60.0;

pub(super) fn format_now() -> String {
    format_now_with(&SystemClock)
}

pub(super) fn format_days_ago(days: u32) -> String {
    format_days_ago_with(&SystemClock, days)
}

pub(super) fn fts5_escape(input: &str) -> String {
    let has_special = input
        .chars()
        .any(|c| matches!(c, '"' | '(' | ')' | '^' | '-' | '*'));
    if has_special {
        format!("\"{}\"", input.replace('"', "\"\""))
    } else {
        input.to_string()
    }
}

/// Extract vocabulary terms from text for the vocabulary table.
pub(super) fn extract_vocab_terms(text: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for word in text.split(|c: char| !c.is_alphanumeric()) {
        let lower = word.to_lowercase();
        if lower.len() >= 3
            && lower.chars().all(|c| c.is_ascii_alphabetic())
            && seen.insert(lower.clone())
        {
            out.push(lower);
        }
    }
    out
}

/// Reciprocal-Rank Fusion of two ranked result lists.
pub(super) fn rrf_merge(list_a: &[ChunkResult], list_b: &[ChunkResult]) -> Vec<ChunkResult> {
    let mut scores: HashMap<String, f64> = HashMap::new();
    let mut items: HashMap<String, &ChunkResult> = HashMap::new();

    for (rank, chunk) in list_a.iter().enumerate() {
        let key = chunk.chunk_id.clone();
        *scores.entry(key.clone()).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
        items.entry(key).or_insert(chunk);
    }
    for (rank, chunk) in list_b.iter().enumerate() {
        let key = chunk.chunk_id.clone();
        *scores.entry(key.clone()).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
        items.entry(key).or_insert(chunk);
    }

    let mut ranked: Vec<(&ChunkResult, f64)> = scores
        .iter()
        .filter_map(|(key, &score)| items.get(key).map(|chunk| (*chunk, score)))
        .collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    ranked.into_iter().map(|(chunk, _)| chunk.clone()).collect()
}

/// Proximity reranking: boost results where query terms appear close together.
pub(super) fn proximity_rerank(results: &mut [ChunkResult], terms: &[&str]) {
    let score_chunk = |chunk: &ChunkResult| -> i64 {
        let words: Vec<&str> = chunk.content.split_whitespace().collect();
        let n = words.len();
        if n == 0 {
            return 0;
        }
        let lower_words: Vec<String> = words.iter().map(|word| word.to_lowercase()).collect();
        let lower_terms: Vec<String> = terms.iter().map(|term| term.to_lowercase()).collect();

        let positions: Vec<Vec<usize>> = lower_terms
            .iter()
            .map(|term| {
                lower_words
                    .iter()
                    .enumerate()
                    .filter(|(_, word)| word.contains(term.as_str()))
                    .map(|(index, _)| index)
                    .collect()
            })
            .collect();

        let window = 50usize;
        let mut bonus: i64 = 0;
        for i in 0..positions.len() {
            for j in (i + 1)..positions.len() {
                for &left in &positions[i] {
                    for &right in &positions[j] {
                        if left.abs_diff(right) <= window {
                            bonus += 1;
                        }
                    }
                }
            }
        }

        if let Some(ref title) = chunk.title {
            let lower_title = title.to_lowercase();
            for term in &lower_terms {
                if lower_title.contains(term.as_str()) {
                    bonus += 5;
                }
            }
        }
        bonus
    };

    results.sort_by_key(|chunk| std::cmp::Reverse(score_chunk(chunk)));
}

/// Levenshtein edit distance (byte-level; capped at 3 for early exit).
pub(super) fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let m = a.len();
    let n = b.len();
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];
    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
        if *prev.iter().min().unwrap_or(&0) > 2 {
            return 3;
        }
    }
    prev[n]
}

/// Return `true` when error string indicates SQLite database corruption.
pub(super) fn is_corruption_error(err: &AtlasError) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("malformed")
        || msg.contains("not a database")
        || msg.contains("disk image is malformed")
        || msg.contains("database disk image")
        || msg.contains("file is not a database")
}

/// Rename corrupt database file to `{path}.quarantine`.
pub(super) fn quarantine_db(path: &str) {
    let qpath = format!("{path}.quarantine");
    if let Err(e) = std::fs::rename(path, &qpath) {
        debug!("content DB quarantine rename failed: {e}");
    } else {
        info!(
            path = path,
            quarantine = %qpath,
            "corrupt content DB quarantined; a fresh store will be created on next open"
        );
    }
}

fn format_now_with(clock: &dyn Clock) -> String {
    format_rfc3339(clock.now_utc())
}

fn format_days_ago_with(clock: &dyn Clock, days: u32) -> String {
    let duration = time::Duration::days(days as i64);
    let ts = clock.now_utc() - duration;
    format_rfc3339(ts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::FixedClock;
    use time::OffsetDateTime;

    #[test]
    fn injected_clock_formats_contentstore_timestamps() {
        let now = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let clock = FixedClock::new(now);
        assert_eq!(format_now_with(&clock), "2023-11-14T22:13:20Z");
        assert_eq!(format_days_ago_with(&clock, 2), "2023-11-12T22:13:20Z");
    }
}
