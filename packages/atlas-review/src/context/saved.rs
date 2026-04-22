use super::*;

/// Maximum number of saved-context sources to include in a result.
const MAX_SAVED_SOURCES: usize = 5;

pub(super) fn retrieve_saved_context(
    content_store: &ContentStore,
    request: &ContextRequest,
    result: &ContextResult,
) -> Vec<SavedContextSource> {
    let mut terms: Vec<String> = result
        .nodes
        .iter()
        .take(5)
        .map(|sn| sn.node.name.clone())
        .collect();
    for sf in result.files.iter().take(3) {
        let basename = sf.path.rsplit('/').next().unwrap_or(&sf.path);
        terms.push(basename.to_string());
    }
    terms.dedup();

    if terms.is_empty() {
        return vec![];
    }

    let query = terms.join(" ");
    let filters = SearchFilters {
        session_id: request.session_id.clone(),
        ..SearchFilters::default()
    };

    let chunks = match content_store.search_with_fallback(&query, &filters) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let mut seen_ids: Vec<String> = Vec::new();
    for chunk in &chunks {
        if !seen_ids.contains(&chunk.source_id) {
            seen_ids.push(chunk.source_id.clone());
            if seen_ids.len() >= MAX_SAVED_SOURCES {
                break;
            }
        }
    }

    let seven_days_ago = {
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let cutoff = now_secs.saturating_sub(7 * 24 * 60 * 60);
        let secs = cutoff;
        let days_since_epoch = secs / 86400;
        let rem = secs % 86400;
        let hours = rem / 3600;
        let minutes = (rem % 3600) / 60;
        let seconds = rem % 60;
        let (year, month, day) = epoch_days_to_ymd(days_since_epoch);
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
            year, month, day, hours, minutes, seconds
        )
    };

    let mut scored: Vec<SavedContextSource> = Vec::new();
    for (rank, source_id) in seen_ids.iter().enumerate() {
        let meta: SourceRow = match content_store.get_source(source_id) {
            Ok(Some(m)) => m,
            _ => continue,
        };
        let preview: String = chunks
            .iter()
            .find(|c| &c.source_id == source_id)
            .map(|c| c.content.chars().take(512).collect())
            .unwrap_or_default();

        let mut score = 10.0_f32 / (rank as f32 + 1.0);

        if meta.created_at.as_str() >= seven_days_ago.as_str() {
            score += 5.0;
        }

        if let (Some(req_sid), Some(art_sid)) =
            (request.session_id.as_deref(), meta.session_id.as_deref())
            && req_sid == art_sid
        {
            score += 10.0;
        }

        let retrieval_hint = format!(
            "source_id={} label={:?} type={}",
            source_id, meta.label, meta.source_type
        );

        scored.push(SavedContextSource {
            source_id: source_id.clone(),
            label: meta.label,
            source_type: meta.source_type,
            session_id: meta.session_id,
            preview,
            retrieval_hint,
            relevance_score: score,
        });
    }

    scored.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored
}

fn epoch_days_to_ymd(days: u64) -> (u64, u8, u8) {
    let z = days as i64 + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as u64, m as u8, d as u8)
}
