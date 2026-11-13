//! Deterministic symbol resolution shared by MCP `resolve_symbol`, CLI
//! `symbols --resolve`, and query-intent execution.
//!
//! Resolution order is fixed: exact qualified name, exact node name, name
//! prefix. The fuzzy/lexical fallback only runs when the caller explicitly opts
//! in with `fuzzy = true`. Filters are applied before ranking, and the stage
//! that produced the winning matches is reported so callers can distinguish an
//! exact hit from a prefix or fuzzy guess.

use atlas_core::{Result, ScoredNode, SearchQuery};
use atlas_store_sqlite::Store;

/// Resolution stage that produced the returned matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionStage {
    /// Exact `qualified_name` column match.
    QualifiedName,
    /// Exact `name` column match.
    ExactName,
    /// `name` prefix match.
    Prefix,
    /// Explicit opt-in fuzzy lexical match.
    Fuzzy,
}

impl ResolutionStage {
    pub fn as_str(self) -> &'static str {
        match self {
            ResolutionStage::QualifiedName => "qualified_name",
            ResolutionStage::ExactName => "exact_name",
            ResolutionStage::Prefix => "prefix",
            ResolutionStage::Fuzzy => "fuzzy",
        }
    }
}

/// Input for [`resolve_symbol_nodes`].
#[derive(Debug, Clone)]
pub struct SymbolResolveRequest<'a> {
    pub name: &'a str,
    pub kind: Option<&'a str>,
    pub language: Option<&'a str>,
    pub file: Option<&'a str>,
    pub limit: usize,
    pub fuzzy: bool,
}

/// Resolution outcome.
#[derive(Debug, Clone)]
pub struct SymbolResolution {
    /// Stage that produced `matches`; `None` when nothing matched.
    pub stage: Option<ResolutionStage>,
    /// Matches after filtering, ranked and capped at `limit`.
    pub matches: Vec<atlas_core::Node>,
    /// `true` when matches were dropped by `limit`.
    pub truncated: bool,
}

impl SymbolResolution {
    /// Status string used on the wire: `resolved`, `ambiguous`, or `not_found`.
    pub fn status(&self) -> &'static str {
        match self.matches.len() {
            0 => "not_found",
            1 => "resolved",
            _ => "ambiguous",
        }
    }

    pub fn best_match(&self) -> Option<&atlas_core::Node> {
        self.matches.first()
    }
}

fn canonical_kind(kind: Option<&str>) -> Option<String> {
    kind.map(atlas_core::kinds::normalize_kind_alias)
}

/// Resolve a symbol name through the fixed resolution ladder.
pub fn resolve_symbol_nodes(
    store: &Store,
    request: &SymbolResolveRequest<'_>,
) -> Result<SymbolResolution> {
    let name = request.name.trim();
    let limit = request.limit.max(1);
    let fetch = (limit * 4).max(40);
    let kind = canonical_kind(request.kind);
    let keep = |node: &atlas_core::Node| {
        request
            .file
            .is_none_or(|pattern| node.file_path.contains(pattern))
            && kind
                .as_deref()
                .is_none_or(|kind| node.kind.as_str() == kind)
            && request
                .language
                .is_none_or(|language| node.language == language)
    };

    if name.is_empty() {
        return Ok(SymbolResolution {
            stage: None,
            matches: Vec::new(),
            truncated: false,
        });
    }

    let mut stage = None;
    let mut matched: Vec<atlas_core::Node> = Vec::new();

    if let Some(node) = store.node_by_qname(name)?
        && keep(&node)
    {
        stage = Some(ResolutionStage::QualifiedName);
        matched.push(node);
    }

    if matched.is_empty() {
        matched = store
            .nodes_by_name(name, fetch)?
            .into_iter()
            .filter(keep)
            .collect();
        if !matched.is_empty() {
            stage = Some(ResolutionStage::ExactName);
        }
    }

    if matched.is_empty() {
        matched = store
            .nodes_by_name_prefix(name, fetch)?
            .into_iter()
            .filter(keep)
            .collect();
        if !matched.is_empty() {
            stage = Some(ResolutionStage::Prefix);
        }
    }

    if matched.is_empty() && request.fuzzy {
        let query = SearchQuery {
            text: name.to_owned(),
            kind: kind.clone(),
            language: request.language.map(str::to_owned),
            limit: fetch,
            ..Default::default()
        };
        matched = crate::execute_query(store, &query, false)?
            .into_iter()
            .map(|scored: ScoredNode| scored.node)
            .filter(keep)
            .collect();
        if !matched.is_empty() {
            stage = Some(ResolutionStage::Fuzzy);
        }
    }

    // Stable dedupe: identical qualified names can repeat across query paths.
    let mut seen = std::collections::HashSet::new();
    matched.retain(|node| seen.insert(node.qualified_name.clone()));

    let truncated = matched.len() > limit;
    matched.truncate(limit);

    Ok(SymbolResolution {
        stage,
        matches: matched,
        truncated,
    })
}
