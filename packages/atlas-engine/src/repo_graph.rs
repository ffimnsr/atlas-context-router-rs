use anyhow::{Context, Result};
use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile};
use atlas_repo::{RepoRegistry, RepoRelationshipKind};
use atlas_store_sqlite::Store;

const SYNTHETIC_REPO_SOURCE_ID: &str = "registry";
const SYNTHETIC_REPO_GRAPH_PATH: &str = ".atlas/synthetic/repos/registry.atlas";

pub fn refresh_repo_registry_graph(store: &mut Store, registry: &RepoRegistry) -> Result<()> {
    for path in store
        .file_paths_with_prefix_for_repo(SYNTHETIC_REPO_SOURCE_ID, ".atlas/synthetic/repos/")
        .context("cannot list synthetic repo registry graph files")?
    {
        store
            .delete_file_graph_for_repo(SYNTHETIC_REPO_SOURCE_ID, &path)
            .with_context(|| format!("cannot delete synthetic repo graph for {path}"))?;
    }

    let parsed_file = make_registry_file(registry);
    store
        .replace_files_transactional_for_repo(SYNTHETIC_REPO_SOURCE_ID, &[parsed_file])
        .context("cannot refresh synthetic repo registry graph")?;
    Ok(())
}

fn make_registry_file(registry: &RepoRegistry) -> ParsedFile {
    let registry_qn = "registry::multi_repo".to_owned();
    let mut nodes = vec![Node {
        id: NodeId::UNSET,
        kind: NodeKind::Package,
        name: "multi_repo_registry".to_owned(),
        qualified_name: registry_qn.clone(),
        file_path: SYNTHETIC_REPO_GRAPH_PATH.to_owned(),
        line_start: 1,
        line_end: 1,
        language: "atlas".to_owned(),
        parent_name: None,
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: registry_hash(registry),
        extra_json: serde_json::json!({
            "synthetic_kind": "repo_registry",
            "schema_version": registry.schema_version,
            "root_repo_id": registry.root_repo_id,
            "repo_id": SYNTHETIC_REPO_SOURCE_ID,
        }),
    }];

    let mut edges = Vec::new();
    for registration in &registry.registrations {
        let registration_repo_qn = repo_qn(&registration.repo_id);
        nodes.push(Node {
            id: NodeId::UNSET,
            kind: NodeKind::Package,
            name: registration.display_alias.clone(),
            qualified_name: registration_repo_qn.clone(),
            file_path: SYNTHETIC_REPO_GRAPH_PATH.to_owned(),
            line_start: 1,
            line_end: 1,
            language: "atlas".to_owned(),
            parent_name: Some(registry_qn.clone()),
            params: None,
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: registry_hash(registry),
            extra_json: serde_json::json!({
                "synthetic_kind": "repo",
                "repo_id": registration.repo_id,
                "root": registration.root,
                "display_alias": registration.display_alias,
                "relationship_kind": format!("{:?}", registration.relationship.kind).to_ascii_lowercase(),
                "enabled": registration.enabled,
                "trust_state": format!("{:?}", registration.trust_state).to_ascii_lowercase(),
                "head": registration.vcs.head,
                "default_branch": registration.vcs.default_branch,
                "remote_url": registration.vcs.remote_url,
            }),
        });
        edges.push(Edge {
            id: 0,
            kind: EdgeKind::Contains,
            source_qn: registry_qn.clone(),
            target_qn: registration_repo_qn.clone(),
            file_path: SYNTHETIC_REPO_GRAPH_PATH.to_owned(),
            line: None,
            confidence: 1.0,
            confidence_tier: Some("registry_contains_repo".to_owned()),
            extra_json: serde_json::json!({
                "synthetic_kind": "registry_contains_repo",
                "repo_id": registration.repo_id,
            }),
        });
        if let Some(parent_repo_id) = &registration.relationship.parent_repo_id
            && registration.relationship.kind == RepoRelationshipKind::Submodule
        {
            edges.push(Edge {
                id: 0,
                kind: EdgeKind::References,
                source_qn: repo_qn(parent_repo_id),
                target_qn: registration_repo_qn.clone(),
                file_path: SYNTHETIC_REPO_GRAPH_PATH.to_owned(),
                line: None,
                confidence: 1.0,
                confidence_tier: Some("repo_submodule_of".to_owned()),
                extra_json: serde_json::json!({
                    "synthetic_kind": "repo_submodule_of",
                    "parent_repo_id": parent_repo_id,
                    "child_repo_id": registration.repo_id,
                    "parent_path": registration.relationship.parent_path,
                }),
            });
        }
        for dependency in &registration.dependencies {
            edges.push(Edge {
                id: 0,
                kind: EdgeKind::References,
                source_qn: registration_repo_qn.clone(),
                target_qn: repo_qn(&dependency.repo_id),
                file_path: SYNTHETIC_REPO_GRAPH_PATH.to_owned(),
                line: None,
                confidence: 1.0,
                confidence_tier: Some("repo_depends_on_repo".to_owned()),
                extra_json: serde_json::json!({
                    "synthetic_kind": "repo_depends_on_repo",
                    "repo_id": registration.repo_id,
                    "dependency_repo_id": dependency.repo_id,
                    "dependency_kind": dependency.kind,
                }),
            });
        }
    }

    ParsedFile {
        path: SYNTHETIC_REPO_GRAPH_PATH.to_owned(),
        language: Some("atlas".to_owned()),
        hash: registry_hash(registry),
        size: None,
        nodes,
        edges,
    }
}

fn repo_qn(repo_id: &str) -> String {
    format!("repo::{repo_id}")
}

fn registry_hash(registry: &RepoRegistry) -> String {
    let encoded = serde_json::to_string(registry).unwrap_or_default();
    format!("synthetic:repo-registry:{}", encoded.len())
}
