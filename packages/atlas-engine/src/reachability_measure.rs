//! Weak-edge reachability measurement.
//!
//! `resolve_same_package_target` resolves calls by Cargo/npm/go owner
//! membership plus same-directory proximity. That owner is a package, not a
//! compiler inclusion unit, so a candidate can live in an orphan file or a
//! separate crate target and still receive a `same_package` edge.
//!
//! This module measures that weak layer deterministically. It never infers
//! reachability from directory scans; it only classifies persisted weak edges
//! by standard Cargo layout names so the guard work can be gated on measured
//! evidence.

use anyhow::Result;
use atlas_store_sqlite::Store;
use serde::{Deserialize, Serialize};

/// Confidence tier produced by weak same-package fallback resolution.
pub const SAME_PACKAGE_TIER: &str = "same_package";

/// Default number of weak edges inspected per measurement run.
pub const DEFAULT_MEASUREMENT_SAMPLE_LIMIT: usize = 5000;

/// Source marker recorded in measurement output.
pub const MEASUREMENT_SOURCE: &str = "edges.confidence_tier";

/// Standard Cargo file layout kind used for measurement only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RustFileKind {
    /// `src/lib.rs`.
    Lib,
    /// `src/main.rs` or `src/bin/*.rs`.
    Main,
    /// `examples/*.rs`.
    Example,
    /// `tests/*.rs`.
    Test,
    /// `benches/*.rs`.
    Bench,
    /// Anything else, including files outside a recognizable Cargo target.
    Other,
}

impl RustFileKind {
    /// Stable snake-case label used in measurement output.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lib => "lib",
            Self::Main => "main",
            Self::Example => "example",
            Self::Test => "test",
            Self::Bench => "bench",
            Self::Other => "other",
        }
    }
}

/// Deterministic classification of a Rust file path by Cargo layout.
///
/// Only standard Cargo target names count. Unrecognized paths return
/// [`RustFileKind::Other`]; this function never consults the filesystem.
pub fn classify_rust_file_path(path: &str) -> RustFileKind {
    let normalized = path.replace('\\', "/");
    let trimmed = normalized.trim_start_matches("./");
    let segments: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
    let Some(file_name) = segments.last().copied() else {
        return RustFileKind::Other;
    };
    if !file_name.ends_with(".rs") {
        return RustFileKind::Other;
    }

    let parent_is = |name: &str| {
        segments
            .get(segments.len().saturating_sub(2))
            .is_some_and(|segment| *segment == name)
    };
    let has_segment = |name: &str| segments[..segments.len().saturating_sub(1)].contains(&name);

    if file_name == "lib.rs" && parent_is("src") {
        return RustFileKind::Lib;
    }
    if file_name == "main.rs" && parent_is("src") {
        return RustFileKind::Main;
    }
    if has_segment("bin") {
        return RustFileKind::Main;
    }
    if has_segment("examples") {
        return RustFileKind::Example;
    }
    if has_segment("tests") {
        return RustFileKind::Test;
    }
    if has_segment("benches") {
        return RustFileKind::Bench;
    }
    RustFileKind::Other
}

/// Deterministic weak-edge measurement for one repo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReachabilityMeasurement {
    /// All persisted edges whose confidence tier is `same_package`.
    pub total_same_package_edges: usize,
    /// Edges actually inspected (bounded by `sample_limit`).
    pub sampled_edges: usize,
    /// Sampled edges crossing distinct known Cargo root kinds.
    pub cross_crate_root_edges: usize,
    /// Sampled edges whose candidate file is not a known Cargo root kind.
    pub orphan_candidate_edges: usize,
    /// Sampled edges whose classification could not be determined.
    pub unknown_edges: usize,
    /// Provenance marker for how the measurement was produced.
    pub measurement_source: String,
    /// Sample bound applied to the persisted edge table.
    pub sample_limit: usize,
}

/// Measure `same_package` edges for one repo.
///
/// The measurement samples at most `sample_limit` weak edges ordered by
/// `(source_file, target_qn, kind)` so repeated runs are byte-stable.
pub fn measure_same_package_edges(
    store: &Store,
    source_repo_id: &str,
    sample_limit: usize,
) -> Result<ReachabilityMeasurement> {
    let total_same_package_edges =
        store.count_edges_with_confidence_tier(source_repo_id, SAME_PACKAGE_TIER)?;
    let samples =
        store.edges_with_confidence_tier(source_repo_id, SAME_PACKAGE_TIER, sample_limit)?;

    let mut cross_crate_root_edges = 0usize;
    let mut orphan_candidate_edges = 0usize;
    let mut unknown_edges = 0usize;

    for sample in &samples {
        let caller_kind = classify_rust_file_path(&sample.source_file);
        let Some(target_file) = sample.target_file.as_deref() else {
            unknown_edges += 1;
            continue;
        };
        let candidate_kind = classify_rust_file_path(target_file);

        match (caller_kind, candidate_kind) {
            (RustFileKind::Other, _) => unknown_edges += 1,
            (_, RustFileKind::Other) => orphan_candidate_edges += 1,
            (caller, candidate) if caller != candidate => cross_crate_root_edges += 1,
            _ => {}
        }
    }

    Ok(ReachabilityMeasurement {
        total_same_package_edges,
        sampled_edges: samples.len(),
        cross_crate_root_edges,
        orphan_candidate_edges,
        unknown_edges,
        measurement_source: MEASUREMENT_SOURCE.to_owned(),
        sample_limit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_standard_cargo_layouts() {
        assert_eq!(classify_rust_file_path("src/lib.rs"), RustFileKind::Lib);
        assert_eq!(
            classify_rust_file_path("crates/foo/src/lib.rs"),
            RustFileKind::Lib
        );
        assert_eq!(classify_rust_file_path("src/main.rs"), RustFileKind::Main);
        assert_eq!(
            classify_rust_file_path("src/bin/tool.rs"),
            RustFileKind::Main
        );
        assert_eq!(
            classify_rust_file_path("crates/foo/src/bin/worker.rs"),
            RustFileKind::Main
        );
        assert_eq!(
            classify_rust_file_path("tests/integration.rs"),
            RustFileKind::Test
        );
        assert_eq!(
            classify_rust_file_path("crates/foo/tests/it/main.rs"),
            RustFileKind::Test
        );
        assert_eq!(
            classify_rust_file_path("examples/demo.rs"),
            RustFileKind::Example
        );
        assert_eq!(
            classify_rust_file_path("benches/context_bench.rs"),
            RustFileKind::Bench
        );
        assert_eq!(
            classify_rust_file_path("src/orphan.rs"),
            RustFileKind::Other
        );
        assert_eq!(
            classify_rust_file_path("fuzz/fuzz_targets/seed.rs"),
            RustFileKind::Other
        );
        assert_eq!(classify_rust_file_path("src/lib.txt"), RustFileKind::Other);
    }

    #[test]
    fn same_kind_differs_only_when_paths_differ() {
        assert_ne!(
            classify_rust_file_path("src/lib.rs"),
            classify_rust_file_path("src/main.rs")
        );
        assert_eq!(
            classify_rust_file_path("crates/a/src/lib.rs"),
            classify_rust_file_path("crates/b/src/lib.rs")
        );
    }
}
