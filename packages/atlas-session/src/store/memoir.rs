//! ICM-F — memoir concept graph storage layer.
//!
//! Memoir data lives in the continuity-owned session database next to shared
//! memory. It is intentionally a separate knowledge layer: memoir concepts and
//! relations never merge into the code graph tables (`nodes`/`edges`) and no
//! code graph query can observe these tables unless the memoir surface is
//! invoked explicitly.
//!
//! Relation types are restricted to a controlled vocabulary by
//! [`normalize_memoir_relation`]; aliases are normalized with explicit
//! direction or tagging rules. Missing concept endpoints are never created
//! unless the caller opts in via `create_missing`.

use std::collections::{BTreeSet, VecDeque};

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use atlas_core::{AtlasError, Result};

use super::memoir_schema::{MEMOIR_CONCEPT_COLUMNS, MEMOIR_GRAPH_COLUMNS, MEMOIR_RELATION_COLUMNS};
use super::memory::format_memory_now;
use super::types::MemoryScope;
use super::util::{hex_encode, to_from_sql_error};

/// Controlled relation vocabulary. Unknown relation names are rejected unless
/// a future config gate explicitly enables custom relations.
pub const MEMOIR_RELATION_TYPES: &[&str] = &[
    "depends_on",
    "part_of",
    "contradicts",
    "refines",
    "replaces",
    "caused_by",
    "fixed_by",
    "blocked_by",
    "decided_by",
    "related_to",
];

/// Hard ceilings for bounded inspect traversal.
const MAX_MEMOIR_INSPECT_DEPTH: u32 = 5;
const MAX_MEMOIR_INSPECT_NODES: usize = 200;
const MAX_MEMOIR_INSPECT_EDGES: usize = 500;

// ── Public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoirGraph {
    pub id: String,
    pub repo_root: String,
    pub scope: MemoryScope,
    pub name: String,
    pub description: String,
    pub created_at: String,
    pub updated_at: String,
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewMemoirGraph {
    pub repo_root: String,
    #[serde(default)]
    pub scope: MemoryScope,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_metadata")]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoirConcept {
    pub id: String,
    pub graph_id: String,
    pub repo_root: String,
    pub name: String,
    pub description: String,
    pub kind: String,
    pub source_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewMemoirConcept {
    pub repo_root: String,
    #[serde(default)]
    pub scope: MemoryScope,
    pub graph: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_concept_kind")]
    pub kind: String,
    pub source_id: Option<String>,
    #[serde(default = "default_metadata")]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoirRelation {
    pub id: String,
    pub graph_id: String,
    pub repo_root: String,
    pub source_concept_id: String,
    pub target_concept_id: String,
    pub relation_type: String,
    pub confidence: f64,
    pub source_id: Option<String>,
    pub created_at: String,
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewMemoirRelation {
    pub repo_root: String,
    #[serde(default)]
    pub scope: MemoryScope,
    pub graph: String,
    pub source: String,
    pub target: String,
    /// Requested relation type; aliases are normalized before persistence.
    pub relation: String,
    #[serde(default = "default_confidence")]
    pub confidence: f64,
    pub source_id: Option<String>,
    #[serde(default = "default_metadata")]
    pub metadata: Value,
    /// Explicit opt-in for creating missing concept endpoints.
    #[serde(default)]
    pub create_missing: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoirInspectQuery {
    pub repo_root: String,
    /// Optional graph name. When omitted, the concept name is resolved across
    /// graphs in the repo and ambiguous matches fail with candidate names.
    pub graph: Option<String>,
    pub concept: String,
    #[serde(default)]
    pub scope: Option<MemoryScope>,
    #[serde(default = "default_inspect_depth")]
    pub depth: u32,
    pub relation: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoirRelationDirection {
    Outgoing,
    Incoming,
}

impl MemoirRelationDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Outgoing => "outgoing",
            Self::Incoming => "incoming",
        }
    }
}

/// One traversed relation edge with canonical direction and source evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoirInspectionEdge {
    pub relation_id: String,
    pub relation_type: String,
    /// Direction relative to the node at `depth - 1` that led to this edge.
    pub direction: MemoirRelationDirection,
    pub depth: u32,
    pub from_concept_id: String,
    pub from_concept: String,
    pub to_concept_id: String,
    pub to_concept: String,
    pub confidence: f64,
    pub source_id: Option<String>,
    pub created_at: String,
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoirInspection {
    pub graph: MemoirGraph,
    pub root: MemoirConcept,
    pub depth: u32,
    pub relation: Option<String>,
    pub edges: Vec<MemoirInspectionEdge>,
    pub truncated: bool,
}

/// Outcome of relation-type normalization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedMemoirRelation {
    pub relation_type: &'static str,
    /// When true the caller's source/target must be swapped before persistence
    /// so the canonical relation direction stays consistent.
    pub reversed: bool,
    /// Original alias retained in relation metadata, when one was used.
    pub alias: Option<&'static str>,
}

fn default_metadata() -> Value {
    Value::Object(Default::default())
}

fn default_concept_kind() -> String {
    "concept".to_owned()
}

fn default_confidence() -> f64 {
    1.0
}

fn default_inspect_depth() -> u32 {
    1
}

// ── Validation and normalization ──────────────────────────────────────────────

/// Normalize a requested relation type against the controlled vocabulary.
///
/// Alias rules:
/// - `replaced_by` means the caller's *target* replaces the *source*, so the
///   endpoints are swapped and the canonical relation becomes `replaces`.
/// - `separate_from` maps to `related_to` without swapping and records the
///   alias in relation metadata.
pub fn normalize_memoir_relation(value: &str) -> Result<NormalizedMemoirRelation> {
    let cleaned = value.trim().to_ascii_lowercase().replace(['-', ' '], "_");
    let (relation_type, reversed, alias) = match cleaned.as_str() {
        "depends_on" => ("depends_on", false, None),
        "part_of" => ("part_of", false, None),
        "contradicts" => ("contradicts", false, None),
        "refines" => ("refines", false, None),
        "replaces" => ("replaces", false, None),
        "caused_by" => ("caused_by", false, None),
        "fixed_by" => ("fixed_by", false, None),
        "blocked_by" => ("blocked_by", false, None),
        "decided_by" => ("decided_by", false, None),
        "related_to" => ("related_to", false, None),
        "replaced_by" => ("replaces", true, Some("replaced_by")),
        "separate_from" | "separated_from" => ("related_to", false, Some("separate_from")),
        other => {
            return Err(AtlasError::Other(format!(
                "unknown memoir relation type: {other}; expected one of {}",
                MEMOIR_RELATION_TYPES.join(", ")
            )));
        }
    };
    Ok(NormalizedMemoirRelation {
        relation_type,
        reversed,
        alias,
    })
}

fn validate_confidence(confidence: f64) -> Result<()> {
    if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
        return Err(AtlasError::Other(
            "memoir relation confidence must be between 0.0 and 1.0".to_owned(),
        ));
    }
    Ok(())
}

fn require_name(kind: &str, value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(AtlasError::Other(format!(
            "memoir {kind} name must not be empty"
        )));
    }
    Ok(trimmed.to_owned())
}

fn canonical_name(value: &str) -> String {
    value.trim().to_lowercase()
}

// ── IDs ───────────────────────────────────────────────────────────────────────

fn derive_memoir_id(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update(part.as_bytes());
        hasher.update(b"\x00");
    }
    hex_encode(&hasher.finalize())
}

fn derive_memoir_graph_id(repo_root: &str, scope: MemoryScope, name: &str) -> String {
    derive_memoir_id(&[
        "memoir_graph",
        repo_root,
        scope.as_str(),
        &canonical_name(name),
    ])
}

fn derive_memoir_concept_id(graph_id: &str, name: &str) -> String {
    derive_memoir_id(&["memoir_concept", graph_id, &canonical_name(name)])
}

fn derive_memoir_relation_id(
    graph_id: &str,
    source_concept_id: &str,
    target_concept_id: &str,
    relation_type: &str,
) -> String {
    derive_memoir_id(&[
        "memoir_relation",
        graph_id,
        source_concept_id,
        target_concept_id,
        relation_type,
    ])
}

// ── Row mapping ───────────────────────────────────────────────────────────────

fn parse_scope(raw: String) -> rusqlite::Result<MemoryScope> {
    raw.parse::<MemoryScope>().map_err(to_from_sql_error)
}

fn parse_metadata(raw: String) -> Value {
    serde_json::from_str(&raw).unwrap_or_else(|_| default_metadata())
}

pub(super) fn row_to_memoir_graph(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoirGraph> {
    Ok(MemoirGraph {
        id: row.get(0)?,
        repo_root: row.get(1)?,
        scope: parse_scope(row.get(2)?)?,
        name: row.get(3)?,
        description: row.get(4)?,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        metadata: parse_metadata(row.get(7)?),
    })
}

fn row_to_memoir_concept(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoirConcept> {
    Ok(MemoirConcept {
        id: row.get(0)?,
        graph_id: row.get(1)?,
        repo_root: row.get(2)?,
        name: row.get(3)?,
        description: row.get(4)?,
        kind: row.get(5)?,
        source_id: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
        metadata: parse_metadata(row.get(9)?),
    })
}

pub(super) fn memoir_graph_select() -> &'static str {
    concat!(
        "SELECT id, repo_root, scope, name, description, created_at, updated_at, metadata_json ",
        "FROM memoir_graphs"
    )
}

fn memoir_concept_select() -> &'static str {
    concat!(
        "SELECT id, graph_id, repo_root, name, description, kind, source_id, created_at, ",
        "updated_at, metadata_json FROM memoir_concepts"
    )
}

/// Read-only concept listing ordered by name, used by the memory TUI.
fn list_memoir_concepts(
    conn: &Connection,
    repo_root: &str,
    limit: usize,
) -> Result<Vec<MemoirConcept>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let sql = format!(
        "{} WHERE repo_root = ?1 ORDER BY name ASC, id ASC LIMIT ?2",
        memoir_concept_select()
    );
    let mut statement = conn
        .prepare(&sql)
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    statement
        .query_map(params![repo_root, limit as i64], row_to_memoir_concept)
        .map_err(|e| AtlasError::Db(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| AtlasError::Db(e.to_string()))
}

fn is_unique_violation(error: &AtlasError) -> bool {
    error
        .to_string()
        .to_ascii_lowercase()
        .contains("unique constraint failed")
}

// ── Lookup helpers ────────────────────────────────────────────────────────────

fn load_memoir_graph(
    conn: &Connection,
    repo_root: &str,
    scope: MemoryScope,
    name: &str,
) -> Result<MemoirGraph> {
    let sql = format!(
        "{} WHERE repo_root = ?1 AND scope = ?2 AND name = ?3",
        memoir_graph_select()
    );
    conn.query_row(
        &sql,
        params![repo_root, scope.as_str(), name],
        row_to_memoir_graph,
    )
    .optional()
    .map_err(|e| AtlasError::Db(e.to_string()))?
    .ok_or_else(|| {
        AtlasError::Other(format!(
            "memoir graph '{name}' not found in scope '{}'",
            scope.as_str()
        ))
    })
}

fn load_memoir_concept(
    conn: &Connection,
    graph_id: &str,
    name: &str,
) -> Result<Option<MemoirConcept>> {
    let sql = format!(
        "{} WHERE graph_id = ?1 AND name = ?2",
        memoir_concept_select()
    );
    conn.query_row(&sql, params![graph_id, name], row_to_memoir_concept)
        .optional()
        .map_err(|e| AtlasError::Db(e.to_string()))
}

// ── Writes ────────────────────────────────────────────────────────────────────

pub(super) fn create_memoir_graph(
    conn: &Connection,
    input: &NewMemoirGraph,
) -> Result<MemoirGraph> {
    let name = require_name("graph", &input.name)?;
    if input.repo_root.trim().is_empty() {
        return Err(AtlasError::Other(
            "memoir repo_root must not be empty".to_owned(),
        ));
    }
    let id = derive_memoir_graph_id(&input.repo_root, input.scope, &name);
    let now = format_memory_now();
    let metadata_json = serde_json::to_string(&input.metadata).unwrap_or_else(|_| "{}".to_owned());

    conn.execute(
        "INSERT INTO memoir_graphs
            (id, repo_root, scope, name, description, created_at, updated_at, metadata_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7)",
        params![
            id,
            input.repo_root,
            input.scope.as_str(),
            name,
            input.description,
            now,
            metadata_json,
        ],
    )
    .map_err(|e| {
        let error = AtlasError::Db(e.to_string());
        if is_unique_violation(&error) {
            AtlasError::Other(format!(
                "memoir graph '{name}' already exists in repo scope '{}'",
                input.scope.as_str()
            ))
        } else {
            error
        }
    })?;

    Ok(MemoirGraph {
        id,
        repo_root: input.repo_root.clone(),
        scope: input.scope,
        name,
        description: input.description.clone(),
        created_at: now.clone(),
        updated_at: now,
        metadata: input.metadata.clone(),
    })
}

pub(super) fn add_memoir_concept(
    conn: &Connection,
    input: &NewMemoirConcept,
) -> Result<MemoirConcept> {
    let name = require_name("concept", &input.name)?;
    let kind = if input.kind.trim().is_empty() {
        default_concept_kind()
    } else {
        input.kind.trim().to_owned()
    };
    let graph = load_memoir_graph(conn, &input.repo_root, input.scope, &input.graph)?;
    let id = derive_memoir_concept_id(&graph.id, &name);
    let now = format_memory_now();
    let metadata_json = serde_json::to_string(&input.metadata).unwrap_or_else(|_| "{}".to_owned());

    conn.execute(
        "INSERT INTO memoir_concepts
            (id, graph_id, repo_root, name, description, kind, source_id, created_at,
             updated_at, metadata_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9)",
        params![
            id,
            graph.id,
            input.repo_root,
            name,
            input.description,
            kind,
            input.source_id,
            now,
            metadata_json,
        ],
    )
    .map_err(|e| {
        let error = AtlasError::Db(e.to_string());
        if is_unique_violation(&error) {
            AtlasError::Other(format!(
                "memoir concept '{name}' already exists in graph '{}'",
                graph.name
            ))
        } else {
            error
        }
    })?;

    Ok(MemoirConcept {
        id,
        graph_id: graph.id,
        repo_root: input.repo_root.clone(),
        name,
        description: input.description.clone(),
        kind,
        source_id: input.source_id.clone(),
        created_at: now.clone(),
        updated_at: now,
        metadata: input.metadata.clone(),
    })
}

pub(super) fn link_memoir_concepts(
    conn: &Connection,
    input: &NewMemoirRelation,
) -> Result<MemoirRelation> {
    validate_confidence(input.confidence)?;
    let normalized = normalize_memoir_relation(&input.relation)?;
    let source_name = require_name("source concept", &input.source)?;
    let target_name = require_name("target concept", &input.target)?;
    if canonical_name(&source_name) == canonical_name(&target_name) {
        return Err(AtlasError::Other(
            "memoir relation source and target must differ".to_owned(),
        ));
    }

    let graph = load_memoir_graph(conn, &input.repo_root, input.scope, &input.graph)?;
    let (source, target) = resolve_link_endpoints(conn, input, &graph, &source_name, &target_name)?;

    // `replaced_by` keeps the caller-visible direction explicit: the target
    // replaces the source, so canonical persistence swaps the endpoints.
    let (source_id, target_id) = if normalized.reversed {
        (target.id.clone(), source.id.clone())
    } else {
        (source.id.clone(), target.id.clone())
    };

    let id = derive_memoir_relation_id(&graph.id, &source_id, &target_id, normalized.relation_type);
    let now = format_memory_now();
    let mut metadata = input.metadata.clone();
    if let Some(alias) = normalized.alias
        && let Value::Object(map) = &mut metadata
    {
        map.insert("alias".to_owned(), Value::String(alias.to_owned()));
        if normalized.reversed {
            map.insert("direction".to_owned(), Value::String("reversed".to_owned()));
        }
    }
    let metadata_json = serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_owned());

    conn.execute(
        "INSERT INTO memoir_relations
            (id, graph_id, repo_root, source_concept_id, target_concept_id, relation_type,
             confidence, source_id, created_at, metadata_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            id,
            graph.id,
            input.repo_root,
            source_id,
            target_id,
            normalized.relation_type,
            input.confidence,
            input.source_id,
            now,
            metadata_json,
        ],
    )
    .map_err(|e| {
        let error = AtlasError::Db(e.to_string());
        if is_unique_violation(&error) {
            AtlasError::Other(format!(
                "memoir relation '{}' already exists between '{}' and '{}' in graph '{}'",
                normalized.relation_type, source.name, target.name, graph.name
            ))
        } else {
            error
        }
    })?;

    Ok(MemoirRelation {
        id,
        graph_id: graph.id,
        repo_root: input.repo_root.clone(),
        source_concept_id: source_id,
        target_concept_id: target_id,
        relation_type: normalized.relation_type.to_owned(),
        confidence: input.confidence,
        source_id: input.source_id.clone(),
        created_at: now,
        metadata,
    })
}

fn resolve_link_endpoints(
    conn: &Connection,
    input: &NewMemoirRelation,
    graph: &MemoirGraph,
    source_name: &str,
    target_name: &str,
) -> Result<(MemoirConcept, MemoirConcept)> {
    let source = match load_memoir_concept(conn, &graph.id, source_name)? {
        Some(concept) => concept,
        None if input.create_missing => {
            create_missing_endpoint(conn, input, graph, source_name, "source")?
        }
        None => return Err(missing_endpoint_error(graph, source_name)),
    };
    let target = match load_memoir_concept(conn, &graph.id, target_name)? {
        Some(concept) => concept,
        None if input.create_missing => {
            create_missing_endpoint(conn, input, graph, target_name, "target")?
        }
        None => return Err(missing_endpoint_error(graph, target_name)),
    };
    Ok((source, target))
}

fn missing_endpoint_error(graph: &MemoirGraph, name: &str) -> AtlasError {
    AtlasError::Other(format!(
        "memoir concept '{name}' not found in graph '{}'; pass create_missing to opt in to creation",
        graph.name
    ))
}

fn create_missing_endpoint(
    conn: &Connection,
    input: &NewMemoirRelation,
    graph: &MemoirGraph,
    name: &str,
    role: &str,
) -> Result<MemoirConcept> {
    add_memoir_concept(
        conn,
        &NewMemoirConcept {
            repo_root: input.repo_root.clone(),
            scope: input.scope,
            graph: graph.name.clone(),
            name: name.to_owned(),
            description: format!("Auto-created {role} endpoint for a memoir relation"),
            kind: default_concept_kind(),
            source_id: input.source_id.clone(),
            metadata: serde_json::json!({ "auto_created": true }),
        },
    )
}

// ── Inspect ───────────────────────────────────────────────────────────────────

pub(super) fn inspect_memoir_concept(
    conn: &Connection,
    query: &MemoirInspectQuery,
) -> Result<MemoirInspection> {
    let relation_filter = query
        .relation
        .as_deref()
        .map(normalize_memoir_relation)
        .transpose()?
        .map(|normalized| normalized.relation_type.to_owned());
    let graph = resolve_inspect_graph(conn, query)?;
    let root = match load_memoir_concept(conn, &graph.id, &query.concept)? {
        Some(concept) => concept,
        None => {
            return Err(AtlasError::Other(format!(
                "memoir concept '{}' not found in graph '{}'",
                query.concept, graph.name
            )));
        }
    };

    let depth = query.depth.clamp(1, MAX_MEMOIR_INSPECT_DEPTH);
    let mut visited = BTreeSet::new();
    visited.insert(root.id.clone());
    let mut seen_edges = BTreeSet::new();
    let mut edges = Vec::new();
    let mut queue = VecDeque::from([(root.id.clone(), 0_u32)]);
    let mut truncated = false;

    while let Some((concept_id, level)) = queue.pop_front() {
        if level >= depth {
            continue;
        }
        let mut statement = conn
            .prepare(
                "SELECT r.id, r.relation_type, r.confidence, r.source_id, r.created_at,
                        r.metadata_json, r.source_concept_id, s.name,
                        r.target_concept_id, t.name
                 FROM memoir_relations r
                 JOIN memoir_concepts s ON s.id = r.source_concept_id
                 JOIN memoir_concepts t ON t.id = r.target_concept_id
                 WHERE r.graph_id = ?1
                   AND (r.source_concept_id = ?2 OR r.target_concept_id = ?2)
                   AND (?3 IS NULL OR r.relation_type = ?3)
                 ORDER BY r.relation_type ASC, r.id ASC",
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        let rows = statement
            .query_map(params![graph.id, concept_id, relation_filter], |row| {
                let source_id: String = row.get(6)?;
                let target_id: String = row.get(8)?;
                let outgoing = source_id == concept_id;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    if outgoing {
                        MemoirRelationDirection::Outgoing
                    } else {
                        MemoirRelationDirection::Incoming
                    },
                    source_id,
                    row.get::<_, String>(7)?,
                    target_id,
                    row.get::<_, String>(9)?,
                    if outgoing {
                        row.get::<_, String>(8)?
                    } else {
                        row.get::<_, String>(6)?
                    },
                ))
            })
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        for row in rows {
            let (
                relation_id,
                relation_type,
                confidence,
                source_evidence_id,
                created_at,
                metadata_json,
                direction,
                from_concept_id,
                from_concept,
                to_concept_id,
                to_concept,
                neighbor_id,
            ) = row.map_err(|e| AtlasError::Db(e.to_string()))?;

            if seen_edges.insert(relation_id.clone()) {
                if edges.len() >= MAX_MEMOIR_INSPECT_EDGES {
                    truncated = true;
                    continue;
                }
                edges.push(MemoirInspectionEdge {
                    relation_id,
                    relation_type,
                    direction,
                    depth: level + 1,
                    from_concept_id,
                    from_concept,
                    to_concept_id,
                    to_concept,
                    confidence,
                    source_id: source_evidence_id,
                    created_at,
                    metadata: parse_metadata(metadata_json),
                });
            }

            if visited.insert(neighbor_id.clone()) {
                if visited.len() > MAX_MEMOIR_INSPECT_NODES {
                    truncated = true;
                    visited.remove(&neighbor_id);
                } else {
                    queue.push_back((neighbor_id, level + 1));
                }
            }
        }
    }

    edges.sort_by(|left, right| {
        left.depth
            .cmp(&right.depth)
            .then_with(|| left.relation_type.cmp(&right.relation_type))
            .then_with(|| left.relation_id.cmp(&right.relation_id))
    });

    Ok(MemoirInspection {
        graph,
        root,
        depth,
        relation: relation_filter,
        edges,
        truncated,
    })
}

fn resolve_inspect_graph(conn: &Connection, query: &MemoirInspectQuery) -> Result<MemoirGraph> {
    if let Some(graph_name) = query
        .graph
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        return load_memoir_graph(
            conn,
            &query.repo_root,
            query.scope.unwrap_or_default(),
            graph_name,
        );
    }

    // Resolve by concept name across graphs; ambiguous names fail instead of
    // silently picking one scope's graph.
    let sql = concat!(
        "SELECT g.id, g.repo_root, g.scope, g.name, g.description, g.created_at, ",
        "g.updated_at, g.metadata_json ",
        "FROM memoir_concepts c JOIN memoir_graphs g ON g.id = c.graph_id ",
        "WHERE c.repo_root = ?1 AND c.name = ?2",
        " AND (?3 IS NULL OR g.scope = ?3)"
    );
    let mut statement = conn
        .prepare(sql)
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    let rows = statement
        .query_map(
            params![
                query.repo_root,
                query.concept,
                query.scope.map(MemoryScope::as_str)
            ],
            row_to_memoir_graph,
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    let mut matches = Vec::new();
    for row in rows {
        matches.push(row.map_err(|e| AtlasError::Db(e.to_string()))?);
    }
    match matches.len() {
        0 => Err(AtlasError::Other(format!(
            "memoir concept '{}' not found in any graph for this repo",
            query.concept
        ))),
        1 => Ok(matches.remove(0)),
        _ => {
            let candidates = matches
                .iter()
                .map(|graph| format!("{} ({})", graph.name, graph.scope.as_str()))
                .collect::<Vec<_>>()
                .join(", ");
            Err(AtlasError::Other(format!(
                "memoir concept '{}' is ambiguous across graphs: {candidates}; pass graph name to disambiguate",
                query.concept
            )))
        }
    }
}

// ── Schema validation ─────────────────────────────────────────────────────────

fn table_columns(conn: &Connection, table: &str) -> Vec<String> {
    let sql = format!("PRAGMA table_info('{table}')");
    conn.prepare(&sql)
        .ok()
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get::<_, String>(1))
                .ok()
                .map(|rows| rows.filter_map(std::result::Result::ok).collect())
        })
        .unwrap_or_default()
}

fn table_exists(conn: &Connection, table: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |row| row.get::<_, i64>(0),
    )
    .unwrap_or(0)
        > 0
}

/// Returns schema issues for the memoir tables; empty when healthy.
pub(super) fn memoir_schema_issues(conn: &Connection) -> Vec<String> {
    let mut issues = Vec::new();
    for (table, columns) in [
        ("memoir_graphs", MEMOIR_GRAPH_COLUMNS),
        ("memoir_concepts", MEMOIR_CONCEPT_COLUMNS),
        ("memoir_relations", MEMOIR_RELATION_COLUMNS),
    ] {
        if !table_exists(conn, table) {
            issues.push(format!("missing table: {table}"));
            continue;
        }
        let present = table_columns(conn, table);
        for column in columns {
            if !present.iter().any(|name| name == column) {
                issues.push(format!("missing column: {table}.{column}"));
            }
        }
    }
    issues
}

// ── SessionStore surface ──────────────────────────────────────────────────────

impl super::SessionStore {
    /// Create a memoir graph in the repo; duplicate `(repo, scope, name)`
    /// combinations fail deterministically.
    pub fn create_memoir_graph(&mut self, input: &NewMemoirGraph) -> Result<MemoirGraph> {
        create_memoir_graph(&self.conn, input)
    }

    /// Add a concept to an existing memoir graph. Missing graphs are never
    /// auto-created.
    pub fn add_memoir_concept(&mut self, input: &NewMemoirConcept) -> Result<MemoirConcept> {
        add_memoir_concept(&self.conn, input)
    }

    /// Link two existing concepts; unknown relation types are rejected and
    /// missing endpoints require explicit `create_missing` opt-in.
    pub fn link_memoir_concepts(&mut self, input: &NewMemoirRelation) -> Result<MemoirRelation> {
        link_memoir_concepts(&self.conn, input)
    }

    /// Bounded memoir traversal with canonical relation direction and source
    /// evidence ids on every edge.
    pub fn inspect_memoir_concept(&self, query: &MemoirInspectQuery) -> Result<MemoirInspection> {
        inspect_memoir_concept(&self.conn, query)
    }

    /// Delete a memoir graph and its concepts/relations. Missing graphs report
    /// `found: false`; `dry_run` reports counts without writing.
    pub fn delete_memoir_graph(
        &self,
        repo_root: &str,
        scope: MemoryScope,
        name: &str,
        dry_run: bool,
    ) -> Result<super::memoir_delete::MemoirDeleteResult> {
        super::memoir_delete::delete_memoir_graph(&self.conn, repo_root, scope, name, dry_run)
    }

    /// Read-only concept listing for the memory TUI (ICM-H2).
    pub fn list_memoir_concepts(
        &self,
        repo_root: &str,
        limit: usize,
    ) -> Result<Vec<MemoirConcept>> {
        list_memoir_concepts(&self.conn, repo_root, limit)
    }

    /// Validates the memoir schema (`memoir_graphs`, `memoir_concepts`, and
    /// `memoir_relations`). Returns an empty list when healthy.
    pub fn memoir_schema_issues(&self) -> Vec<String> {
        memoir_schema_issues(&self.conn)
    }
}
