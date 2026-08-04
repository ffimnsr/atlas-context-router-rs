use super::*;

#[test]
fn code_spans_populated_for_target() {
    let mut store = open_store();
    seed_graph(&mut store);
    let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
    let mut req = symbol_request("src/a.rs::fn_a");
    req.include_code_spans = true;
    let result = build_symbol_context(&store, seed, &req).unwrap();

    let target_file = result
        .files
        .iter()
        .find(|f| f.path == "src/a.rs")
        .expect("src/a.rs must be in files");
    assert!(
        !target_file.line_ranges.is_empty(),
        "target file must have line ranges"
    );
}

#[test]
fn code_spans_not_populated_when_disabled() {
    let mut store = open_store();
    seed_graph(&mut store);
    let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
    let mut req = symbol_request("src/a.rs::fn_a");
    req.include_code_spans = false;
    let result = build_symbol_context(&store, seed, &req).unwrap();

    for sf in &result.files {
        assert!(
            sf.line_ranges.is_empty(),
            "line_ranges should be empty when spans disabled"
        );
    }
}

#[test]
fn code_spans_merge_overlapping_ranges() {
    let spans = vec![(1u32, 5u32), (3, 8), (15, 20)];
    let merged = super::spans::merge_spans(&spans);
    assert_eq!(merged, vec![(1, 8), (15, 20)]);
}

#[test]
fn code_spans_merge_adjacent_ranges() {
    let spans = vec![(1u32, 5u32), (6, 10)];
    let merged = super::spans::merge_spans(&spans);
    assert_eq!(merged, vec![(1, 10)]);
}

#[test]
fn code_spans_single_range_unchanged() {
    let spans = vec![(10u32, 20u32)];
    let merged = super::spans::merge_spans(&spans);
    assert_eq!(merged, vec![(10, 20)]);
}
