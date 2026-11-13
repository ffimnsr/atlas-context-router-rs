//! Shared doctor/schema probes for CLI and MCP surfaces.
//!
//! The CLI `doctor` command and the MCP `doctor` tool previously discovered
//! their own check sets, which drifted (`path_canonicality` vs
//! `content_path_identity`, no `schema_version` check, divergent wire field
//! names). This module owns the probe logic for the checks that must be
//! byte-identical across both surfaces and emits a single wire shape.

use std::path::Path;

use atlas_contentstore::CONTENT_SCHEMA_VERSION;
use atlas_contentstore::ContentStore;
use atlas_session::SESSION_SCHEMA_VERSION;
use atlas_session::SessionStore;
use atlas_store_sqlite::GRAPH_SCHEMA_VERSION;
use atlas_store_sqlite::Store;
use serde::Serialize;

use crate::paths::{content_db_path, session_db_path};

/// Check names every doctor surface must emit, in canonical order.
///
/// Keep this list frozen: CLI and MCP both append these checks and tests assert
/// parity against it. Adding a new shared check means bumping this list and
/// wiring it into both builders.
pub const SHARED_DOCTOR_CHECKS: &[&str] = &[
    "schema_version",
    "session_schema",
    "path_canonicality",
    "connection_mode",
];

/// Canonical shared check names.
pub fn canonical_check_names() -> &'static [&'static str] {
    SHARED_DOCTOR_CHECKS
}

/// Result of probing one SQLite store's schema version.
#[derive(Debug, Clone, Serialize)]
pub struct SchemaProbe {
    /// Store label: `graph`, `content`, or `session`.
    pub label: &'static str,
    /// Absolute path to the database file.
    pub path: String,
    /// Whether the file existed before the probe. Missing optional stores are
    /// created on demand and are not schema failures.
    pub exists: bool,
    /// Version recorded in the store metadata table, when readable.
    pub schema_version: Option<i32>,
    /// Highest version this binary supports.
    pub supported_version: i32,
    /// `true` when the store is absent, readable, and not newer than support.
    pub ok: bool,
    /// Open/read error, or the newer-than-support explanation.
    pub error: Option<String>,
}

impl SchemaProbe {
    fn missing(label: &'static str, path: String, supported_version: i32) -> Self {
        Self {
            label,
            path,
            exists: false,
            schema_version: None,
            supported_version,
            ok: true,
            error: None,
        }
    }

    fn from_open_result(
        label: &'static str,
        path: String,
        supported_version: i32,
        opened_version: Option<i32>,
        error: Option<String>,
    ) -> Self {
        match (opened_version, error) {
            (Some(version), None) => {
                let newer = version > supported_version;
                Self {
                    label,
                    path,
                    exists: true,
                    schema_version: Some(version),
                    supported_version,
                    ok: !newer,
                    error: newer.then(|| {
                        format!(
                            "database schema version {version} newer than atlas-supported version {supported_version}; upgrade Atlas to open this {label} store"
                        )
                    }),
                }
            }
            (_, error) => Self {
                label,
                path,
                exists: true,
                schema_version: None,
                supported_version,
                ok: false,
                error,
            },
        }
    }

    /// One-line `label=version/supported` fragment for check messages.
    pub fn describe(&self) -> String {
        if !self.exists {
            return format!("{}=", self.label);
        }
        match self.schema_version {
            Some(version) => format!("{}={version}/{}", self.label, self.supported_version),
            None => format!("{}=unreadable/{}", self.label, self.supported_version),
        }
    }
}

/// Probe the graph store schema version.
pub fn probe_graph_schema(db_path: &str) -> SchemaProbe {
    let path = db_path.to_owned();
    if !Path::new(db_path).exists() {
        return SchemaProbe::missing("graph", path, GRAPH_SCHEMA_VERSION);
    }
    match Store::open(db_path) {
        Ok(store) => {
            let version = store.schema_version().ok();
            SchemaProbe::from_open_result(
                "graph",
                path,
                GRAPH_SCHEMA_VERSION,
                version,
                version
                    .is_none()
                    .then(|| "cannot read graph schema_version".to_owned()),
            )
        }
        Err(error) => SchemaProbe::from_open_result(
            "graph",
            path,
            GRAPH_SCHEMA_VERSION,
            None,
            Some(error.to_string()),
        ),
    }
}

/// Probe the content-store (`context.db`) schema version.
pub fn probe_content_schema(content_db_path: &str) -> SchemaProbe {
    let path = content_db_path.to_owned();
    if !Path::new(content_db_path).exists() {
        return SchemaProbe::missing("content", path, CONTENT_SCHEMA_VERSION);
    }
    match ContentStore::open(content_db_path) {
        Ok(store) => {
            let version = store.schema_version().ok();
            SchemaProbe::from_open_result(
                "content",
                path,
                CONTENT_SCHEMA_VERSION,
                version,
                version
                    .is_none()
                    .then(|| "cannot read content schema_version".to_owned()),
            )
        }
        Err(error) => SchemaProbe::from_open_result(
            "content",
            path,
            CONTENT_SCHEMA_VERSION,
            None,
            Some(error.to_string()),
        ),
    }
}

/// Probe the session-store (`session.db`) schema version.
pub fn probe_session_schema(session_db_path: &str) -> SchemaProbe {
    let path = session_db_path.to_owned();
    if !Path::new(session_db_path).exists() {
        return SchemaProbe::missing("session", path, SESSION_SCHEMA_VERSION);
    }
    match SessionStore::open(session_db_path) {
        Ok(store) => {
            let version = store.schema_version().ok();
            SchemaProbe::from_open_result(
                "session",
                path,
                SESSION_SCHEMA_VERSION,
                version,
                version
                    .is_none()
                    .then(|| "cannot read session schema_version".to_owned()),
            )
        }
        Err(error) => SchemaProbe::from_open_result(
            "session",
            path,
            SESSION_SCHEMA_VERSION,
            None,
            Some(error.to_string()),
        ),
    }
}

/// Unified wire shape for one doctor check.
///
/// Both `check`/`name`, `ok`/`status`, and `detail`/`message` aliases are
/// emitted so older CLI/MCP consumers keep working while the surfaces converge.
#[derive(Debug, Clone, Serialize)]
pub struct DoctorCheck {
    /// Canonical check name (MCP spelling).
    pub name: &'static str,
    /// Legacy CLI spelling alias of [`Self::name`].
    pub check: &'static str,
    /// `pass` or `fail`.
    pub status: &'static str,
    /// Boolean form of [`Self::status`].
    pub ok: bool,
    /// Human-readable detail (MCP spelling).
    pub message: String,
    /// Legacy CLI spelling alias of [`Self::message`].
    pub detail: String,
    /// Stable machine code for failures, when one exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue_code: Option<&'static str>,
    /// Structured detail block kept for MCP consumers.
    pub details: DoctorCheckDetails,
}

/// Structured details embedded in [`DoctorCheck`].
#[derive(Debug, Clone, Serialize)]
pub struct DoctorCheckDetails {
    pub label: &'static str,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue_code: Option<&'static str>,
    /// Row count for `path_canonicality`; absent on other checks.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub noncanonical_path_rows: Option<usize>,
}

impl DoctorCheck {
    /// Build a passing check.
    pub fn pass(name: &'static str, message: impl Into<String>) -> Self {
        Self::build(name, true, message.into(), None)
    }

    /// Build a failing check.
    pub fn fail(
        name: &'static str,
        message: impl Into<String>,
        issue_code: Option<&'static str>,
    ) -> Self {
        Self::build(name, false, message.into(), issue_code)
    }

    fn build(
        name: &'static str,
        ok: bool,
        message: String,
        issue_code: Option<&'static str>,
    ) -> Self {
        Self {
            name,
            check: name,
            status: if ok { "pass" } else { "fail" },
            ok,
            detail: message.clone(),
            message,
            issue_code,
            details: DoctorCheckDetails {
                label: name,
                ok,
                issue_code,
                noncanonical_path_rows: None,
            },
        }
    }

    /// Attach the noncanonical row count for the path-canonicality check.
    pub fn with_noncanonical_rows(mut self, rows: usize) -> Self {
        self.details.noncanonical_path_rows = Some(rows);
        self
    }

    /// Suggested remediation for a failed check.
    pub fn fix_hint(&self) -> Option<String> {
        match self.issue_code {
            Some("schema_mismatch") => Some(
                "upgrade Atlas to the binary version that wrote this database, or rebuild the store"
                    .to_owned(),
            ),
            Some("noncanonical_path_rows") => {
                Some("rebuild graph/content state from canonical repo paths".to_owned())
            }
            _ => None,
        }
    }
}

/// Aggregate schema-version check across graph, content, and session stores.
///
/// A store whose `user_version` is newer than this binary's migration set fails
/// with `issue_code = "schema_mismatch"` and never gets classified as
/// corruption, so the database is preserved for a newer Atlas binary.
pub fn schema_version_check(
    graph: &SchemaProbe,
    content: &SchemaProbe,
    session: &SchemaProbe,
) -> DoctorCheck {
    let mut failures = Vec::new();
    for probe in [graph, content, session] {
        if probe.exists && !probe.ok {
            failures.push(
                probe
                    .error
                    .clone()
                    .unwrap_or_else(|| format!("{} schema is unreadable", probe.label)),
            );
        }
    }
    let message = format!(
        "{} {} {}",
        graph.describe(),
        content.describe(),
        session.describe()
    );
    if failures.is_empty() {
        DoctorCheck::pass("schema_version", message)
    } else {
        DoctorCheck::fail(
            "schema_version",
            failures.join("; "),
            Some("schema_mismatch"),
        )
    }
}

/// Validate session-store table schemas (memory, feedback, overview,
/// experience) using the same probe as `db_check`.
pub fn session_schema_check(session_db_path: &str) -> DoctorCheck {
    if !Path::new(session_db_path).exists() {
        return DoctorCheck::pass("session_schema", "session store not initialised");
    }
    match SessionStore::open(session_db_path) {
        Ok(store) => {
            let mut issues = store.memory_schema_issues();
            issues.extend(store.feedback_schema_issues());
            issues.extend(store.overview_schema_issues());
            issues.extend(store.experience_schema_issues());
            if issues.is_empty() {
                DoctorCheck::pass("session_schema", "ok")
            } else {
                DoctorCheck::fail("session_schema", issues.join("; "), Some("schema_mismatch"))
            }
        }
        Err(error) => {
            DoctorCheck::fail("session_schema", error.to_string(), Some("schema_mismatch"))
        }
    }
}

/// Emit the `path_canonicality` check with the noncanonical row count.
///
/// This replaces the older CLI-only `content_path_identity` check name and
/// keeps the row count visible on both surfaces.
pub fn path_canonicality_check(content_db_path: &str) -> DoctorCheck {
    if !Path::new(content_db_path).exists() {
        return DoctorCheck::pass("path_canonicality", "noncanonical_path_rows=0")
            .with_noncanonical_rows(0);
    }
    match ContentStore::open(content_db_path) {
        Ok(store) => match store.noncanonical_repo_path_sources(usize::MAX) {
            Ok(issues) if issues.is_empty() => {
                DoctorCheck::pass("path_canonicality", "noncanonical_path_rows=0")
                    .with_noncanonical_rows(0)
            }
            Ok(issues) => DoctorCheck::fail(
                "path_canonicality",
                format!(
                    "noncanonical_path_rows={}: {}",
                    issues.len(),
                    issues.join("; ")
                ),
                Some("noncanonical_path_rows"),
            )
            .with_noncanonical_rows(issues.len()),
            Err(error) => DoctorCheck::fail(
                "path_canonicality",
                error.to_string(),
                Some("noncanonical_path_rows"),
            ),
        },
        Err(error) => DoctorCheck::fail(
            "path_canonicality",
            error.to_string(),
            Some("noncanonical_path_rows"),
        ),
    }
}

/// Static `connection_mode` check shared by both surfaces.
pub fn connection_mode_check() -> DoctorCheck {
    DoctorCheck::pass(
        "connection_mode",
        "single_connection_per_store; parallel_parse+sequential_persistence; no_read_pool (pooled graph reads not implemented)",
    )
}

/// Probe all three stores under one repo/db pair.
pub fn probe_all_schemas(db_path: &str) -> (SchemaProbe, SchemaProbe, SchemaProbe) {
    let content = probe_content_schema(&content_db_path(db_path));
    let session = probe_session_schema(&session_db_path(db_path));
    let graph = probe_graph_schema(db_path);
    (graph, content, session)
}

/// Build the shared doctor checks every surface must emit.
///
/// Order matches [`SHARED_DOCTOR_CHECKS`]. CLI and MCP append these verbatim
/// after their surface-specific checks.
pub fn shared_checks(db_path: &str) -> Vec<DoctorCheck> {
    let (graph, content, session) = probe_all_schemas(db_path);
    vec![
        schema_version_check(&graph, &content, &session),
        session_schema_check(&session.path),
        path_canonicality_check(&content.path),
        connection_mode_check(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bump_schema(path: &std::path::Path, version: i32) {
        let conn = rusqlite::Connection::open(path).expect("open raw db");
        conn.execute(
            "UPDATE metadata SET value = ?1 WHERE key = 'schema_version'",
            [version.to_string()],
        )
        .expect("bump metadata schema_version");
        conn.execute_batch(&format!("PRAGMA user_version={version}"))
            .expect("bump user_version");
    }

    #[test]
    fn newer_content_schema_is_reported_as_schema_mismatch() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("context.db");
        let path_str = path.to_string_lossy().to_string();
        drop(ContentStore::open(&path_str).expect("create content store"));
        bump_schema(&path, CONTENT_SCHEMA_VERSION + 1);

        let probe = probe_content_schema(&path_str);
        assert!(!probe.ok);
        assert!(probe.exists);
        let error = probe.error.unwrap_or_default();
        assert!(
            error.contains("newer than atlas-supported version"),
            "unexpected error: {error}"
        );
        assert!(atlas_core::is_schema_mismatch_error(&error));
    }

    #[test]
    fn newer_session_schema_is_reported_as_schema_mismatch() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("session.db");
        let path_str = path.to_string_lossy().to_string();
        drop(SessionStore::open(&path_str).expect("create session store"));
        bump_schema(&path, SESSION_SCHEMA_VERSION + 1);

        let probe = probe_session_schema(&path_str);
        assert!(!probe.ok);
        assert!(
            probe
                .error
                .as_deref()
                .is_some_and(|error| error.contains("newer than atlas-supported version")),
            "unexpected probe: {probe:?}"
        );
    }

    #[test]
    fn schema_version_check_fails_with_schema_mismatch_code() {
        let dir = tempfile::tempdir().expect("tempdir");
        let content_path = dir.path().join("context.db");
        let content_path_str = content_path.to_string_lossy().to_string();
        drop(ContentStore::open(&content_path_str).expect("create content store"));
        bump_schema(&content_path, CONTENT_SCHEMA_VERSION + 1);

        let content = probe_content_schema(&content_path_str);
        let graph = SchemaProbe::missing("graph", "unused".to_owned(), GRAPH_SCHEMA_VERSION);
        let session = SchemaProbe::missing("session", "unused".to_owned(), SESSION_SCHEMA_VERSION);
        let check = schema_version_check(&graph, &content, &session);
        assert!(!check.ok);
        assert_eq!(check.name, "schema_version");
        assert_eq!(check.issue_code, Some("schema_mismatch"));
    }

    #[test]
    fn shared_check_names_are_stable() {
        assert_eq!(
            canonical_check_names(),
            &[
                "schema_version",
                "session_schema",
                "path_canonicality",
                "connection_mode"
            ]
        );
    }

    #[test]
    fn missing_stores_are_not_schema_failures() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("nope.db").to_string_lossy().to_string();
        assert!(probe_content_schema(&missing).ok);
        assert!(probe_session_schema(&missing).ok);
        assert!(probe_graph_schema(&missing).ok);
    }
}
