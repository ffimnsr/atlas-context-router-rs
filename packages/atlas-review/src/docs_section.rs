use atlas_core::kinds::NodeKind;
use atlas_core::{AtlasError, Result, model::Node};
use atlas_repo::CanonicalRepoPath;
use atlas_store_sqlite::Store;
use camino::Utf8Path;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DocsSectionSelector {
    Heading(String),
    Line(u32),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DocsSectionLine {
    pub line: u32,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DocsSectionCandidate {
    pub title: String,
    pub heading_path: String,
    pub heading_slug: String,
    pub heading_level: u32,
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DocsSectionLookup {
    pub file: String,
    pub selector_kind: String,
    pub resolved: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading_slug: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading_level: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<DocsSectionLine>,
    pub truncated: bool,
    pub omitted_byte_count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub candidates: Vec<DocsSectionCandidate>,
    pub atlas_result_kind: &'static str,
}

#[derive(Clone, Debug)]
struct IndexedHeading {
    title: String,
    path: String,
    slug: String,
    level: u32,
    start_line: u32,
    end_line: u32,
    file_hash: Option<String>,
}

pub fn lookup_docs_section(
    store: &Store,
    repo_root: &Utf8Path,
    file: &str,
    selector: DocsSectionSelector,
    max_bytes: usize,
) -> Result<DocsSectionLookup> {
    let canonical = CanonicalRepoPath::from_repo_relative(file)
        .map_err(|error| AtlasError::Other(format!("invalid file path '{file}': {error}")))?;
    let file = canonical.as_str().to_owned();
    let abs_path = repo_root.join(&file);
    if !abs_path.is_file() {
        return Err(AtlasError::Other(format!("file not found: {file}")));
    }
    let contents = std::fs::read_to_string(&abs_path).map_err(|error| {
        AtlasError::Other(format!("cannot read UTF-8 text file '{file}': {error}"))
    })?;
    let file_lines: Vec<&str> = contents.lines().collect();

    let nodes = store.nodes_by_file(&file)?;
    if nodes.is_empty() {
        return Err(AtlasError::Other(format!("document not indexed: {file}")));
    }

    let headings = indexed_headings(&nodes, file_lines.len() as u32);
    if headings.is_empty() {
        return Err(AtlasError::Other(format!(
            "no indexed markdown headings found in {file}"
        )));
    }

    let selector_kind = match &selector {
        DocsSectionSelector::Heading(_) => "heading",
        DocsSectionSelector::Line(_) => "line",
    }
    .to_owned();

    let selected = match selector {
        DocsSectionSelector::Heading(query) => {
            let matches = headings
                .iter()
                .filter(|heading| heading_matches(heading, &query))
                .cloned()
                .collect::<Vec<_>>();
            if matches.is_empty() {
                return Err(AtlasError::Other(format!(
                    "heading not found in {file}: {query}"
                )));
            }
            if matches.len() > 1 {
                return Ok(DocsSectionLookup {
                    file,
                    selector_kind,
                    resolved: false,
                    query: Some(query),
                    title: None,
                    heading_path: None,
                    heading_slug: None,
                    heading_level: None,
                    start_line: None,
                    end_line: None,
                    line_count: None,
                    file_hash: None,
                    content: None,
                    lines: Vec::new(),
                    truncated: false,
                    omitted_byte_count: 0,
                    candidates: matches.into_iter().map(candidate_from_heading).collect(),
                    atlas_result_kind: "docs_section",
                });
            }
            matches.into_iter().next().expect("single heading match")
        }
        DocsSectionSelector::Line(line) => {
            if line == 0 {
                return Err(AtlasError::Other(
                    "line numbers are 1-based; got 0".to_owned(),
                ));
            }
            headings
                .iter()
                .filter(|heading| heading.start_line <= line && line <= heading.end_line)
                .max_by_key(|heading| heading.level)
                .cloned()
                .ok_or_else(|| {
                    AtlasError::Other(format!("no section covers line {line} in {file}"))
                })?
        }
    };
    let excerpt = bounded_excerpt(
        &file_lines,
        selected.start_line,
        selected.end_line,
        max_bytes,
    );

    Ok(DocsSectionLookup {
        file,
        selector_kind,
        resolved: true,
        query: None,
        title: Some(selected.title),
        heading_path: Some(selected.path),
        heading_slug: Some(selected.slug),
        heading_level: Some(selected.level),
        start_line: Some(selected.start_line),
        end_line: Some(selected.end_line),
        line_count: Some(excerpt.lines.len()),
        file_hash: selected.file_hash,
        content: Some(excerpt.content),
        lines: excerpt.lines,
        truncated: excerpt.truncated,
        omitted_byte_count: excerpt.omitted_byte_count,
        candidates: Vec::new(),
        atlas_result_kind: "docs_section",
    })
}

fn indexed_headings(nodes: &[Node], total_lines: u32) -> Vec<IndexedHeading> {
    let mut headings = nodes
        .iter()
        .filter_map(indexed_heading_from_node)
        .collect::<Vec<_>>();
    headings.sort_by_key(|heading| (heading.start_line, heading.path.clone()));

    for idx in 0..headings.len() {
        let level = headings[idx].level;
        let next_start = headings
            .iter()
            .skip(idx + 1)
            .find(|candidate| candidate.level <= level)
            .map(|candidate| candidate.start_line);
        headings[idx].end_line = next_start
            .map(|start| start.saturating_sub(1).max(headings[idx].start_line))
            .unwrap_or(total_lines.max(headings[idx].start_line));
    }

    headings
}

fn indexed_heading_from_node(node: &Node) -> Option<IndexedHeading> {
    if node.kind != NodeKind::Module || node.language != "markdown" {
        return None;
    }
    let level = node.extra_json.get("level")?.as_u64()? as u32;
    let path = node.extra_json.get("path")?.as_str()?.to_owned();
    let slug = path.rsplit('.').next().unwrap_or(&path).to_owned();
    Some(IndexedHeading {
        title: node.name.clone(),
        path,
        slug,
        level,
        start_line: node.line_start,
        end_line: node.line_end.max(node.line_start),
        file_hash: (!node.file_hash.is_empty()).then_some(node.file_hash.clone()),
    })
}

fn heading_matches(heading: &IndexedHeading, query: &str) -> bool {
    let normalized = query.trim().trim_matches('.');
    !normalized.is_empty()
        && (heading.path == normalized
            || heading.path == format!("document.{normalized}")
            || heading.slug == normalized
            || heading.title.eq_ignore_ascii_case(normalized))
}

fn candidate_from_heading(heading: IndexedHeading) -> DocsSectionCandidate {
    DocsSectionCandidate {
        title: heading.title,
        heading_path: heading.path,
        heading_slug: heading.slug,
        heading_level: heading.level,
        start_line: heading.start_line,
        end_line: heading.end_line,
    }
}

struct BoundedExcerpt {
    content: String,
    lines: Vec<DocsSectionLine>,
    truncated: bool,
    omitted_byte_count: usize,
}

fn bounded_excerpt(
    lines: &[&str],
    start_line: u32,
    end_line: u32,
    max_bytes: usize,
) -> BoundedExcerpt {
    let mut out_lines = Vec::new();
    let mut content = String::new();
    let mut used_bytes = 0usize;
    let mut truncated = false;

    for line_number in start_line..=end_line {
        let idx = (line_number as usize).saturating_sub(1);
        if idx >= lines.len() {
            break;
        }

        let line_text = lines[idx].trim_end_matches('\r');
        let prefix_bytes = usize::from(!content.is_empty());
        let full_bytes = prefix_bytes + line_text.len();
        if used_bytes + full_bytes <= max_bytes {
            if !content.is_empty() {
                content.push('\n');
                used_bytes += 1;
            }
            content.push_str(line_text);
            used_bytes += line_text.len();
            out_lines.push(DocsSectionLine {
                line: line_number,
                text: line_text.to_owned(),
            });
            continue;
        }

        let remaining = max_bytes.saturating_sub(used_bytes + prefix_bytes);
        if remaining > 0 {
            let partial = truncate_to_boundary(line_text, remaining);
            if !content.is_empty() {
                content.push('\n');
            }
            content.push_str(&partial);
            out_lines.push(DocsSectionLine {
                line: line_number,
                text: partial,
            });
        }
        truncated = true;
        break;
    }

    let full_content = (start_line..=end_line)
        .filter_map(|line_number| lines.get((line_number as usize).saturating_sub(1)).copied())
        .map(|line| line.trim_end_matches('\r'))
        .collect::<Vec<_>>()
        .join("\n");

    BoundedExcerpt {
        omitted_byte_count: full_content.len().saturating_sub(content.len()),
        content,
        lines: out_lines,
        truncated,
    }
}

fn truncate_to_boundary(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_owned();
    }
    let mut end = 0usize;
    for (index, _) in text.char_indices() {
        if index > max_bytes {
            break;
        }
        end = index;
    }
    if end == 0 && max_bytes > 0 && text.is_char_boundary(max_bytes) {
        end = max_bytes;
    }
    text[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::NodeId;
    use serde_json::json;
    use tempfile::tempdir;

    fn markdown_node(title: &str, path: &str, level: u32, file: &str, start: u32) -> Node {
        Node {
            id: NodeId::UNSET,
            kind: NodeKind::Module,
            name: title.to_owned(),
            qualified_name: format!("{file}::heading::{path}"),
            file_path: file.to_owned(),
            line_start: start,
            line_end: start,
            language: "markdown".to_owned(),
            parent_name: None,
            params: None,
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: "h1".to_owned(),
            extra_json: json!({ "level": level, "path": path }),
        }
    }

    fn setup_doc_repo(contents: &str, nodes: &[Node]) -> (tempfile::TempDir, Store) {
        let dir = tempdir().expect("tempdir");
        std::fs::write(dir.path().join("README.md"), contents).expect("write README");
        let db_path = dir.path().join("atlas.db");
        let mut store = Store::open(&db_path.to_string_lossy()).expect("open store");
        store
            .replace_file_graph("README.md", "h1", Some("markdown"), Some(8), nodes, &[])
            .expect("replace graph");
        (dir, store)
    }

    #[test]
    fn resolves_nested_heading_from_indexed_nodes() {
        let contents = "# Overview\nintro\n## Install\nstep\n## Usage\nrun\n";
        let nodes = vec![
            markdown_node("Overview", "document.overview", 1, "README.md", 1),
            markdown_node("Install", "document.overview.install", 2, "README.md", 3),
            markdown_node("Usage", "document.overview.usage", 2, "README.md", 5),
        ];
        let (dir, store) = setup_doc_repo(contents, &nodes);
        let result = lookup_docs_section(
            &store,
            Utf8Path::from_path(dir.path()).expect("utf8 repo root"),
            "README.md",
            DocsSectionSelector::Heading("document.overview.install".to_owned()),
            16_384,
        )
        .expect("lookup docs section");

        assert!(result.resolved);
        assert_eq!(
            result.heading_path.as_deref(),
            Some("document.overview.install")
        );
        assert_eq!(result.start_line, Some(3));
        assert_eq!(result.end_line, Some(4));
    }

    #[test]
    fn returns_candidates_for_ambiguous_slug() {
        let contents = "# One\n## Install\na\n# Two\n## Install\nb\n";
        let nodes = vec![
            markdown_node("One", "document.one", 1, "README.md", 1),
            markdown_node("Install", "document.one.install", 2, "README.md", 2),
            markdown_node("Two", "document.two", 1, "README.md", 4),
            markdown_node("Install", "document.two.install", 2, "README.md", 5),
        ];
        let (dir, store) = setup_doc_repo(contents, &nodes);
        let result = lookup_docs_section(
            &store,
            Utf8Path::from_path(dir.path()).expect("utf8 repo root"),
            "README.md",
            DocsSectionSelector::Heading("install".to_owned()),
            16_384,
        )
        .expect("lookup docs section");

        assert!(!result.resolved);
        assert_eq!(result.candidates.len(), 2);
        assert_eq!(result.candidates[0].heading_path, "document.one.install");
        assert_eq!(result.candidates[1].heading_path, "document.two.install");
    }

    #[test]
    fn truncates_section_bytes_deterministically() {
        let contents = "# Overview\nalpha beta gamma\n";
        let nodes = vec![markdown_node(
            "Overview",
            "document.overview",
            1,
            "README.md",
            1,
        )];
        let (dir, store) = setup_doc_repo(contents, &nodes);
        let result = lookup_docs_section(
            &store,
            Utf8Path::from_path(dir.path()).expect("utf8 repo root"),
            "README.md",
            DocsSectionSelector::Heading("overview".to_owned()),
            12,
        )
        .expect("lookup docs section");

        assert!(result.resolved);
        assert!(result.truncated);
        assert!(result.omitted_byte_count > 0);
    }
}
