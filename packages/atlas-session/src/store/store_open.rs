use super::*;

impl SessionStore {
    pub fn open(path: &str) -> Result<Self> {
        Self::open_with_config(path, SessionStoreConfig::default())
    }

    pub fn open_in_repo(repo_root: impl AsRef<Path>) -> Result<Self> {
        let path = repo_root.as_ref().join(".atlas").join(DEFAULT_SESSION_DB);
        Self::open(path_to_str(&path)?)
    }

    pub fn open_with_config(path: &str, config: SessionStoreConfig) -> Result<Self> {
        if let Some(parent) = Path::new(path).parent() {
            std::fs::create_dir_all(parent)?;
        }

        let path_lock = session_db_open_lock(path);
        let _path_guard = path_lock.lock().expect("session db open lock poisoned");

        match Self::try_open(path, config.clone()) {
            Ok(store) => Ok(store),
            Err(e) => {
                if is_corruption_error(&e) {
                    quarantine_db(path);
                }
                Err(e)
            }
        }
    }

    fn try_open(path: &str, config: SessionStoreConfig) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;

        let atlas_dir = Path::new(path)
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| Path::new(".").join(".atlas"));
        let markdown = MarkdownStore::open(&atlas_dir)
            .map_err(|error| AtlasError::Other(format!("markdown store: {error}")))?;

        let mut store = Self {
            conn,
            config,
            markdown,
            db_path: std::path::PathBuf::from(path),
            _thread_bound: std::marker::PhantomData,
        };
        apply_atlas_pragmas(&store.conn)?;
        set_application_id(&store.conn, application_id::SESSION)?;
        store.migrate()?;
        if store.config.markdown_reconcile_on_open {
            match markdown_import::import_all(&store.markdown, &store.conn) {
                Ok(report) => {
                    if !report.issues.is_empty() {
                        tracing::warn!(
                            count = report.issues.len(),
                            issues = ?report.issues,
                            "markdown import skipped malformed records"
                        );
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "markdown import failed; derived rows may be stale");
                }
            }
            markdown_memory::reconcile_on_open(&store.markdown, &store.conn)?;
        }
        Ok(store)
    }

    pub fn migrate(&mut self) -> Result<()> {
        self.migrate_to(LATEST_VERSION)
    }

    pub fn migrate_to(&mut self, target_version: i32) -> Result<()> {
        let current: i32 = self
            .conn
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if target_version >= current {
            for migration in MIGRATION_SET.migrations.iter().filter(|migration| {
                migration.version > current && migration.version <= target_version
            }) {
                info!(
                    version = migration.version,
                    name = migration.name,
                    "applying session migration"
                );
            }
        } else {
            info!(
                current,
                target_version, "rebuilding session schema for downgrade"
            );
        }
        migrate_database_to(&mut self.conn, &MIGRATION_SET, target_version)
    }

    pub fn schema_version(&self) -> Result<i32> {
        self.conn
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| AtlasError::Db(e.to_string()))
    }

    /// Validates the shared memory schema (`memories` table, columns, and
    /// indexes). Returns an empty list when healthy; used by `atlas db check`.
    pub fn memory_schema_issues(&self) -> Vec<String> {
        memory_schema_issues(&self.conn, &self.markdown)
    }
}
