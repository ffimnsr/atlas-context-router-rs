//! Read-only collection layer for the memory TUI (ICM-H2).
//!
//! Every section is a bounded read against the session store (plus the routed
//! artifact index in `context.db`). No section writes, creates, or repairs
//! state, so browsing can never mutate memory, memoir, overview, or feedback
//! data.

use anyhow::{Context, Result};

use atlas_contentstore::ContentStore;
use atlas_session::{
    MemoryDecayPolicy, MemoryListFilter, OverviewFreshness, OverviewListFilter, OverviewSourceKind,
    OverviewSubjectKind, SessionStore,
};

/// Reader filters accepted by `atlas memory tui`.
#[derive(Debug, Clone)]
pub struct TuiFilters {
    /// Memory filters (topic, scope, importance, updated-at bounds).
    pub memory: MemoryListFilter,
    /// Filter memories and feedback by frontend identity.
    pub frontend: Option<String>,
    /// Overview subject-kind filter.
    pub subject_kind: Option<String>,
    /// Overview freshness filter.
    pub freshness: Option<String>,
    /// Overview package-name filter.
    pub package: Option<String>,
    /// Overview module-path filter.
    pub module: Option<String>,
    /// Overview canonical-file filter.
    pub file: Option<String>,
    /// Overview source-kind filter.
    pub source_kind: Option<String>,
    /// Only overview records updated before this timestamp.
    pub overview_older_than: Option<String>,
    /// Only overview records updated after this timestamp.
    pub overview_newer_than: Option<String>,
    /// Maximum rows rendered per section.
    pub limit: usize,
}

impl Default for TuiFilters {
    fn default() -> Self {
        Self {
            memory: MemoryListFilter::default(),
            frontend: None,
            subject_kind: None,
            freshness: None,
            package: None,
            module: None,
            file: None,
            source_kind: None,
            overview_older_than: None,
            overview_newer_than: None,
            limit: 50,
        }
    }
}

/// Fixed TUI sections; order defines tab cycling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TuiSection {
    Memories,
    Topics,
    Feedback,
    Memoir,
    Overview,
    Health,
    Artifacts,
}

impl TuiSection {
    pub const ALL: [TuiSection; 7] = [
        TuiSection::Memories,
        TuiSection::Topics,
        TuiSection::Feedback,
        TuiSection::Memoir,
        TuiSection::Overview,
        TuiSection::Health,
        TuiSection::Artifacts,
    ];

    pub fn title(self) -> &'static str {
        match self {
            TuiSection::Memories => "Memories",
            TuiSection::Topics => "Topics",
            TuiSection::Feedback => "Feedback",
            TuiSection::Memoir => "Memoir",
            TuiSection::Overview => "Overview",
            TuiSection::Health => "Health",
            TuiSection::Artifacts => "Artifacts",
        }
    }

    pub fn next(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|section| *section == self)
            .unwrap_or(0);
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    pub fn prev(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|section| *section == self)
            .unwrap_or(0);
        Self::ALL[(index + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

/// One rendered row: primary text plus bounded secondary detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TuiRow {
    pub primary: String,
    pub secondary: String,
}

impl TuiRow {
    pub fn new(primary: impl Into<String>, secondary: impl Into<String>) -> Self {
        Self {
            primary: primary.into(),
            secondary: secondary.into(),
        }
    }
}

/// Bounded read-only snapshot rendered by the TUI.
#[derive(Debug, Clone, Default)]
pub struct TuiModel {
    pub repo_root: String,
    sections: std::collections::BTreeMap<TuiSection, Vec<TuiRow>>,
    /// Warnings for sections that could not be read (never fatal).
    pub warnings: Vec<String>,
}

impl TuiModel {
    /// Empty model bound to one repo root.
    pub fn new(repo_root: impl Into<String>) -> Self {
        Self {
            repo_root: repo_root.into(),
            ..Self::default()
        }
    }

    pub fn rows(&self, section: TuiSection) -> &[TuiRow] {
        self.sections
            .get(&section)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn insert(&mut self, section: TuiSection, rows: Vec<TuiRow>) {
        self.sections.insert(section, rows);
    }

    pub fn total_rows(&self) -> usize {
        self.sections.values().map(Vec::len).sum()
    }
}

/// Collect every section within the reader filters.
pub fn collect_model(
    repo_root: &str,
    store: &SessionStore,
    filters: &TuiFilters,
) -> Result<TuiModel> {
    let limit = filters.limit.clamp(1, 500);
    let mut model = TuiModel::new(repo_root);

    let memories = store
        .list_memories(repo_root, &filters.memory)
        .context("cannot list memories for the memory TUI")?;
    let memories = memories
        .into_iter()
        .filter(|memory| {
            filters
                .frontend
                .as_deref()
                .is_none_or(|frontend| memory.frontend.as_deref() == Some(frontend))
        })
        .take(limit)
        .collect::<Vec<_>>();

    model.insert(
        TuiSection::Memories,
        memories
            .iter()
            .map(|memory| {
                TuiRow::new(
                    format!(
                        "{} [{} · {}]",
                        label(&memory.topic, &memory.title),
                        memory.importance,
                        memory.scope
                    ),
                    format!(
                        "{} · id {} · updated {}",
                        preview(&memory.body, 96),
                        memory.id,
                        memory.updated_at
                    ),
                )
            })
            .collect(),
    );

    let mut topic_counts: std::collections::BTreeMap<&str, (usize, usize)> =
        std::collections::BTreeMap::new();
    for memory in &memories {
        let entry = topic_counts.entry(memory.topic.as_str()).or_insert((0, 0));
        entry.0 += 1;
        if memory.importance == atlas_session::MemoryImportance::Critical {
            entry.1 += 1;
        }
    }
    model.insert(
        TuiSection::Topics,
        topic_counts
            .into_iter()
            .map(|(topic, (count, critical))| {
                TuiRow::new(
                    label(topic, "(untitled topic)"),
                    format!("{count} memories · {critical} critical"),
                )
            })
            .collect(),
    );

    let feedback = store
        .recent_feedback(repo_root, limit)
        .unwrap_or_else(|error| {
            model
                .warnings
                .push(format!("feedback unavailable: {error}"));
            Vec::new()
        });
    model.insert(
        TuiSection::Feedback,
        feedback
            .iter()
            .map(|record| {
                TuiRow::new(
                    format!("{} → {}", record.predicted, record.actual),
                    format!(
                        "{} · {} · {}",
                        record.tool_name, record.analysis_kind, record.created_at
                    ),
                )
            })
            .collect(),
    );

    let concepts = store
        .list_memoir_concepts(repo_root, limit)
        .unwrap_or_else(|error| {
            model.warnings.push(format!("memoir unavailable: {error}"));
            Vec::new()
        });
    model.insert(
        TuiSection::Memoir,
        concepts
            .iter()
            .map(|concept| {
                TuiRow::new(
                    format!("{} [{}]", concept.name, concept.kind),
                    format!("{} · id {}", preview(&concept.description, 80), concept.id),
                )
            })
            .collect(),
    );

    let overview_rows = collect_overview_rows(repo_root, store, filters, limit, &mut model);
    model.insert(TuiSection::Overview, overview_rows);

    let health = store
        .memory_health(
            repo_root,
            &filters.memory,
            &MemoryDecayPolicy::default(),
            &|_| true,
        )
        .map_err(|error| {
            model.warnings.push(format!("health unavailable: {error}"));
        })
        .ok();
    let mut health_rows = health
        .as_ref()
        .map(|health| {
            health
                .by_category
                .iter()
                .map(|(category, count)| TuiRow::new(category.clone(), format!("{count} findings")))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if let Some(health) = health.as_ref() {
        for finding in health.findings.iter().take(limit) {
            health_rows.push(TuiRow::new(
                format!("{:?}", finding.category),
                preview(&finding.detail, 100),
            ));
        }
    }
    model.insert(TuiSection::Health, health_rows);

    let artifact_rows = collect_artifact_rows(repo_root, limit, &mut model);
    model.insert(TuiSection::Artifacts, artifact_rows);

    Ok(model)
}

fn collect_overview_rows(
    repo_root: &str,
    store: &SessionStore,
    filters: &TuiFilters,
    limit: usize,
    model: &mut TuiModel,
) -> Vec<TuiRow> {
    let list_filter = OverviewListFilter {
        subject_kind: parse_opt::<OverviewSubjectKind>(filters.subject_kind.as_deref()),
        freshness: parse_opt::<OverviewFreshness>(filters.freshness.as_deref()),
        source_kind: parse_opt::<OverviewSourceKind>(filters.source_kind.as_deref()),
        older_than: filters.overview_older_than.clone(),
        newer_than: filters.overview_newer_than.clone(),
        limit: Some(limit),
    };
    if (filters.subject_kind.is_some() && list_filter.subject_kind.is_none())
        || (filters.freshness.is_some() && list_filter.freshness.is_none())
        || (filters.source_kind.is_some() && list_filter.source_kind.is_none())
    {
        model
            .warnings
            .push("ignored unknown overview filter value".to_owned());
    }

    let records = store
        .list_overview_records(repo_root, &list_filter)
        .unwrap_or_else(|error| {
            model
                .warnings
                .push(format!("overview unavailable: {error}"));
            Vec::new()
        });

    records
        .into_iter()
        .filter(|record| {
            filters
                .package
                .as_deref()
                .is_none_or(|package| record.package_name.as_deref() == Some(package))
                && filters
                    .module
                    .as_deref()
                    .is_none_or(|module| record.module_path.as_deref() == Some(module))
                && filters
                    .file
                    .as_deref()
                    .is_none_or(|file| record.canonical_file_path.as_deref() == Some(file))
        })
        .take(limit)
        .map(|record| {
            TuiRow::new(
                format!(
                    "{} [{} · {:?}]",
                    record.title, record.subject_id, record.freshness
                ),
                format!(
                    "{} · {} · id {}",
                    record.subject_kind.as_str(),
                    preview(&record.summary, 80),
                    record.id
                ),
            )
        })
        .collect()
}

fn collect_artifact_rows(repo_root: &str, limit: usize, model: &mut TuiModel) -> Vec<TuiRow> {
    let graph_db = atlas_engine::paths::default_db_path(repo_root);
    let content_db = atlas_engine::paths::content_db_path(&graph_db);
    let Ok(mut store) = ContentStore::open(&content_db) else {
        model.warnings.push("artifact index unavailable".to_owned());
        return Vec::new();
    };
    if store.migrate().is_err() {
        model
            .warnings
            .push("artifact index cannot migrate".to_owned());
        return Vec::new();
    }
    let Ok(ids) =
        store.recent_source_ids_by_prefix("", &atlas_contentstore::SearchFilters::default(), limit)
    else {
        return Vec::new();
    };
    ids.into_iter()
        .filter_map(|id| {
            store.get_source(&id).ok().flatten().map(|source| {
                TuiRow::new(
                    format!("{} [{}]", source.label, source.source_type),
                    format!("{} · {}", source.id, source.created_at),
                )
            })
        })
        .collect()
}

fn parse_opt<T: std::str::FromStr>(raw: Option<&str>) -> Option<T> {
    raw.and_then(|value| value.parse::<T>().ok())
}

fn label(topic: &str, title: &str) -> String {
    match (topic.trim().is_empty(), title.trim().is_empty()) {
        (false, false) => format!("{topic} · {title}"),
        (false, true) => topic.to_owned(),
        (true, false) => title.to_owned(),
        (true, true) => "(untitled)".to_owned(),
    }
}

/// Character-bounded preview with an ellipsis marker.
pub fn preview(text: &str, max_chars: usize) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    let mut truncated = normalized
        .chars()
        .take(max_chars.saturating_sub(1))
        .collect::<String>();
    truncated.push('…');
    truncated
}
