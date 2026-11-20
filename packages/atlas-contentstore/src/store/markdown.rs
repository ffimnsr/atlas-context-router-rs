//! M4 — markdown truth files for saved context artifacts.
//!
//! `.atlas/artifacts/<source_id>.md` holds the verbatim payload plus the
//! retrieval metadata. `context.db` keeps only derived chunk/FTS/vocabulary
//! indexes rebuilt from these files.

use std::path::{Path, PathBuf};

use atlas_core::{AtlasError, Result};
use atlas_store_markdown::slug::slugify;
use atlas_store_markdown::{Frontmatter, MarkdownRecord, MarkdownStore, Value};

use crate::store::SourceMeta;

pub(super) const ARTIFACTS_DIR: &str = "artifacts";

fn artifact_path(source_id: &str) -> PathBuf {
    let name = slugify(source_id);
    Path::new(ARTIFACTS_DIR).join(format!("{name}.md"))
}

/// Write the artifact truth file. The body is the verbatim payload; frontmatter
/// carries retrieval metadata needed to rebuild the derived index.
pub(super) fn write_artifact(
    store: &MarkdownStore,
    meta: &SourceMeta,
    raw_text: &str,
    content_type: &str,
    created_at: &str,
) -> Result<()> {
    let mut frontmatter = Frontmatter::new();
    frontmatter.set("id", Value::str(meta.id.clone()));
    frontmatter.set("label", Value::str(meta.label.clone()));
    frontmatter.set("source_type", Value::str(meta.source_type.clone()));
    frontmatter.set(
        "session_id",
        meta.session_id.clone().map_or(Value::Null, Value::str),
    );
    frontmatter.set(
        "agent_id",
        meta.agent_id.clone().map_or(Value::Null, Value::str),
    );
    frontmatter.set("content_type", Value::str(content_type));
    frontmatter.set("identity_kind", Value::str(meta.identity_kind.clone()));
    frontmatter.set("identity_value", Value::str(meta.identity_value.clone()));
    frontmatter.set(
        "repo_roots",
        Value::list(meta.repo_roots.iter().cloned().map(Value::Str)),
    );
    frontmatter.set(
        "repo_ids",
        Value::list(meta.repo_ids.iter().cloned().map(Value::Str)),
    );
    frontmatter.set("created_at", Value::str(created_at));
    let record = MarkdownRecord::new(frontmatter, raw_text);
    let relative = artifact_path(&meta.id);
    store
        .with_writer_lock("artifacts", || store.write_record(&relative, &record))
        .map_err(|error| AtlasError::Other(format!("artifact markdown: {error}")))
}

/// Remove one artifact truth file.
pub(super) fn delete_artifact(store: &MarkdownStore, source_id: &str) -> Result<()> {
    let relative = artifact_path(source_id);
    store
        .delete_record(&relative)
        .map(|_| ())
        .map_err(|error| AtlasError::Other(format!("delete artifact markdown: {error}")))
}

/// Metadata reconstructed from one artifact file.
pub(super) struct ArtifactFile {
    pub source_id: String,
    pub label: String,
    pub source_type: String,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub content_type: String,
    pub identity_kind: String,
    pub identity_value: String,
    pub repo_roots: Vec<String>,
    pub repo_ids: Vec<String>,
    pub body: String,
}

/// Read every artifact markdown file in the workspace.
pub(super) fn read_all(store: &MarkdownStore) -> Result<Vec<ArtifactFile>> {
    let mut out = Vec::new();
    for relative in store
        .list_markdown(Path::new(ARTIFACTS_DIR))
        .map_err(|error| AtlasError::Other(format!("list artifacts: {error}")))?
    {
        let record = store
            .read_record(&relative)
            .map_err(|error| AtlasError::Other(format!("read {}: {error}", relative.display())))?;
        let frontmatter = record.frontmatter;
        let source_id = frontmatter.get_str("id").unwrap_or_default().to_owned();
        if source_id.is_empty() {
            continue;
        }
        let strings = |field: &str| -> Vec<String> {
            frontmatter
                .get_list(field)
                .unwrap_or_default()
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        };
        out.push(ArtifactFile {
            source_id,
            label: frontmatter.get_str("label").unwrap_or_default().to_owned(),
            source_type: frontmatter
                .get_str("source_type")
                .unwrap_or("mcp_artifact")
                .to_owned(),
            session_id: frontmatter.get_str("session_id").map(str::to_owned),
            agent_id: frontmatter.get_str("agent_id").map(str::to_owned),
            content_type: frontmatter
                .get_str("content_type")
                .unwrap_or("text/plain")
                .to_owned(),
            identity_kind: frontmatter
                .get_str("identity_kind")
                .unwrap_or("source_id")
                .to_owned(),
            identity_value: frontmatter
                .get_str("identity_value")
                .unwrap_or_default()
                .to_owned(),
            repo_roots: strings("repo_roots"),
            repo_ids: strings("repo_ids"),
            body: record.body,
        });
    }
    Ok(out)
}

impl ArtifactFile {
    /// Rebuild indexing metadata from the truth file.
    pub(super) fn to_meta(&self) -> SourceMeta {
        SourceMeta {
            id: self.source_id.clone(),
            session_id: self.session_id.clone(),
            agent_id: self.agent_id.clone(),
            source_type: self.source_type.clone(),
            label: self.label.clone(),
            repo_root: self.repo_roots.first().cloned(),
            repo_roots: self.repo_roots.clone(),
            repo_id: self.repo_ids.first().cloned(),
            repo_ids: self.repo_ids.clone(),
            identity_kind: self.identity_kind.clone(),
            identity_value: self.identity_value.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ContentStore, SourceMeta};
    use tempfile::TempDir;

    fn meta(id: &str, label: &str) -> SourceMeta {
        SourceMeta {
            id: id.to_owned(),
            session_id: Some("s1".to_owned()),
            agent_id: None,
            source_type: "mcp_artifact".to_owned(),
            label: label.to_owned(),
            repo_root: Some("/repo".to_owned()),
            repo_roots: vec!["/repo".to_owned()],
            repo_id: None,
            repo_ids: Vec::new(),
            identity_kind: "source_id".to_owned(),
            identity_value: id.to_owned(),
        }
    }

    fn open_store(dir: &TempDir) -> ContentStore {
        std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();
        let path = dir.path().join(".atlas").join("context.db");
        ContentStore::open(path.to_str().unwrap()).unwrap()
    }

    #[test]
    fn index_artifact_writes_markdown_and_delete_removes_it() {
        let dir = TempDir::new().unwrap();
        let mut store = open_store(&dir);
        store
            .index_artifact(
                meta("src-1", "label: with colon"),
                "hello world",
                "text/plain",
            )
            .unwrap();

        let path = store.markdown.absolute(&artifact_path("src-1"));
        assert!(path.exists());
        let record = store.markdown.read_record(&artifact_path("src-1")).unwrap();
        assert_eq!(
            record.frontmatter.get_str("label"),
            Some("label: with colon")
        );
        assert_eq!(record.body, "hello world");

        store.delete_source("src-1").unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn rebuild_reproduces_chunk_boundaries_and_search() {
        let dir = TempDir::new().unwrap();
        let mut store = open_store(&dir);
        let body = "alpha paragraph\n\nbeta paragraph with more words\n";
        store
            .index_artifact(meta("src-2", "rebuild"), body, "text/plain")
            .unwrap();
        let before = store.get_chunks("src-2").unwrap();
        let hits_before = store.search("beta", &Default::default()).unwrap();

        store
            .conn
            .execute_batch(
                "DELETE FROM chunks_fts; DELETE FROM chunks_trigram; DELETE FROM chunks; DELETE FROM sources;",
            )
            .unwrap();
        assert!(store.get_chunks("src-2").unwrap().is_empty());

        let indexed = store.rebuild_from_markdown(true).unwrap();
        assert_eq!(indexed, 1);
        let after = store.get_chunks("src-2").unwrap();
        assert_eq!(
            before
                .iter()
                .map(|chunk| chunk.content.as_str())
                .collect::<Vec<_>>(),
            after
                .iter()
                .map(|chunk| chunk.content.as_str())
                .collect::<Vec<_>>()
        );
        let hits_after = store.search("beta", &Default::default()).unwrap();
        assert_eq!(hits_before.len(), hits_after.len());
    }

    #[test]
    fn rebuild_prunes_sources_whose_markdown_file_was_deleted() {
        let dir = TempDir::new().unwrap();
        let mut store = open_store(&dir);
        store
            .index_artifact(meta("src-prune", "prune"), "content to find", "text/plain")
            .unwrap();
        assert!(store.get_source("src-prune").unwrap().is_some());

        std::fs::remove_file(store.markdown.absolute(&artifact_path("src-prune"))).unwrap();
        store.rebuild_from_markdown(false).unwrap();

        assert!(
            store.get_source("src-prune").unwrap().is_none(),
            "incremental rebuild must prune rows without truth files"
        );
        assert!(
            store
                .search("content", &Default::default())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn artifact_body_round_trips_crlf_and_bom() {
        let dir = TempDir::new().unwrap();
        let mut store = open_store(&dir);
        let body = "\u{feff}line one\r\nline two\r\nno trailing newline";
        store
            .index_artifact(meta("src-3", "edge"), body, "text/plain")
            .unwrap();
        let record = store.markdown.read_record(&artifact_path("src-3")).unwrap();
        assert_eq!(record.body, body);
    }
}
