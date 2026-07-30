use anyhow::{Context, Result};
use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, PackageOwner, ParsedFile};
use atlas_repo::{PackageOwners, RepoRegistry, WorkspaceRoot, hash_file};
use atlas_store_sqlite::Store;
use camino::Utf8Path;

const SYNTHETIC_GRAPH_PREFIX: &str = ".atlas/synthetic/owners/";

pub(crate) fn refresh_owner_graphs(
    store: &mut Store,
    repo_root: &Utf8Path,
    owners: &PackageOwners,
    source_repo_id: &str,
    namespace_qualified_names: bool,
) -> Result<()> {
    let existing_paths = store
        .file_paths_with_prefix_for_repo(source_repo_id, SYNTHETIC_GRAPH_PREFIX)
        .context("cannot list synthetic owner graph files")?;
    for path in existing_paths {
        store
            .delete_file_graph_for_repo(source_repo_id, &path)
            .with_context(|| format!("cannot delete synthetic graph for {path}"))?;
    }

    let package_files: Vec<(ParsedFile, PackageOwner)> = owners
        .all()
        .iter()
        .map(|owner| make_package_file(repo_root, owner, source_repo_id, namespace_qualified_names))
        .collect::<Result<Vec<_>>>()?;
    let workspace_files: Vec<ParsedFile> = owners
        .workspaces()
        .iter()
        .map(|workspace| {
            make_workspace_file(
                repo_root,
                workspace,
                source_repo_id,
                namespace_qualified_names,
            )
        })
        .collect::<Result<Vec<_>>>()?;

    let mut parsed_files: Vec<ParsedFile> = package_files
        .iter()
        .map(|(parsed_file, _)| parsed_file.clone())
        .collect();
    parsed_files.extend(workspace_files);

    if !parsed_files.is_empty() {
        store
            .replace_files_transactional_for_repo(source_repo_id, &parsed_files)
            .context("cannot refresh synthetic owner/workspace nodes")?;
        for (parsed_file, owner) in &package_files {
            store
                .upsert_file_owner(&parsed_file.path, Some(owner))
                .with_context(|| {
                    format!(
                        "cannot store owner metadata for synthetic package {}",
                        parsed_file.path
                    )
                })?;
        }
    }

    Ok(())
}

fn make_package_file(
    repo_root: &Utf8Path,
    owner: &PackageOwner,
    source_repo_id: &str,
    namespace_qualified_names: bool,
) -> Result<(ParsedFile, PackageOwner)> {
    let path = synthetic_package_path(owner);
    let hash = manifest_hash(repo_root, &owner.manifest_path, &owner.owner_id)?;
    let language = manifest_language(owner.kind).to_owned();
    let qualified_name = maybe_namespace_qn(
        &format!("package::{}", owner.owner_id),
        source_repo_id,
        namespace_qualified_names,
    );
    let node = Node {
        id: NodeId::UNSET,
        kind: NodeKind::Package,
        name: synthetic_name(
            owner.package_name.as_deref(),
            &owner.root,
            &owner.manifest_path,
        ),
        qualified_name,
        file_path: path.clone(),
        line_start: 1,
        line_end: 1,
        language: language.clone(),
        parent_name: None,
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: hash.clone(),
        extra_json: serde_json::json!({
            "synthetic_kind": "package_owner",
            "owner_id": owner.owner_id,
            "owner_kind": owner.kind.as_str(),
            "owner_root": owner.root,
            "owner_manifest_path": owner.manifest_path,
            "owner_name": owner.package_name,
            "repo_id": source_repo_id,
        }),
    };

    Ok((
        ParsedFile {
            path,
            language: Some(language),
            hash,
            size: None,
            nodes: vec![node],
            edges: vec![],
        },
        owner.clone(),
    ))
}

fn make_workspace_file(
    repo_root: &Utf8Path,
    workspace: &WorkspaceRoot,
    source_repo_id: &str,
    namespace_qualified_names: bool,
) -> Result<ParsedFile> {
    let path = synthetic_workspace_path(workspace);
    let hash = manifest_hash(repo_root, &workspace.manifest_path, &workspace.workspace_id)?;
    let language = manifest_language(workspace.kind).to_owned();
    let qualified_name = maybe_namespace_qn(
        &format!(
            "workspace::{}::{}",
            workspace.kind.as_str(),
            workspace.manifest_path
        ),
        source_repo_id,
        namespace_qualified_names,
    );
    let node = Node {
        id: NodeId::UNSET,
        kind: NodeKind::Package,
        name: synthetic_name(None, &workspace.root, &workspace.manifest_path),
        qualified_name: qualified_name.clone(),
        file_path: path.clone(),
        line_start: 1,
        line_end: 1,
        language: language.clone(),
        parent_name: None,
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: hash.clone(),
        extra_json: serde_json::json!({
            "synthetic_kind": "workspace_root",
            "workspace_id": workspace.workspace_id,
            "workspace_kind": workspace.kind.as_str(),
            "workspace_root": workspace.root,
            "workspace_manifest_path": workspace.manifest_path,
            "member_roots": workspace.member_roots,
            "member_owner_ids": workspace.member_owner_ids,
            "repo_id": source_repo_id,
        }),
    };
    let mut edges: Vec<Edge> = workspace
        .member_owner_ids
        .iter()
        .map(|owner_id| Edge {
            id: 0,
            kind: EdgeKind::Contains,
            source_qn: qualified_name.clone(),
            target_qn: maybe_namespace_qn(
                &format!("package::{owner_id}"),
                source_repo_id,
                namespace_qualified_names,
            ),
            file_path: path.clone(),
            line: None,
            confidence: 1.0,
            confidence_tier: Some("workspace_membership".to_owned()),
            extra_json: serde_json::json!({
                "synthetic_kind": "workspace_membership",
                "workspace_id": workspace.workspace_id,
                "repo_id": source_repo_id,
            }),
        })
        .collect();

    if let Ok(registry) = RepoRegistry::load_or_bootstrap(repo_root) {
        for member_root in &workspace.member_roots {
            if let Some(registration) = registry.registrations.iter().find(|registration| {
                registration.repo_id != source_repo_id
                    && (registration.display_alias == member_root.as_str()
                        || registration.root.ends_with(member_root.as_str()))
            }) {
                edges.push(Edge {
                    id: 0,
                    kind: EdgeKind::References,
                    source_qn: qualified_name.clone(),
                    target_qn: format!("repo::{}", registration.repo_id),
                    file_path: path.clone(),
                    line: None,
                    confidence: 1.0,
                    confidence_tier: Some("workspace_link".to_owned()),
                    extra_json: serde_json::json!({
                        "synthetic_kind": "workspace_repo_bridge",
                        "workspace_id": workspace.workspace_id,
                        "source_repo": source_repo_id,
                        "target_repo": registration.repo_id,
                        "relationship_reason": "workspace_link",
                        "member_root": member_root,
                    }),
                });
            }
        }
    }

    Ok(ParsedFile {
        path,
        language: Some(language),
        hash,
        size: None,
        nodes: vec![node],
        edges,
    })
}

fn maybe_namespace_qn(qname: &str, source_repo_id: &str, namespace: bool) -> String {
    if namespace {
        format!("repo::{source_repo_id}::{qname}")
    } else {
        qname.to_owned()
    }
}

fn manifest_hash(repo_root: &Utf8Path, manifest_path: &str, fallback: &str) -> Result<String> {
    let manifest = repo_root.join(manifest_path);
    if manifest.exists() {
        return hash_file(&manifest).with_context(|| format!("cannot hash {manifest_path}"));
    }
    Ok(format!("synthetic:{fallback}"))
}

fn synthetic_package_path(owner: &PackageOwner) -> String {
    format!(
        "{SYNTHETIC_GRAPH_PREFIX}package/{}/{}.atlas",
        owner.kind.as_str(),
        slugify(&owner.owner_id)
    )
}

fn synthetic_workspace_path(workspace: &WorkspaceRoot) -> String {
    format!(
        "{SYNTHETIC_GRAPH_PREFIX}workspace/{}/{}.atlas",
        workspace.kind.as_str(),
        slugify(&workspace.workspace_id)
    )
}

fn manifest_language(kind: atlas_core::PackageOwnerKind) -> &'static str {
    match kind {
        atlas_core::PackageOwnerKind::Cargo => "toml",
        atlas_core::PackageOwnerKind::Npm => "json",
        atlas_core::PackageOwnerKind::Go => "go",
    }
}

fn synthetic_name(package_name: Option<&str>, root: &str, manifest_path: &str) -> String {
    if let Some(package_name) = package_name
        && !package_name.is_empty()
    {
        return package_name.to_owned();
    }
    if let Some(name) = root.rsplit('/').find(|segment| !segment.is_empty()) {
        return name.to_owned();
    }
    manifest_path.to_owned()
}

fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => out.push(ch),
            _ => out.push('_'),
        }
    }
    out
}
