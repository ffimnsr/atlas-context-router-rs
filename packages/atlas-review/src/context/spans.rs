use super::*;

pub(super) fn apply_code_spans(result: &mut ContextResult) {
    use std::collections::HashMap as FMap;

    let mut span_map: FMap<String, Vec<(u32, u32)>> = FMap::new();

    for sn in &result.nodes {
        let start = sn.node.line_start;
        let end = sn.node.line_end.max(start);

        let include = matches!(
            sn.selection_reason,
            SelectionReason::DirectTarget
                | SelectionReason::Caller
                | SelectionReason::Callee
                | SelectionReason::ImpactNeighbor
        );
        if include {
            span_map
                .entry(sn.node.file_path.clone())
                .or_default()
                .push((start, end));
        }
    }

    for sf in &mut result.files {
        if let Some(spans) = span_map.get(&sf.path) {
            sf.line_ranges = merge_spans(spans);
        }
    }
}

pub(super) fn merge_spans(spans: &[(u32, u32)]) -> Vec<(u32, u32)> {
    if spans.is_empty() {
        return vec![];
    }
    let mut sorted = spans.to_vec();
    sorted.sort_by_key(|&(s, _)| s);

    let mut merged: Vec<(u32, u32)> = Vec::with_capacity(sorted.len());
    let (mut cur_start, mut cur_end) = sorted[0];

    for &(start, end) in &sorted[1..] {
        if start <= cur_end + 1 {
            cur_end = cur_end.max(end);
        } else {
            merged.push((cur_start, cur_end));
            cur_start = start;
            cur_end = end;
        }
    }
    merged.push((cur_start, cur_end));
    merged
}
