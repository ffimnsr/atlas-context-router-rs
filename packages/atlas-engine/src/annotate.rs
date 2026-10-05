//! Per-file annotation shared by the full-build and incremental-update
//! pipelines.
//!
//! Both pipelines parse files in parallel and then annotate every parsed file
//! with repo provenance and package-owner metadata before persisting it.  The
//! logic used to be duplicated in `build.rs` and `update.rs`; it lives here so
//! both paths stay byte-for-byte consistent.

use atlas_core::{PackageOwner, model::ParsedFile};
use atlas_repo::stable_repo_fingerprint;
use camino::Utf8Path;
use serde_json::Value;

/// Namespace `node`/`edge` qualified names with `repo::<repo_id>::` and attach
/// repo provenance to every entity.
///
/// Namespacing is skipped when the qualified name already carries a `repo::`
/// prefix, so repeated annotation is idempotent.
pub(crate) fn annotate_parsed_file_repo(
    parsed_file: &mut ParsedFile,
    repo_id: &str,
    repo_root: &str,
    namespace_qnames: bool,
) {
    if namespace_qnames {
        for node in &mut parsed_file.nodes {
            let original = node.qualified_name.clone();
            node.qualified_name = namespace_qname(repo_id, &original);
            node.parent_name = node
                .parent_name
                .as_deref()
                .map(|parent| namespace_qname(repo_id, parent));
        }
        for edge in &mut parsed_file.edges {
            edge.source_qn = namespace_qname(repo_id, &edge.source_qn);
            edge.target_qn = namespace_qname(repo_id, &edge.target_qn);
        }
    }

    let repo_provenance = atlas_core::RepoProvenance::new(repo_id.to_owned())
        .with_repo_fingerprint(stable_repo_fingerprint(Utf8Path::new(repo_root), None))
        .with_repo_root(repo_root.to_owned());

    // The same provenance payload is attached to every node and edge, so
    // serialize it once per file instead of once per entity on this hot path.
    let repo_id_json = Value::String(repo_id.to_owned());
    let repo_root_json = Value::String(repo_root.to_owned());
    let provenance_json = serde_json::to_value(&repo_provenance).unwrap_or(Value::Null);

    for node in &mut parsed_file.nodes {
        let mut extra = node.extra_json.as_object().cloned().unwrap_or_default();
        extra.insert("repo_id".to_owned(), repo_id_json.clone());
        extra.insert("repo_root".to_owned(), repo_root_json.clone());
        extra.insert("repo_provenance".to_owned(), provenance_json.clone());
        node.extra_json = Value::Object(extra);
        node.repo_provenance = Some(repo_provenance.clone());
    }

    for edge in &mut parsed_file.edges {
        let mut extra = edge.extra_json.as_object().cloned().unwrap_or_default();
        extra.insert("repo_id".to_owned(), repo_id_json.clone());
        extra.insert("repo_root".to_owned(), repo_root_json.clone());
        extra.insert("repo_provenance".to_owned(), provenance_json.clone());
        edge.extra_json = Value::Object(extra);
        edge.repo_provenance = Some(repo_provenance.clone());
    }
}

/// Attach package-owner metadata to every node in `parsed_file`.
///
/// No-op when `owner` is `None`.
pub(crate) fn annotate_parsed_file_owner(
    parsed_file: &mut ParsedFile,
    owner: Option<&PackageOwner>,
) {
    let Some(owner) = owner else {
        return;
    };
    for node in &mut parsed_file.nodes {
        let mut extra = node.extra_json.as_object().cloned().unwrap_or_default();
        extra.insert("owner_id".to_owned(), Value::String(owner.owner_id.clone()));
        extra.insert(
            "owner_kind".to_owned(),
            Value::String(owner.kind.as_str().to_owned()),
        );
        extra.insert("owner_root".to_owned(), Value::String(owner.root.clone()));
        extra.insert(
            "owner_manifest_path".to_owned(),
            Value::String(owner.manifest_path.clone()),
        );
        if let Some(package_name) = &owner.package_name {
            extra.insert("owner_name".to_owned(), Value::String(package_name.clone()));
        }
        node.extra_json = Value::Object(extra);
    }
}

fn namespace_qname(repo_id: &str, qname: &str) -> String {
    if qname.starts_with("repo::") {
        qname.to_owned()
    } else {
        format!("repo::{repo_id}::{qname}")
    }
}

#[cfg(test)]
mod tests {
    use super::{annotate_parsed_file_repo, namespace_qname};
    use atlas_core::model::ParsedFile;

    fn sample_parsed_file() -> ParsedFile {
        let node = atlas_core::Node {
            id: atlas_core::NodeId::UNSET,
            kind: atlas_core::NodeKind::Function,
            name: "run".to_owned(),
            qualified_name: "src/lib.rs::fn::run".to_owned(),
            file_path: "src/lib.rs".to_owned(),
            line_start: 1,
            line_end: 3,
            language: "rust".to_owned(),
            parent_name: Some("src/lib.rs".to_owned()),
            params: None,
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: "hash123".to_owned(),
            extra_json: serde_json::json!({ "keep": true }),
            repo_provenance: None,
        };
        let edge = atlas_core::Edge {
            id: 0,
            kind: atlas_core::EdgeKind::Contains,
            source_qn: "src/lib.rs".to_owned(),
            target_qn: "src/lib.rs::fn::run".to_owned(),
            file_path: "src/lib.rs".to_owned(),
            line: Some(1),
            confidence: 1.0,
            confidence_tier: Some("definite".to_owned()),
            extra_json: serde_json::json!({ "keep_edge": true }),
            repo_provenance: None,
        };
        ParsedFile {
            path: "src/lib.rs".to_owned(),
            language: Some("rust".to_owned()),
            hash: "hash123".to_owned(),
            size: Some(10),
            nodes: vec![node],
            edges: vec![edge],
        }
    }

    #[test]
    fn namespace_qname_prefixes_once() {
        assert_eq!(
            namespace_qname("r1", "src/lib.rs::fn::x"),
            "repo::r1::src/lib.rs::fn::x"
        );
        assert_eq!(
            namespace_qname("r1", "repo::other::src/lib.rs::fn::x"),
            "repo::other::src/lib.rs::fn::x"
        );
    }

    #[test]
    fn annotates_namespaced_repo_provenance_and_is_idempotent() {
        let repo_root = env!("CARGO_MANIFEST_DIR");
        let mut parsed_file = sample_parsed_file();
        annotate_parsed_file_repo(&mut parsed_file, "repo_a", repo_root, true);

        let node = &parsed_file.nodes[0];
        assert_eq!(node.qualified_name, "repo::repo_a::src/lib.rs::fn::run");
        assert_eq!(
            node.parent_name.as_deref(),
            Some("repo::repo_a::src/lib.rs")
        );
        assert_eq!(node.extra_json.get("keep"), Some(&serde_json::json!(true)));
        assert_eq!(
            node.extra_json.get("repo_id"),
            Some(&serde_json::json!("repo_a"))
        );
        assert_eq!(
            node.extra_json.get("repo_root"),
            Some(&serde_json::json!(repo_root))
        );
        assert!(node.extra_json.get("repo_provenance").is_some());
        assert!(node.repo_provenance.is_some());

        let edge = &parsed_file.edges[0];
        assert_eq!(edge.source_qn, "repo::repo_a::src/lib.rs");
        assert_eq!(edge.target_qn, "repo::repo_a::src/lib.rs::fn::run");
        assert!(edge.extra_json.get("repo_provenance").is_some());
        assert!(edge.repo_provenance.is_some());

        // Re-running must not double-prefix namespaced identifiers.
        annotate_parsed_file_repo(&mut parsed_file, "repo_a", repo_root, true);
        assert_eq!(
            parsed_file.nodes[0].qualified_name,
            "repo::repo_a::src/lib.rs::fn::run"
        );
        assert_eq!(
            parsed_file.edges[0].target_qn,
            "repo::repo_a::src/lib.rs::fn::run"
        );
    }
}
