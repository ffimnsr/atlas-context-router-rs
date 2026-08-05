use std::fs;
use std::path::{Path, PathBuf};

use atlas_core::{
    AtlasError, GraphStoreHealthClass, classify_graph_store_error, graph_health_error_message,
    graph_health_error_suggestions,
};
use serde::{Deserialize, Serialize};
use time::{OffsetDateTime, format_description::FormatItem, macros::format_description};
use tracing::warn;

use super::Store;

const STRUCTURAL_SCAN_LIMIT: usize = 100;
const QUARANTINE_TIMESTAMP_FORMAT: &[FormatItem<'static>] =
    format_description!("[year][month][day]T[hour][minute][second]Z");

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphRecoveryMode {
    ManualRebuildRequired,
    AutoQuarantineAndRebuild,
    BlockOnly,
}

impl GraphRecoveryMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ManualRebuildRequired => "manual_rebuild_required",
            Self::AutoQuarantineAndRebuild => "auto_quarantine_and_rebuild",
            Self::BlockOnly => "block_only",
        }
    }
}

impl std::fmt::Display for GraphRecoveryMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphStoreRecovery {
    pub recovery_mode: GraphRecoveryMode,
    pub health_class: Option<GraphStoreHealthClass>,
    pub quarantine_path: Option<String>,
    pub full_rebuild_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphStoreRecoveryError {
    pub db_path: Box<str>,
    pub health_class: GraphStoreHealthClass,
    pub recovery_mode: GraphRecoveryMode,
    pub quarantine_path: Option<Box<str>>,
    pub error_code: &'static str,
    pub message: &'static str,
    pub suggestions: &'static [&'static str],
    pub failure_reason: Option<Box<str>>,
}

impl GraphStoreRecoveryError {
    fn new(
        db_path: &str,
        health_class: GraphStoreHealthClass,
        recovery_mode: GraphRecoveryMode,
        quarantine_path: Option<String>,
        failure_reason: Option<String>,
    ) -> Self {
        let error_code = health_class.as_str();
        Self {
            db_path: db_path.into(),
            health_class,
            recovery_mode,
            quarantine_path: quarantine_path.map(Into::into),
            message: graph_health_error_message(error_code),
            suggestions: graph_health_error_suggestions(error_code),
            error_code,
            failure_reason: failure_reason.map(Into::into),
        }
    }
}

impl std::fmt::Display for GraphStoreRecoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(reason) = &self.failure_reason {
            write!(f, "{}: {reason}", self.message)
        } else {
            f.write_str(self.message)
        }
    }
}

impl std::error::Error for GraphStoreRecoveryError {}

impl Store {
    pub fn graph_store_health_class(&self) -> Result<Option<GraphStoreHealthClass>, AtlasError> {
        match self.integrity_check() {
            Ok(issues) if !issues.is_empty() => {
                if issues.iter().any(|issue| {
                    issue.starts_with("foreign_key_check:")
                        || issue.starts_with("noncanonical_path:")
                        || issue.starts_with("missing_repo_provenance:")
                }) {
                    return Ok(Some(GraphStoreHealthClass::LogicalInconsistency));
                }
                return Ok(Some(GraphStoreHealthClass::SqliteCorrupt));
            }
            Ok(_) => {}
            Err(error) => {
                return Ok(Some(classify_graph_store_error(&error.to_string())));
            }
        }

        let structural_dangling = self
            .dangling_edges(STRUCTURAL_SCAN_LIMIT)?
            .into_iter()
            .filter(|(_, _, _, kind, _)| {
                matches!(
                    kind.as_str(),
                    "contains"
                        | "defines"
                        | "implements"
                        | "extends"
                        | "imports"
                        | "tests"
                        | "tested_by"
                )
            })
            .count();
        if structural_dangling > 0 {
            return Ok(Some(GraphStoreHealthClass::LogicalInconsistency));
        }

        Ok(None)
    }

    pub fn prepare_graph_store_rebuild(
        db_path: &str,
        recovery_mode: GraphRecoveryMode,
        allow_automatic_quarantine: bool,
    ) -> std::result::Result<GraphStoreRecovery, GraphStoreRecoveryError> {
        let health_class = inspect_graph_store_health(db_path)?;
        let Some(health_class) = health_class else {
            return Ok(GraphStoreRecovery {
                recovery_mode,
                health_class: None,
                quarantine_path: None,
                full_rebuild_required: false,
            });
        };

        if !matches!(
            health_class,
            GraphStoreHealthClass::SqliteCorrupt
                | GraphStoreHealthClass::SchemaMismatch
                | GraphStoreHealthClass::LogicalInconsistency
        ) {
            return Ok(GraphStoreRecovery {
                recovery_mode,
                health_class: Some(health_class),
                quarantine_path: None,
                full_rebuild_required: false,
            });
        }

        match recovery_mode {
            GraphRecoveryMode::AutoQuarantineAndRebuild => {
                if !allow_automatic_quarantine {
                    return Err(GraphStoreRecoveryError::new(
                        db_path,
                        health_class,
                        GraphRecoveryMode::ManualRebuildRequired,
                        None,
                        Some(
                            "automatic graph DB quarantine requires explicit opt-in; rerun through an explicit build/update entry point or pass the dedicated allow flag".to_owned(),
                        ),
                    ));
                }
                let quarantine_path = quarantine_graph_store(db_path).map_err(|error| {
                    GraphStoreRecoveryError::new(
                        db_path,
                        health_class,
                        recovery_mode,
                        None,
                        Some(error.to_string()),
                    )
                })?;
                Store::open(db_path).map_err(|error| {
                    GraphStoreRecoveryError::new(
                        db_path,
                        health_class,
                        recovery_mode,
                        Some(quarantine_path.clone()),
                        Some(format!(
                            "fresh graph database creation failed after quarantine: {error}"
                        )),
                    )
                })?;
                Ok(GraphStoreRecovery {
                    recovery_mode,
                    health_class: Some(health_class),
                    quarantine_path: Some(quarantine_path),
                    full_rebuild_required: true,
                })
            }
            GraphRecoveryMode::ManualRebuildRequired | GraphRecoveryMode::BlockOnly => Err(
                GraphStoreRecoveryError::new(db_path, health_class, recovery_mode, None, None),
            ),
        }
    }
}

fn inspect_graph_store_health(
    db_path: &str,
) -> std::result::Result<Option<GraphStoreHealthClass>, GraphStoreRecoveryError> {
    if !Path::new(db_path).exists() {
        return Ok(None);
    }

    let store = match Store::open(db_path) {
        Ok(store) => store,
        Err(error) => {
            let health_class = classify_graph_store_error(&error.to_string());
            return Ok(Some(health_class));
        }
    };

    store.graph_store_health_class().map_err(|error| {
        GraphStoreRecoveryError::new(
            db_path,
            classify_graph_store_error(&error.to_string()),
            GraphRecoveryMode::BlockOnly,
            None,
            Some(error.to_string()),
        )
    })
}

fn quarantine_graph_store(db_path: &str) -> Result<String, AtlasError> {
    let db_file = Path::new(db_path);
    let Some(parent) = db_file.parent() else {
        return Err(AtlasError::Other(format!(
            "cannot quarantine graph DB without parent directory: {db_path}"
        )));
    };
    let Some(file_name) = db_file.file_name().and_then(|name| name.to_str()) else {
        return Err(AtlasError::Other(format!(
            "cannot quarantine graph DB with non-utf8 file name: {db_path}"
        )));
    };

    let timestamp = OffsetDateTime::now_utc()
        .format(QUARANTINE_TIMESTAMP_FORMAT)
        .map_err(|error| {
            AtlasError::Other(format!("cannot format quarantine timestamp: {error}"))
        })?;

    let artifacts = sqlite_graph_artifacts(db_file);
    let existing_artifacts = artifacts
        .into_iter()
        .filter(|(source, _)| source.exists())
        .collect::<Vec<_>>();

    if existing_artifacts.is_empty() {
        return Err(AtlasError::Other(format!(
            "cannot quarantine missing graph DB artifacts at {db_path}"
        )));
    }

    for attempt in 0usize.. {
        let quarantine_base = parent.join(format!("{file_name}.quarantine.{timestamp}.{attempt}"));
        let collision = existing_artifacts
            .iter()
            .any(|(_, suffix)| quarantine_destination_path(&quarantine_base, suffix).exists());
        if collision {
            continue;
        }

        for (source, suffix) in &existing_artifacts {
            let destination = quarantine_destination_path(&quarantine_base, suffix);
            fs::rename(source, &destination).map_err(|error| {
                AtlasError::Other(format!(
                    "cannot move {} to {} during graph DB quarantine: {error}",
                    source.display(),
                    destination.display()
                ))
            })?;
        }

        let quarantine_path = quarantine_base.to_string_lossy().into_owned();
        warn!(
            path = db_path,
            quarantine = %quarantine_path,
            "graph DB quarantined before rebuild"
        );
        return Ok(quarantine_path);
    }

    unreachable!("quarantine attempt loop must return once a suffix is free")
}

fn sqlite_graph_artifacts(db_file: &Path) -> Vec<(PathBuf, &'static str)> {
    vec![
        (db_file.to_path_buf(), ""),
        (PathBuf::from(format!("{}-wal", db_file.display())), "-wal"),
        (PathBuf::from(format!("{}-shm", db_file.display())), "-shm"),
        (PathBuf::from(format!("{}.wal", db_file.display())), ".wal"),
        (PathBuf::from(format!("{}.shm", db_file.display())), ".shm"),
    ]
}

fn quarantine_destination_path(quarantine_base: &Path, suffix: &str) -> PathBuf {
    PathBuf::from(format!("{}{suffix}", quarantine_base.display()))
}
