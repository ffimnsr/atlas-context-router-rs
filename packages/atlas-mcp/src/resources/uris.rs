//! Canonical `atlas://` URI namespace plus the four scoped access primitives
//! (`Read`, `List`, `Grep`, `Search`).
//!
//! Every form is prefix-preserving and canonical. Path segments are
//! percent-encoded, `..`/absolute/empty segments are rejected, and repo paths
//! are canonicalized through [`CanonicalRepoPath`] before lookup.

use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use atlas_repo::{CanonicalRepoPath, RepoRegistry, stable_repo_id};
use atlas_session::{
    OVERVIEW_L0_MAX_CHARS, OVERVIEW_L1_MAX_CHARS, OverviewLevel, OverviewListFilter, SessionStore,
    render_level,
};
use camino::Utf8Path;

/// Parsed `atlas://` URI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AtlasUri {
    Tree {
        repo_id: String,
        dir: String,
    },
    File {
        repo_id: String,
        path: String,
    },
    Symbol {
        repo_id: String,
        qualified_name: String,
    },
    MemoryTopic {
        topic: String,
    },
    MemoryId {
        memory_id: String,
    },
    Memoir {
        graph: String,
        concept: String,
    },
    Artifact {
        source_id: String,
        chunk_index: usize,
    },
}

impl AtlasUri {
    pub(crate) fn default_level(&self) -> OverviewLevel {
        match self {
            Self::Artifact { .. } | Self::MemoryId { .. } => OverviewLevel::L2,
            _ => OverviewLevel::L1,
        }
    }

    fn supports_level_query(&self) -> bool {
        matches!(
            self,
            Self::Tree { .. } | Self::File { .. } | Self::Symbol { .. }
        )
    }

    pub(crate) fn format(&self) -> String {
        match self {
            Self::Tree { repo_id, dir } => {
                let dir = if dir.is_empty() { "." } else { dir.as_str() };
                format!(
                    "atlas://repo/{}/tree/{}",
                    encode_segment(repo_id),
                    encode_segment(dir)
                )
            }
            Self::File { repo_id, path } => format!(
                "atlas://repo/{}/file/{}",
                encode_segment(repo_id),
                encode_segment(path)
            ),
            Self::Symbol {
                repo_id,
                qualified_name,
            } => format!(
                "atlas://repo/{}/symbol/{}",
                encode_segment(repo_id),
                encode_segment(qualified_name)
            ),
            Self::MemoryTopic { topic } => {
                format!("atlas://memories/topic/{}", encode_segment(topic))
            }
            Self::MemoryId { memory_id } => {
                format!("atlas://memories/id/{}", encode_segment(memory_id))
            }
            Self::Memoir { graph, concept } => format!(
                "atlas://memoir/{}/{}",
                encode_segment(graph),
                encode_segment(concept)
            ),
            Self::Artifact {
                source_id,
                chunk_index,
            } => format!(
                "atlas://artifacts/{}/{}",
                encode_segment(source_id),
                chunk_index
            ),
        }
    }
}

/// Percent-encode one path segment, preserving RFC 3986 unreserved characters
/// plus `:` so qualified names stay readable.
pub(crate) fn encode_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b':' => {
                out.push(byte as char);
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Decode one percent-encoded segment and reject traversal or separators that
/// would escape the segment boundary after decoding.
pub(crate) fn decode_segment(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                bail!("invalid percent-encoding in uri segment '{value}'");
            }
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3])
                .map_err(|_| anyhow!("invalid percent-encoding in uri segment '{value}'"))?;
            let decoded = u8::from_str_radix(hex, 16)
                .map_err(|_| anyhow!("invalid percent-encoding in uri segment '{value}'"))?;
            out.push(decoded);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    let decoded =
        String::from_utf8(out).map_err(|_| anyhow!("uri segment is not valid UTF-8: '{value}'"))?;
    Ok(decoded)
}

/// Reject traversal, absolute paths, and empty path components after decoding.
fn validate_segment(decoded: &str) -> Result<()> {
    if decoded.is_empty() {
        bail!("uri must not contain empty path segments");
    }
    if decoded.contains('\\') {
        bail!("uri segment must not contain backslashes: '{decoded}'");
    }
    if decoded.starts_with('/') {
        bail!("uri must not contain absolute paths: '{decoded}'");
    }
    if decoded.chars().any(char::is_control) {
        bail!("uri segment must not contain control characters");
    }
    if decoded != "."
        && decoded
            .split('/')
            .any(|component| component.is_empty() || component == ".." || component == ".")
    {
        bail!("uri path traversal is not allowed: '{decoded}'");
    }
    Ok(())
}

/// Parse and validate one `atlas://` URI into its typed form plus optional level.
pub(crate) fn parse_uri(uri: &str) -> Result<(AtlasUri, OverviewLevel)> {
    let rest = uri
        .strip_prefix("atlas://")
        .ok_or_else(|| anyhow!("uri must start with atlas://: '{uri}'"))?;
    let (path, query) = match rest.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (rest, None),
    };

    let raw_segments: Vec<&str> = path.split('/').collect();
    if raw_segments.is_empty() || raw_segments.iter().any(|segment| segment.is_empty()) {
        bail!("uri must not contain empty path segments: '{uri}'");
    }

    let mut segments = Vec::with_capacity(raw_segments.len());
    for raw in &raw_segments {
        let decoded = decode_segment(raw)?;
        validate_segment(&decoded)?;
        segments.push(decoded);
    }

    let parsed = match segments.as_slice() {
        [repo, repo_id, kind, tail @ ..] if repo == "repo" => {
            if repo_id.trim().is_empty() {
                bail!("repo uri must include a repo_id");
            }
            match (kind.as_str(), tail) {
                ("tree", tail) if !tail.is_empty() => AtlasUri::Tree {
                    repo_id: repo_id.clone(),
                    dir: if tail.len() == 1 && tail[0] == "." {
                        String::new()
                    } else {
                        tail.join("/")
                    },
                },
                ("file", tail) if !tail.is_empty() => AtlasUri::File {
                    repo_id: repo_id.clone(),
                    path: tail.join("/"),
                },
                ("symbol", [qname]) => AtlasUri::Symbol {
                    repo_id: repo_id.clone(),
                    qualified_name: qname.clone(),
                },
                _ => bail!("unknown repo uri form: '{uri}'"),
            }
        }
        [memories, kind, tail @ ..] if memories == "memories" => match (kind.as_str(), tail) {
            ("topic", [topic]) => AtlasUri::MemoryTopic {
                topic: topic.clone(),
            },
            ("id", [memory_id]) => AtlasUri::MemoryId {
                memory_id: memory_id.clone(),
            },
            _ => bail!("unknown memories uri form: '{uri}'"),
        },
        [memoir, graph, concept] if memoir == "memoir" => AtlasUri::Memoir {
            graph: graph.clone(),
            concept: concept.clone(),
        },
        [artifacts, source_id, chunk_index] if artifacts == "artifacts" => AtlasUri::Artifact {
            source_id: source_id.clone(),
            chunk_index: chunk_index
                .parse::<usize>()
                .map_err(|_| anyhow!("artifact uri chunk index must be a non-negative integer"))?,
        },
        _ => bail!("unknown atlas uri: '{uri}'"),
    };

    let level = match query {
        None => parsed.default_level(),
        Some(query) => {
            let mut level: Option<OverviewLevel> = None;
            for pair in query.split('&') {
                if pair.is_empty() {
                    continue;
                }
                let (key, value) = pair
                    .split_once('=')
                    .ok_or_else(|| anyhow!("invalid uri query parameter: '{pair}'"))?;
                if key != "level" {
                    bail!("unsupported uri query parameter: '{key}'");
                }
                level = Some(OverviewLevel::from_str(value)?);
            }
            let level = level.ok_or_else(|| anyhow!("empty uri query"))?;
            if !parsed.supports_level_query() {
                bail!("level is not supported for this uri: '{uri}'");
            }
            level
        }
    };

    Ok((parsed, level))
}

/// Resolve the filesystem root that owns `repo_id`.
fn resolve_repo_root(current_repo_root: &str, repo_id: &str) -> Result<String> {
    let current = Utf8Path::new(current_repo_root);
    if stable_repo_id(current) == repo_id {
        return Ok(current_repo_root.to_owned());
    }
    if let Ok(registry) = RepoRegistry::load(current)
        && let Some(registration) = registry
            .registrations
            .iter()
            .find(|registration| registration.repo_id == repo_id)
    {
        return Ok(registration.root.to_string());
    }
    bail!("unknown repo id: {repo_id}")
}

fn canonical_relative(repo_root: &str, path: &str) -> Result<String> {
    let canonical =
        CanonicalRepoPath::from_cli_argument(Utf8Path::new(repo_root), Utf8Path::new(path))
            .map_err(|error| anyhow!("path outside canonical repo scope: {path}: {error}"))?;
    Ok(canonical.as_str().to_owned())
}

fn open_session(db_path: &str) -> Result<SessionStore> {
    let session_db = atlas_adapters::derive_session_db_path(db_path);
    SessionStore::open(&session_db).context("cannot open session database")
}

/// Read one URI at the requested level.
pub fn read_uri(
    uri: &str,
    level: Option<&str>,
    repo_root: &str,
    db_path: &str,
) -> Result<(String, String)> {
    let (parsed, default_level) = parse_uri(uri)?;
    let level = match level {
        Some(raw) => OverviewLevel::from_str(raw)?,
        None => default_level,
    };

    match &parsed {
        AtlasUri::Tree { repo_id, dir } => {
            let root = resolve_repo_root(repo_root, repo_id)?;
            tree_view(&root, dir, level, db_path)
        }
        AtlasUri::File { repo_id, path } => {
            let root = resolve_repo_root(repo_root, repo_id)?;
            let relative = canonical_relative(&root, path)?;
            file_view(&root, &relative, level, db_path, uri)
        }
        AtlasUri::Symbol {
            repo_id,
            qualified_name,
        } => {
            let root = resolve_repo_root(repo_root, repo_id)?;
            symbol_view(&root, qualified_name, level, db_path, uri)
        }
        AtlasUri::MemoryTopic { topic } => {
            let response = crate::tools::call(
                "memory_recall",
                Some(&json!({
                    "query": topic,
                    "topic": topic,
                    "limit": 20,
                })),
                repo_root,
                db_path,
            )?;
            let mut value = structured(response)?;
            value["uri"] = json!(uri);
            value["level"] = json!(level.as_str());
            Ok((
                "application/json".to_owned(),
                serde_json::to_string_pretty(&limit_value(value, level))?,
            ))
        }
        AtlasUri::MemoryId { memory_id } => {
            let session = open_session(db_path)?;
            let memory = session
                .get_memory(repo_root, memory_id)?
                .ok_or_else(|| anyhow!("unknown memory id: {memory_id}"))?;
            let value = json!({
                "uri": uri,
                "level": level.as_str(),
                "memory": memory,
            });
            Ok((
                "application/json".to_owned(),
                serde_json::to_string_pretty(&limit_value(value, level))?,
            ))
        }
        AtlasUri::Memoir { graph, concept } => {
            let response = crate::tools::call(
                "memoir_inspect",
                Some(&json!({
                    "graph": graph,
                    "concept": concept,
                    "depth": 1,
                })),
                repo_root,
                db_path,
            )?;
            let mut value = structured(response)?;
            value["uri"] = json!(uri);
            value["level"] = json!(level.as_str());
            Ok((
                "application/json".to_owned(),
                serde_json::to_string_pretty(&limit_value(value, level))?,
            ))
        }
        AtlasUri::Artifact {
            source_id,
            chunk_index,
        } => {
            let max_bytes = match level {
                OverviewLevel::L0 => 512,
                OverviewLevel::L1 => 4096,
                OverviewLevel::L2 => 65536,
            };
            let response = crate::tools::call(
                "read_saved_context",
                Some(&json!({
                    "source_id": source_id,
                    "chunk_offset": chunk_index,
                    "max_bytes": max_bytes,
                })),
                repo_root,
                db_path,
            )?;
            let raw = structured(response)?;
            if raw.get("found").and_then(Value::as_bool) == Some(false) {
                bail!("unknown artifact source_id: {source_id}");
            }
            let text = raw
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            Ok(("text/plain".to_owned(), text))
        }
    }
}

fn tree_view(
    repo_root: &str,
    dir: &str,
    level: OverviewLevel,
    db_path: &str,
) -> Result<(String, String)> {
    let relative = if dir.is_empty() {
        String::new()
    } else {
        canonical_relative(repo_root, dir)?
    };
    let absolute = if relative.is_empty() {
        std::path::PathBuf::from(repo_root)
    } else {
        std::path::Path::new(repo_root).join(&relative)
    };
    if !absolute.is_dir() {
        bail!("tree uri target is not a directory: {relative}");
    }

    let abstracts = overview_l0_abstracts(repo_root, &relative, db_path).unwrap_or_default();
    let mut directories = Vec::new();
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&absolute)
        .with_context(|| format!("cannot read directory {}", absolute.display()))?
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().to_string();
        if hidden_entry(&name) {
            continue;
        }
        let child = if relative.is_empty() {
            name.clone()
        } else {
            format!("{relative}/{name}")
        };
        if entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            directories.push(child);
        } else {
            files.push(child);
        }
    }
    directories.sort();
    files.sort();

    let files: Vec<Value> = files
        .into_iter()
        .map(|path| {
            let abstract_text = abstracts.get(&path).cloned();
            json!({
                "path": path,
                "l0": abstract_text,
            })
        })
        .collect();

    let value = json!({
        "uri": AtlasUri::Tree { repo_id: stable_repo_id(Utf8Path::new(repo_root)), dir: relative }.format(),
        "level": level.as_str(),
        "directories": directories,
        "files": files,
        "recursive": false,
    });
    let value = if level == OverviewLevel::L0 {
        json!({
            "uri": value["uri"],
            "level": value["level"],
            "directories": value["directories"],
            "files": value["files"],
            "recursive": false,
        })
    } else {
        value
    };
    Ok((
        "application/json".to_owned(),
        serde_json::to_string_pretty(&value)?,
    ))
}

fn overview_l0_abstracts(
    repo_root: &str,
    relative_dir: &str,
    db_path: &str,
) -> Result<std::collections::HashMap<String, String>> {
    let session = open_session(db_path)?;
    let records = session.list_overview_records(
        repo_root,
        &OverviewListFilter {
            limit: Some(500),
            ..Default::default()
        },
    )?;
    let mut abstracts = std::collections::HashMap::new();
    for record in records {
        let Some(path) = record.canonical_file_path.as_deref() else {
            continue;
        };
        let parent = path
            .rsplit_once('/')
            .map(|(parent, _)| parent)
            .unwrap_or("");
        if parent == relative_dir {
            let view = render_level(&record, OverviewLevel::L0);
            abstracts.insert(path.to_owned(), view.summary);
        }
    }
    Ok(abstracts)
}

fn file_view(
    repo_root: &str,
    relative: &str,
    level: OverviewLevel,
    db_path: &str,
    uri: &str,
) -> Result<(String, String)> {
    let session = open_session(db_path)?;
    let value = match session.resolve_overview_subject(repo_root, relative) {
        Ok(record) => serde_json::to_value(render_level(&record, level))?,
        Err(_) => {
            let path = std::path::Path::new(repo_root).join(relative);
            let meta = std::fs::metadata(&path)
                .with_context(|| format!("file uri target does not exist: {relative}"))?;
            if !meta.is_file() {
                bail!("file uri target is not a file: {relative}");
            }
            let summary = file_summary(&path);
            let mut value = json!({
                "level": level.as_str(),
                "freshness": "unverified",
                "title": relative.rsplit('/').next().unwrap_or(relative),
                "summary": summary,
                "canonical_file_path": relative,
            });
            if level >= OverviewLevel::L1 {
                value["bytes"] = json!(meta.len());
            }
            if level == OverviewLevel::L2 {
                value["read_hint"] = json!(format!("read_file_excerpt {{ file: \"{relative}\" }}"));
            }
            value
        }
    };
    let mut value = value;
    value["uri"] = json!(uri);
    Ok((
        "application/json".to_owned(),
        serde_json::to_string_pretty(&value)?,
    ))
}

fn file_summary(path: &std::path::Path) -> String {
    if let Ok(contents) = std::fs::read_to_string(path) {
        for line in contents.lines().take(40) {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let candidate = trimmed
                .trim_start_matches("//")
                .trim_start_matches('#')
                .trim_start_matches("/*")
                .trim_start_matches('*')
                .trim();
            if !candidate.is_empty() {
                return candidate.to_owned();
            }
        }
    }
    String::new()
}

fn symbol_view(
    repo_root: &str,
    qualified_name: &str,
    level: OverviewLevel,
    db_path: &str,
    uri: &str,
) -> Result<(String, String)> {
    let session = open_session(db_path)?;
    let value = match session.resolve_overview_subject(repo_root, qualified_name) {
        Ok(record) => serde_json::to_value(render_level(&record, level))?,
        Err(_) => {
            let response = crate::tools::call(
                "resolve_symbol",
                Some(&json!({ "name": qualified_name })),
                repo_root,
                db_path,
            )?;
            let resolved = structured(response)?;
            let node = resolved
                .get("match")
                .cloned()
                .or_else(|| resolved.get("symbol").cloned())
                .unwrap_or(resolved);
            json!({
                "level": level.as_str(),
                "freshness": "unverified",
                "title": node.get("name").and_then(Value::as_str).unwrap_or(qualified_name),
                "summary": node.get("kind").and_then(Value::as_str).unwrap_or_default(),
                "canonical_file_path": node.get("file_path").cloned(),
                "line_start": node.get("line_start").cloned(),
                "line_end": node.get("line_end").cloned(),
                "read_hint": if level == OverviewLevel::L2 {
                    Some(format!("read_file_excerpt {{ file: \"{}\" }}", node.get("file_path").and_then(Value::as_str).unwrap_or_default()))
                } else {
                    None
                },
            })
        }
    };
    let mut value = value;
    value["uri"] = json!(uri);
    Ok((
        "application/json".to_owned(),
        serde_json::to_string_pretty(&value)?,
    ))
}

/// `List` primitive: immediate children only, for tree URIs.
pub fn list_uri(uri: &str, repo_root: &str, db_path: &str) -> Result<(String, String)> {
    let (parsed, level) = parse_uri(uri)?;
    match parsed {
        AtlasUri::Tree { repo_id, dir } => {
            let root = resolve_repo_root(repo_root, &repo_id)?;
            tree_view(&root, &dir, level, db_path)
        }
        _ => bail!("list is only supported for repo tree uris"),
    }
}

/// `Grep` primitive: delegate to `search_content` scoped to the URI subtree.
pub fn grep_uri(
    uri: &str,
    pattern: &str,
    is_regex: bool,
    repo_root: &str,
    db_path: &str,
) -> Result<(String, String)> {
    let (parsed, _) = parse_uri(uri)?;
    let mut args = json!({
        "query": pattern,
        "is_regex": is_regex,
    });
    match parsed {
        AtlasUri::Tree { repo_id, dir } => {
            let root = resolve_repo_root(repo_root, &repo_id)?;
            if !dir.is_empty() {
                args["subpath"] = json!(canonical_relative(&root, &dir)?);
            }
        }
        AtlasUri::File { repo_id, path } => {
            let root = resolve_repo_root(repo_root, &repo_id)?;
            args["subpath"] = json!(canonical_relative(&root, &path)?);
        }
        AtlasUri::Symbol {
            repo_id,
            qualified_name,
        } => {
            let root = resolve_repo_root(repo_root, &repo_id)?;
            let response = crate::tools::call(
                "resolve_symbol",
                Some(&json!({ "name": qualified_name })),
                &root,
                db_path,
            )?;
            let resolved = structured(response)?;
            let file = resolved
                .pointer("/match/file_path")
                .and_then(Value::as_str)
                .or_else(|| {
                    resolved
                        .pointer("/symbol/file_path")
                        .and_then(Value::as_str)
                });
            if let Some(file) = file {
                args["subpath"] = json!(file);
            }
        }
        _ => bail!("grep is only supported for repo tree, file, and symbol uris"),
    }
    let response = crate::tools::call("search_content", Some(&args), repo_root, db_path)?;
    let value = structured(response)?;
    Ok((
        "application/json".to_owned(),
        serde_json::to_string_pretty(&value)?,
    ))
}

/// `Search` primitive: delegate to `query_graph`, `overview_search`, or
/// `memory_recall` with the URI subtree as scope.
pub fn search_uri(
    uri: &str,
    query: &str,
    repo_root: &str,
    db_path: &str,
) -> Result<(String, String)> {
    let (parsed, _) = parse_uri(uri)?;
    let response = match &parsed {
        AtlasUri::Tree { repo_id, dir } => {
            let root = resolve_repo_root(repo_root, repo_id)?;
            let mut args = json!({ "text": query });
            if !dir.is_empty() {
                args["subpath"] = json!(canonical_relative(&root, dir)?);
            }
            args["repo_id"] = json!(repo_id);
            crate::tools::call("query_graph", Some(&args), &root, db_path)?
        }
        AtlasUri::File { repo_id, path } => {
            let root = resolve_repo_root(repo_root, repo_id)?;
            let relative = canonical_relative(&root, path)?;
            let args = json!({ "text": query, "subpath": relative, "repo_id": repo_id });
            crate::tools::call("query_graph", Some(&args), &root, db_path)?
        }
        AtlasUri::Symbol {
            repo_id,
            qualified_name,
        } => {
            let root = resolve_repo_root(repo_root, repo_id)?;
            let args = json!({ "name": qualified_name, "repo_id": repo_id });
            crate::tools::call("resolve_symbol", Some(&args), &root, db_path)?
        }
        AtlasUri::MemoryTopic { topic } => {
            let args = json!({ "query": query, "topic": topic });
            crate::tools::call("memory_recall", Some(&args), repo_root, db_path)?
        }
        AtlasUri::MemoryId { memory_id } => {
            let args = json!({ "query": memory_id });
            crate::tools::call("memory_recall", Some(&args), repo_root, db_path)?
        }
        AtlasUri::Memoir { graph, concept } => {
            let args = json!({ "graph": graph, "concept": concept, "depth": 1 });
            crate::tools::call("memoir_inspect", Some(&args), repo_root, db_path)?
        }
        AtlasUri::Artifact { source_id, .. } => {
            let args = json!({ "query": query, "source_id": source_id });
            crate::tools::call("overview_search", Some(&args), repo_root, db_path)?
        }
    };
    let value = structured(response)?;
    Ok((
        "application/json".to_owned(),
        serde_json::to_string_pretty(&value)?,
    ))
}

fn structured(response: Value) -> Result<Value> {
    Ok(crate::tool_result::structured_content(&response)
        .cloned()
        .or_else(|| response.get("result").cloned())
        .unwrap_or(response))
}

/// Generic level projection for non-overview URI payloads.
///
/// L0 keeps identity/count fields only, L1 caps long strings at the L1 budget,
/// and L2 passes through untouched.
fn limit_value(value: Value, level: OverviewLevel) -> Value {
    match level {
        OverviewLevel::L2 => value,
        OverviewLevel::L1 => truncate_strings(value, OVERVIEW_L1_MAX_CHARS),
        OverviewLevel::L0 => {
            let truncated = truncate_strings(value.clone(), OVERVIEW_L0_MAX_CHARS);
            match truncated {
                Value::Object(map) => {
                    let kept: serde_json::Map<String, Value> = map
                        .into_iter()
                        .filter(|(key, _)| {
                            matches!(
                                key.as_str(),
                                "uri"
                                    | "level"
                                    | "topic"
                                    | "id"
                                    | "memory_id"
                                    | "shell"
                                    | "title"
                                    | "summary"
                                    | "freshness"
                                    | "graph"
                                    | "concept"
                                    | "name"
                                    | "kind"
                                    | "query"
                                    | "results"
                                    | "hits"
                                    | "memories"
                                    | "summary_text"
                            )
                        })
                        .collect();
                    Value::Object(kept)
                }
                other => other,
            }
        }
    }
}

fn truncate_strings(value: Value, max_chars: usize) -> Value {
    match value {
        Value::String(text) => {
            if text.chars().count() > max_chars {
                Value::String(text.chars().take(max_chars).collect())
            } else {
                Value::String(text)
            }
        }
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| truncate_strings(item, max_chars))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, item)| (key, truncate_strings(item, max_chars)))
                .collect(),
        ),
        other => other,
    }
}

fn hidden_entry(name: &str) -> bool {
    matches!(name, ".git" | "target" | "node_modules")
}
