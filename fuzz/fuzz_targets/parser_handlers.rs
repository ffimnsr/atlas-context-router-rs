#![no_main]

use atlas_fuzz::{ParserCase, hash_bytes};
use atlas_parser::ParserRegistry;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|case: ParserCase| {
    let registry = ParserRegistry::with_defaults();
    let rel_path = case.path_kind.rel_path();
    let file_hash = hash_bytes(&case.source);

    let first = registry.parse(rel_path, &file_hash, &case.source, None);
    if !case.reuse_old_tree {
        return;
    }

    let Some((_, old_tree)) = first else {
        return;
    };
    let Some(old_tree) = old_tree.as_ref() else {
        return;
    };

    let next_hash = hash_bytes(&case.next_source);
    let _ = registry.parse(rel_path, &next_hash, &case.next_source, Some(old_tree));
});
