//! Assembly of [`DocsData`] from the graph store and insight engines.

use anyhow::Context;
use atlas_core::format_rfc3339;
use atlas_engine::{Config, paths::atlas_dir};
use atlas_reasoning::{
    ComponentLabelRequest, DuplicateDetectionRequest, InsightsEngine, ModuleInferenceAnalysis,
};
use atlas_store_sqlite::Store;

use crate::model::DocsData;

/// Load every ingredient for docs rendering from `store` and the insight
/// engines, using the repository at `repo_root`.
///
/// The timestamp is produced here so the returned [`DocsData`] is
/// self-contained and reproducible when `repo_root` and the graph are
/// unchanged; callers that need stable snapshots should pass an explicit
/// `generated_at` instead (see [`load_docs_context_with_timestamp`]).
pub fn load_docs_context(store: &Store, repo_root: &str) -> anyhow::Result<DocsData> {
    load_docs_context_with_timestamp(store, repo_root, None)
}

/// Like [`load_docs_context`] but with an explicit RFC 3339 timestamp.
///
/// `generated_at` is used verbatim (callers are responsible for validating
/// the format), which makes generated docs deterministic for tests and CI.
pub fn load_docs_context_with_timestamp(
    store: &Store,
    repo_root: &str,
    generated_at: Option<String>,
) -> anyhow::Result<DocsData> {
    let atlas_dir = atlas_dir(repo_root);
    let config = Config::load(&atlas_dir).context("cannot load .atlas/config.toml")?;
    let insights = config
        .insights
        .with_loaded_layer_rules(&atlas_dir)
        .context("cannot load insights layer rules")?;
    let engine =
        InsightsEngine::new(store, insights).context("cannot initialize insights engine")?;

    let module_analysis: ModuleInferenceAnalysis = engine
        .infer_modules(repo_root)
        .context("module inference failed")?;
    let component_analysis = engine
        .label_components(
            repo_root,
            ComponentLabelRequest {
                files: None,
                symbols: None,
                limit: None,
            },
        )
        .context("component labeling failed")?;
    let duplicate_analysis = engine
        .find_duplicates(
            repo_root,
            DuplicateDetectionRequest {
                files: None,
                limit: None,
                min_score: None,
                include_tests: true,
                suppressions: Vec::new(),
            },
        )
        .context("duplicate detection failed")?;

    let generated_at = generated_at.unwrap_or_else(|| format_rfc3339(atlas_core::now_utc()));

    // Strip synthetic multi-repo registry rows (`.atlas/synthetic/…`) from
    // every list and derive stats from the filtered data so index.md and the
    // per-file/per-symbol documents always describe the same repository.
    let mut files = store.list_files().context("cannot list indexed files")?;
    let mut nodes = store.list_all_nodes().context("cannot list graph nodes")?;
    let mut edges = store.list_all_edges().context("cannot list graph edges")?;
    files.retain(|file| !DocsData::is_synthetic_path(&file.path));
    nodes.retain(|node| !DocsData::is_synthetic_path(&node.file_path));
    edges.retain(|edge| !DocsData::is_synthetic_path(&edge.file_path));

    let mut nodes_by_kind = std::collections::BTreeMap::<String, i64>::new();
    let mut languages = std::collections::BTreeSet::<String>::new();
    for node in &nodes {
        *nodes_by_kind
            .entry(node.kind.as_str().to_owned())
            .or_default() += 1;
        languages.insert(node.language.clone());
    }
    let raw_stats = store.stats().context("cannot load graph stats")?;
    let stats = atlas_core::GraphStats {
        file_count: i64::try_from(files.len()).unwrap_or(i64::MAX),
        node_count: i64::try_from(nodes.len()).unwrap_or(i64::MAX),
        edge_count: i64::try_from(edges.len()).unwrap_or(i64::MAX),
        nodes_by_kind: nodes_by_kind.into_iter().collect(),
        languages: languages.into_iter().collect(),
        last_indexed_at: raw_stats.last_indexed_at,
    };

    Ok(DocsData {
        repo_root: repo_root.to_owned(),
        stats,
        files,
        nodes,
        edges,
        modules: module_analysis.modules,
        component_assignments: component_analysis.assignments,
        duplicate_groups: duplicate_analysis.groups,
        generated_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind};

    fn make_store() -> Store {
        let mut store = Store::open(":memory:").unwrap();
        store.migrate().unwrap();
        store
    }

    fn node(name: &str, file: &str) -> Node {
        Node {
            id: NodeId(1),
            kind: NodeKind::Function,
            name: name.to_owned(),
            qualified_name: name.to_owned(),
            file_path: file.to_owned(),
            line_start: 1,
            line_end: 10,
            language: "rust".to_owned(),
            parent_name: None,
            params: None,
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: "hash".to_owned(),
            extra_json: serde_json::Value::Null,
            repo_provenance: None,
        }
    }

    #[test]
    fn context_strips_synthetic_registry_rows_and_derives_stats() {
        let repo = tempfile::tempdir().unwrap();
        let repo_root = repo.path().to_str().unwrap();
        let mut store = make_store();
        store
            .replace_file_graph(
                "src/lib.rs",
                "hash-lib",
                Some("rust"),
                None,
                &[node("greet", "src/lib.rs")],
                &[],
            )
            .unwrap();
        store
            .replace_file_graph(
                ".atlas/synthetic/repos/registry.atlas",
                "hash-registry",
                Some("atlas"),
                None,
                &[node("registry", ".atlas/synthetic/repos/registry.atlas")],
                &[Edge {
                    id: 0,
                    kind: EdgeKind::References,
                    source_qn: "registry".to_owned(),
                    target_qn: "greet".to_owned(),
                    file_path: ".atlas/synthetic/repos/registry.atlas".to_owned(),
                    line: None,
                    confidence: 1.0,
                    confidence_tier: Some("high".to_owned()),
                    extra_json: serde_json::Value::Null,
                    repo_provenance: None,
                }],
            )
            .unwrap();

        let data = load_docs_context_with_timestamp(
            &store,
            repo_root,
            Some("2026-01-01T00:00:00Z".into()),
        )
        .unwrap();
        assert_eq!(data.files.len(), 1);
        assert_eq!(data.files[0].path, "src/lib.rs");
        assert_eq!(data.nodes.len(), 1);
        assert_eq!(data.nodes[0].qualified_name, "greet");
        assert!(data.edges.is_empty(), "synthetic edges must be stripped");
        assert_eq!(data.stats.file_count, 1);
        assert_eq!(data.stats.node_count, 1);
        assert_eq!(data.stats.edge_count, 0);
        assert_eq!(data.stats.nodes_by_kind, vec![("function".to_owned(), 1)]);
        assert_eq!(data.stats.languages, vec!["rust".to_owned()]);
        assert_eq!(data.generated_at, "2026-01-01T00:00:00Z");
    }
}
