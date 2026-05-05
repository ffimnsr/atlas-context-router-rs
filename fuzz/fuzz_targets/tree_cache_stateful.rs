#![no_main]

use arbitrary::{Arbitrary, Unstructured};
use std::collections::HashMap;

use atlas_fuzz::{SupportedPathKind, hash_bytes, split_once_bytes};
use atlas_parser::{ParserRegistry, TreeCache};
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug, Clone)]
pub enum CacheOp {
    Parse {
        path_kind: SupportedPathKind,
        source: Vec<u8>,
    },
    ReparseWithOldTree {
        path_kind: SupportedPathKind,
        source: Vec<u8>,
    },
    Insert {
        path_kind: SupportedPathKind,
        source: Vec<u8>,
    },
    Remove {
        path_kind: SupportedPathKind,
    },
    Evict {
        path_kind: SupportedPathKind,
    },
    RenameKey {
        old_path_kind: SupportedPathKind,
        new_path_kind: SupportedPathKind,
    },
}

#[derive(Arbitrary, Debug)]
pub struct StatefulCacheCase {
    pub ops: Vec<CacheOp>,
}

fuzz_target!(|data: &[u8]| {
    let Some(case) = stateful_cache_case_from_bytes(data) else {
        return;
    };

    let mut cache = TreeCache::new();
    let mut detached_trees = HashMap::new();
    let registry = ParserRegistry::with_defaults();

    for op in case.ops {
        match op {
            CacheOp::Parse { path_kind, source } => {
                let rel_path = path_kind.rel_path();
                let hash = hash_bytes(&source);
                if let Some((_, Some(tree))) = registry.parse(rel_path, &hash, &source, None) {
                    cache.insert(rel_path.to_string(), tree);
                }
            }
            CacheOp::ReparseWithOldTree { path_kind, source } => {
                let rel_path = path_kind.rel_path();
                let hash = hash_bytes(&source);
                let old_tree = cache.remove(rel_path);
                if let Some((_, Some(new_tree))) =
                    registry.parse(rel_path, &hash, &source, old_tree.as_ref())
                {
                    cache.insert(rel_path.to_string(), new_tree);
                }
            }
            CacheOp::Insert { path_kind, source } => {
                let rel_path = path_kind.rel_path();
                if let Some(tree) = detached_trees.remove(rel_path) {
                    cache.insert(rel_path.to_string(), tree);
                    continue;
                }

                let hash = hash_bytes(&source);
                if let Some((_, Some(tree))) = registry.parse(rel_path, &hash, &source, None) {
                    cache.insert(rel_path.to_string(), tree);
                }
            }
            CacheOp::Remove { path_kind } => {
                let rel_path = path_kind.rel_path();
                if let Some(tree) = cache.remove(rel_path) {
                    detached_trees.insert(rel_path.to_string(), tree);
                }
            }
            CacheOp::Evict { path_kind } => {
                let rel_path = path_kind.rel_path();
                cache.evict(rel_path);
                detached_trees.remove(rel_path);
            }
            CacheOp::RenameKey { old_path_kind, new_path_kind } => {
                let old_rel_path = old_path_kind.rel_path();
                let new_rel_path = new_path_kind.rel_path();

                if let Some(tree) = cache.remove(old_rel_path) {
                    cache.insert(new_rel_path.to_string(), tree);
                    cache.evict(old_rel_path);
                    detached_trees.remove(old_rel_path);
                    continue;
                }

                if let Some(tree) = detached_trees.remove(old_rel_path) {
                    cache.insert(new_rel_path.to_string(), tree);
                    cache.evict(old_rel_path);
                }
            }
        }
    }
});

fn stateful_cache_case_from_bytes(data: &[u8]) -> Option<StatefulCacheCase> {
    parse_tree_cache_seed(data).or_else(|| StatefulCacheCase::arbitrary(&mut Unstructured::new(data)).ok())
}

fn parse_tree_cache_seed(data: &[u8]) -> Option<StatefulCacheCase> {
    let body = data.strip_prefix(b"ATLAS_TREE_CACHE_SEED\n")?;
    let (meta, source) = split_once_bytes(body, b"\n===SOURCE===\n")?;
    let path_kind = parse_kind_meta(meta)?;
    let source = source.to_vec();

    Some(StatefulCacheCase {
        ops: vec![
            CacheOp::Parse {
                path_kind,
                source: source.clone(),
            },
            CacheOp::ReparseWithOldTree {
                path_kind,
                source: source.clone(),
            },
            CacheOp::Remove { path_kind },
            CacheOp::Insert { path_kind, source },
        ],
    })
}

fn parse_kind_meta(meta: &[u8]) -> Option<SupportedPathKind> {
    let meta = std::str::from_utf8(meta).ok()?;
    meta.lines()
        .find_map(|line| line.strip_prefix("kind="))
        .and_then(SupportedPathKind::from_seed_name)
}

#[cfg(test)]
mod tests {
    use super::{CacheOp, parse_tree_cache_seed};
    use atlas_fuzz::SupportedPathKind;

    #[test]
    fn tree_cache_seed_expands_to_lifecycle_ops() {
        let seed = concat!(
            "ATLAS_TREE_CACHE_SEED\n",
            "kind=json\n",
            "===SOURCE===\n",
            "{\"ok\":true}\n"
        );

        let case = parse_tree_cache_seed(seed.as_bytes()).expect("seed should decode");
        assert_eq!(case.ops.len(), 4);
        assert!(matches!(case.ops[0], CacheOp::Parse { path_kind: SupportedPathKind::Json, .. }));
        assert!(matches!(case.ops[1], CacheOp::ReparseWithOldTree { path_kind: SupportedPathKind::Json, .. }));
        assert!(matches!(case.ops[2], CacheOp::Remove { path_kind: SupportedPathKind::Json }));
        assert!(matches!(case.ops[3], CacheOp::Insert { path_kind: SupportedPathKind::Json, .. }));
    }
}
