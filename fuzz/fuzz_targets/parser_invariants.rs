#![no_main]

use std::collections::HashMap;

use atlas_core::NodeKind;
use atlas_fuzz::{hash_bytes, source_seed_case_from_bytes};
use atlas_parser::ParserRegistry;
use libfuzzer_sys::fuzz_target;

const MAX_SOURCE_BYTES: usize = 16 * 1024;

fuzz_target!(|data: &[u8]| {
    let Some(case) = source_seed_case_from_bytes(data) else {
        return;
    };

    let source = bounded_source(&case.source);
    let rel_path = case.path_kind.rel_path();
    let file_hash = hash_bytes(&source);
    let registry = ParserRegistry::with_defaults();

    let Some((parsed, _tree)) = registry.parse(rel_path, &file_hash, &source, None) else {
        return;
    };

    assert_eq!(parsed.path, rel_path, "ParsedFile.path must echo input path");

    if let Some(size) = parsed.size {
        assert_eq!(
            size,
            source.len() as i64,
            "ParsedFile.size must match source length when populated"
        );
    }

    let file_node_count = parsed
        .nodes
        .iter()
        .filter(|node| node.kind == NodeKind::File)
        .count();
    assert_eq!(
        file_node_count, 1,
        "supported parse result must contain exactly one file node"
    );

    for node in &parsed.nodes {
        assert_eq!(
            node.file_path, rel_path,
            "node.file_path must stay anchored to input path"
        );
        assert!(
            !node.qualified_name.is_empty(),
            "node.qualified_name must not be empty"
        );
        assert!(node.line_start >= 1, "node.line_start must be 1-based");
        assert!(
            node.line_end >= node.line_start,
            "node.line_end must not precede line_start"
        );
    }

    for edge in &parsed.edges {
        assert!(
            !edge.source_qn.is_empty(),
            "edge.source_qn must not be empty"
        );
        assert!(
            !edge.target_qn.is_empty(),
            "edge.target_qn must not be empty"
        );
    }

    // Duplicate qualified names stay advisory for now. Some language models may
    // intentionally collapse overloaded or synthetic symbols, so fuzzing should
    // observe duplicates without asserting until the validity rules are explicit.
    std::hint::black_box(duplicate_qn_advisory(&parsed));
});

fn bounded_source(source: &[u8]) -> Vec<u8> {
    source[..source.len().min(MAX_SOURCE_BYTES)].to_vec()
}

fn duplicate_qn_advisory(parsed: &atlas_core::ParsedFile) -> Vec<(String, usize)> {
    let mut counts = HashMap::new();
    for node in &parsed.nodes {
        *counts.entry(node.qualified_name.as_str()).or_insert(0usize) += 1;
    }

    counts
        .into_iter()
        .filter_map(|(qualified_name, count)| {
            (count > 1).then(|| (qualified_name.to_owned(), count))
        })
        .collect()
}
