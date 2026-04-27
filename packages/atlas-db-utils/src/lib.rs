//! Shared SQLite connection helpers for all Atlas stores.
//!
//! # Canonical PRAGMA set
//!
//! Every Atlas SQLite connection (`worldtree.db`, `context.db`, `session.db`)
//! must call [`apply_atlas_pragmas`] immediately after opening, before any DDL
//! or DML. The canonical set is defined **once here** so the three stores
//! cannot drift:
//!
//! | PRAGMA                    | Value          | Reason                                             |
//! |---------------------------|----------------|----------------------------------------------------|
//! | `journal_mode`            | `WAL`          | Write-ahead log; concurrent readers during writes  |
//! | `synchronous`             | `NORMAL`       | Durable enough for WAL; faster than `FULL`         |
//! | `foreign_keys`            | `ON`           | Enforce referential integrity at runtime           |
//! | `busy_timeout`            | `5000`         | 5 s retry window before `SQLITE_BUSY`              |
//! | `temp_store`              | `MEMORY`       | Temp tables/indexes in RAM, not disk               |
//! | `mmap_size`               | `268435456`    | 256 MB memory-mapped I/O for read-heavy paths      |
//! | `cache_size`              | `-32768`       | 32 MB page cache per connection                    |
//! | `wal_autocheckpoint`      | `400`          | Checkpoint every 400 pages; default 1000 can stall long-lived MCP daemons |
//! | `auto_vacuum`             | `INCREMENTAL`  | Enable incremental freelist reclaim (new DBs only) |
//!
//! `application_id` is **not** in the canonical set; call [`set_application_id`]
//! with the appropriate [`application_id`] constant per store.
//!
//! # Connection/thread policy
//!
//! Atlas uses one canonical ownership rule for SQLite connections:
//!
//! - each store instance owns exactly one `rusqlite::Connection`
//! - store structs are thread-confined and must not cross Rayon or worker-thread
//!   boundaries
//! - concurrent DB access, when needed, uses separate connections rather than
//!   shared ownership of one connection
//! - current architecture is single-writer per store instance; no read pool
//!   exists yet
//!
//! **Current connection mode (all stores):**
//! - parallel parse + sequential persistence: Rayon closures hash/read/parse
//!   files; all SQLite writes happen after parallel phases complete
//! - single-connection per store instance: one `Connection` per `Store`,
//!   `ContentStore`, and `SessionStore`
//! - separate-connection concurrency only: if a second reader is needed, open
//!   a second store instance — never share one `Connection` across threads
//! - read-pool layer reserved for future measured need: `r2d2_sqlite` or any
//!   equivalent connection pool has not been added; pooled graph reads are not
//!   implemented today
//!
//! **Non-goal for current architecture:** do not add `r2d2_sqlite` or any read
//! pool until contention is measured. WAL already allows one writer plus
//! multiple readers across separate connections; adding a pool before observing
//! `SQLITE_BUSY` pressure only adds complexity without benefit.
//!
//! **Future upgrade rule (when read concurrency is eventually added):**
//! - use separate checked-out connections per reader; never put one
//!   `Connection` behind `Arc<Mutex<_>>`, `RwLock<_>`, or similar shared
//!   wrappers
//! - preserve one write-owning connection per mutable store instance; route
//!   all writes through it exclusively
//! - keep write-ownership policy explicit before introducing mixed read/write
//!   pooling
//! - apply the canonical Atlas PRAGMAs and open flags to every pooled
//!   connection on checkout
//!
//! `rusqlite::Connection` is `!Send`, which matches this policy. `atlas-engine`
//! keeps SQLite work outside Rayon closures: parallel phases hash/read/parse
//! files, then sequential phases persist results through the owning store.
//! WAL permits concurrent reads during writes only across separate connections;
//! it does not make one `Connection` safe to share across threads.
//!
//! # VACUUM policy
//!
//! With `auto_vacuum=INCREMENTAL` enabled on new databases, call
//! [`incremental_vacuum`] after large deletes (graph rebuild, history prune) to
//! reclaim freelist pages without a full offline `VACUUM`. Existing databases
//! that were opened before this setting was added retain `auto_vacuum=NONE` and
//! require a full `VACUUM` to change mode; that is out of scope for runtime
//! operation — use `atlas doctor` to inspect freelist size and advise the
//! operator. The [`freelist_count`] and [`auto_vacuum_mode`] helpers feed into
//! that diagnostic surface.

use std::collections::BTreeSet;

use atlas_core::{AtlasError, Result};
use rusqlite::{Connection, params};

/// Well-known `application_id` values stamped on each Atlas database file so
/// `file(1)` and external SQLite tooling can identify them without reading the
/// schema.
///
/// Values are unique 4-byte big-endian integers stored in the SQLite
/// `application_id` header slot (bytes 60–63).
pub mod application_id {
    /// `worldtree.db` — main graph store (`ATLG` = `0x41_54_4C_47`).
    pub const WORLDTREE: i32 = 0x41544C47_u32 as i32;
    /// `context.db` — content/artifact store (`ATLC` = `0x41_54_4C_43`).
    pub const CONTEXT: i32 = 0x41544C43_u32 as i32;
    /// `session.db` — session event store (`ATLS` = `0x41_54_4C_53`).
    pub const SESSION: i32 = 0x41544C53_u32 as i32;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MigrationDirection {
    Up,
    Down,
}

impl MigrationDirection {
    fn as_str(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Migration {
    pub version: i32,
    pub name: &'static str,
    pub up_sql: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct MigrationSet {
    pub db_kind: &'static str,
    pub migrations: &'static [Migration],
}

/// Apply the canonical Atlas PRAGMA set to `conn`.
///
/// Must be called immediately after opening a connection, before any DDL or
/// DML. Some PRAGMAs (`auto_vacuum`, `journal_mode`) are only effective for
/// brand-new database files; existing databases retain their stored setting.
///
/// `application_id` is **not** applied here — call [`set_application_id`]
/// separately with the appropriate [`application_id`] constant.
pub fn apply_atlas_pragmas(conn: &Connection) -> Result<()> {
    let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
    // auto_vacuum must come before journal_mode for it to take effect on new DBs.
    for sql in &[
        "PRAGMA auto_vacuum=INCREMENTAL",
        "PRAGMA journal_mode=WAL",
        "PRAGMA synchronous=NORMAL",
        "PRAGMA foreign_keys=ON",
        "PRAGMA busy_timeout=5000",
        "PRAGMA temp_store=MEMORY",
        "PRAGMA mmap_size=268435456",
        "PRAGMA cache_size=-32768",
        "PRAGMA wal_autocheckpoint=400",
    ] {
        let mut stmt = conn.prepare(sql).map_err(db_err)?;
        let mut rows = stmt.query([]).map_err(db_err)?;
        while rows.next().map_err(db_err)?.is_some() {}
    }
    Ok(())
}

/// Stamp `application_id` on `conn`.
///
/// Use one of the [`application_id`] constants. This is idempotent: setting
/// the same id repeatedly is a no-op in SQLite.
pub fn set_application_id(conn: &Connection, id: i32) -> Result<()> {
    let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
    // application_id must be formatted into the SQL because rusqlite's
    // `execute` binding is not supported for PRAGMA value slots.
    let sql = format!("PRAGMA application_id={id}");
    let mut stmt = conn.prepare(&sql).map_err(db_err)?;
    let mut rows = stmt.query([]).map_err(db_err)?;
    while rows.next().map_err(db_err)?.is_some() {}
    Ok(())
}

/// Set `PRAGMA user_version` to `version` so `sqlite3 <db> "PRAGMA
/// user_version"` returns the current schema version without reading the
/// `metadata` table.
///
/// Call this at the end of each store's `migrate()`, passing the highest
/// migration version that was applied.
pub fn set_user_version(conn: &Connection, version: i32) -> Result<()> {
    let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
    let sql = format!("PRAGMA user_version={version}");
    let mut stmt = conn.prepare(&sql).map_err(db_err)?;
    let mut rows = stmt.query([]).map_err(db_err)?;
    while rows.next().map_err(db_err)?.is_some() {}
    Ok(())
}

/// Apply Atlas framework metadata plus all pending migrations for `set`.
///
/// Each open stamps `atlas_provenance` and `metadata` with current Atlas
/// binary version. Upgrades append rows into `schema_migrations`.
pub fn migrate_database(conn: &mut Connection, set: &MigrationSet) -> Result<()> {
    migrate_database_to(conn, set, latest_version(set))
}

/// Move database schema to `target_version`.
///
/// Upgrades run migration `up_sql` in order. Downgrades rebuild user schema
/// from scratch to match `target_version`, preserving only framework tables
/// (`metadata`, `schema_migrations`, `atlas_provenance`).
pub fn migrate_database_to(
    conn: &mut Connection,
    set: &MigrationSet,
    target_version: i32,
) -> Result<()> {
    ensure_framework_tables(conn)?;
    stamp_provenance(conn, set.db_kind)?;

    let latest = latest_version(set);
    if target_version < 0 || target_version > latest {
        return Err(AtlasError::Db(format!(
            "unsupported target schema version {target_version}; latest is {latest}"
        )));
    }

    let current = current_schema_version(conn)?;
    if current > latest {
        return Err(AtlasError::Db(format!(
            "database schema version {current} newer than atlas-supported version {latest}"
        )));
    }

    backfill_history_if_missing(conn, set, current)?;

    match current.cmp(&target_version) {
        std::cmp::Ordering::Less => migrate_up(conn, set, current, target_version)?,
        std::cmp::Ordering::Greater => rebuild_down(conn, set, current, target_version)?,
        std::cmp::Ordering::Equal => {}
    }

    set_user_version(conn, target_version)?;
    write_metadata(conn, "schema_version", &target_version.to_string())?;
    write_metadata(conn, "atlas_last_opened_by", &atlas_version_banner())?;
    Ok(())
}

/// Run `PRAGMA foreign_key_check` and return the number of integrity
/// violations found.
///
/// Returns `Ok(0)` for a healthy database. Used by `atlas doctor` /
/// `atlas db_check` for corruption detection. **Not** called on every open
/// because it is O(N) for large graphs.
pub fn foreign_key_check(conn: &Connection) -> Result<u64> {
    let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
    let mut stmt = conn.prepare("PRAGMA foreign_key_check").map_err(db_err)?;
    let mut rows = stmt.query([]).map_err(db_err)?;
    let mut count: u64 = 0;
    while rows.next().map_err(db_err)?.is_some() {
        count += 1;
    }
    Ok(count)
}

/// Run `PRAGMA incremental_vacuum(pages)` to reclaim up to `pages` freelist
/// pages. Safe to call on any Atlas DB; a no-op when `auto_vacuum=NONE`.
///
/// Call after large deletes (graph rebuild, history prune). Use `0` to reclaim
/// all available freelist pages.
pub fn incremental_vacuum(conn: &Connection, pages: u32) -> Result<()> {
    conn.execute_batch(&format!("PRAGMA incremental_vacuum({pages})"))
        .map_err(|e| AtlasError::Db(e.to_string()))
}

/// Return the current freelist page count — pages SQLite has allocated but
/// not yet reclaimed. Feeds into `atlas doctor` diagnostics.
pub fn freelist_count(conn: &Connection) -> Result<i64> {
    conn.query_row("PRAGMA freelist_count", [], |r| r.get(0))
        .map_err(|e| AtlasError::Db(e.to_string()))
}

/// Return the `auto_vacuum` mode: `0` = none, `1` = full, `2` = incremental.
/// Feeds into `atlas doctor` diagnostics.
pub fn auto_vacuum_mode(conn: &Connection) -> Result<i64> {
    conn.query_row("PRAGMA auto_vacuum", [], |r| r.get(0))
        .map_err(|e| AtlasError::Db(e.to_string()))
}

fn latest_version(set: &MigrationSet) -> i32 {
    set.migrations.last().map(|migration| migration.version).unwrap_or(0)
}

fn ensure_framework_tables(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS metadata (
             key   TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS schema_migrations (
             id            INTEGER PRIMARY KEY,
             version       INTEGER NOT NULL,
             name          TEXT    NOT NULL,
             direction     TEXT    NOT NULL CHECK(direction IN ('up', 'down')),
             atlas_version TEXT    NOT NULL,
             applied_at    TEXT    NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_schema_migrations_version_id
             ON schema_migrations(version, id);
         CREATE TABLE IF NOT EXISTS atlas_provenance (
             singleton_key INTEGER PRIMARY KEY CHECK(singleton_key = 1),
             db_kind       TEXT    NOT NULL,
             created_by    TEXT    NOT NULL,
             created_at    TEXT    NOT NULL,
             last_opened_by TEXT   NOT NULL,
             last_opened_at TEXT   NOT NULL
         );",
    )
    .map_err(|e| AtlasError::Db(e.to_string()))
}

fn stamp_provenance(conn: &Connection, db_kind: &str) -> Result<()> {
    let banner = atlas_version_banner();
    conn.execute(
        "INSERT INTO atlas_provenance(
             singleton_key,
             db_kind,
             created_by,
             created_at,
             last_opened_by,
             last_opened_at
         ) VALUES (
             1,
             ?1,
             ?2,
             strftime('%Y-%m-%dT%H:%M:%SZ', 'now'),
             ?2,
             strftime('%Y-%m-%dT%H:%M:%SZ', 'now')
         )
         ON CONFLICT(singleton_key) DO UPDATE SET
             db_kind = excluded.db_kind,
             last_opened_by = excluded.last_opened_by,
             last_opened_at = excluded.last_opened_at",
        params![db_kind, banner],
    )
    .map_err(|e| AtlasError::Db(e.to_string()))?;
    write_metadata(conn, "atlas_db_kind", db_kind)?;
    insert_metadata_if_absent(conn, "atlas_created_by", &banner)?;
    write_metadata(conn, "atlas_last_opened_by", &banner)?;
    Ok(())
}

fn current_schema_version(conn: &Connection) -> Result<i32> {
    conn.query_row(
        "SELECT CAST(value AS INTEGER) FROM metadata WHERE key = 'schema_version'",
        [],
        |row| row.get(0),
    )
    .or_else(|error| match error {
        rusqlite::Error::QueryReturnedNoRows => Ok(0),
        other => Err(other),
    })
    .map_err(|e| AtlasError::Db(e.to_string()))
}

fn backfill_history_if_missing(conn: &Connection, set: &MigrationSet, current: i32) -> Result<()> {
    let existing: i64 = conn
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| row.get(0))
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    if existing > 0 || current == 0 {
        return Ok(());
    }

    let banner = atlas_version_banner();
    for migration in set.migrations.iter().filter(|migration| migration.version <= current) {
        record_migration(conn, migration, MigrationDirection::Up, &banner)?;
    }
    Ok(())
}

fn migrate_up(
    conn: &mut Connection,
    set: &MigrationSet,
    current: i32,
    target: i32,
) -> Result<()> {
    let banner = atlas_version_banner();
    for migration in set
        .migrations
        .iter()
        .filter(|migration| migration.version > current && migration.version <= target)
    {
        conn.execute_batch(migration.up_sql)
            .map_err(|e| AtlasError::Db(format!("migration {} ({}): {e}", migration.version, migration.name)))?;
        write_metadata(conn, "schema_version", &migration.version.to_string())?;
        record_migration(conn, migration, MigrationDirection::Up, &banner)?;
    }
    Ok(())
}

fn rebuild_down(
    conn: &mut Connection,
    set: &MigrationSet,
    current: i32,
    target: i32,
) -> Result<()> {
    let mut target_conn = Connection::open_in_memory().map_err(|e| AtlasError::Db(e.to_string()))?;
    ensure_framework_tables(&target_conn)?;
    migrate_up(&mut target_conn, set, 0, target)?;

    let schema_sql = export_user_schema_sql(&target_conn)?;
    let banner = atlas_version_banner();

    conn.execute_batch("PRAGMA foreign_keys=OFF")
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    drop_user_objects(conn)?;
    if !schema_sql.trim().is_empty() {
        conn.execute_batch(&schema_sql)
            .map_err(|e| AtlasError::Db(format!("rebuild schema v{target}: {e}")))?;
    }
    conn.execute_batch("PRAGMA foreign_keys=ON")
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    for version in ((target + 1)..=current).rev() {
        let migration = set
            .migrations
            .iter()
            .find(|migration| migration.version == version)
            .ok_or_else(|| AtlasError::Db(format!("missing migration definition for version {version}")))?;
        record_migration(conn, migration, MigrationDirection::Down, &banner)?;
    }
    Ok(())
}

fn export_user_schema_sql(conn: &Connection) -> Result<String> {
    let shadow_tables = shadow_table_names(conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT type, name, sql
             FROM sqlite_master
             WHERE sql IS NOT NULL
               AND name NOT LIKE 'sqlite_%'
             ORDER BY CASE type
                 WHEN 'table' THEN 0
                 WHEN 'index' THEN 1
                 WHEN 'trigger' THEN 2
                 WHEN 'view' THEN 3
                 ELSE 4
             END,
             name",
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| AtlasError::Db(e.to_string()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    let sql = rows
        .into_iter()
        .filter(|(_, name, _)| !is_framework_object(name) && !shadow_tables.contains(name))
        .map(|(_, _, sql)| format!("{sql};"))
        .collect::<Vec<_>>()
        .join("\n\n");
    Ok(sql)
}

fn drop_user_objects(conn: &Connection) -> Result<()> {
    let shadow_tables = shadow_table_names(conn)?;
    let mut stmt = conn
        .prepare(
            "SELECT type, name
             FROM sqlite_master
             WHERE name NOT LIKE 'sqlite_%'
             ORDER BY CASE type
                 WHEN 'trigger' THEN 0
                 WHEN 'index' THEN 1
                 WHEN 'view' THEN 2
                 WHEN 'table' THEN 3
                 ELSE 4
             END,
             name",
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    let objects = stmt
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
        .map_err(|e| AtlasError::Db(e.to_string()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    for (kind, name) in objects {
        if is_framework_object(&name) || shadow_tables.contains(&name) {
            continue;
        }
        let ddl = match kind.as_str() {
            "table" => format!("DROP TABLE IF EXISTS \"{}\"", name.replace('"', "\"\"")),
            "index" => format!("DROP INDEX IF EXISTS \"{}\"", name.replace('"', "\"\"")),
            "trigger" => format!("DROP TRIGGER IF EXISTS \"{}\"", name.replace('"', "\"\"")),
            "view" => format!("DROP VIEW IF EXISTS \"{}\"", name.replace('"', "\"\"")),
            _ => continue,
        };
        conn.execute_batch(&ddl)
            .map_err(|e| AtlasError::Db(format!("drop {kind} {name}: {e}")))?;
    }
    Ok(())
}

fn shadow_table_names(conn: &Connection) -> Result<BTreeSet<String>> {
    let mut stmt = conn
        .prepare("PRAGMA table_list")
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    stmt.query_map([], |row| Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?)))
        .map_err(|e| AtlasError::Db(e.to_string()))?
        .filter_map(|row| row.ok())
        .filter(|(_, table_type)| table_type == "shadow")
        .map(|(name, _)| Ok(name))
        .collect::<std::result::Result<BTreeSet<_>, AtlasError>>()
}

fn is_framework_object(name: &str) -> bool {
    matches!(name, "metadata" | "schema_migrations" | "atlas_provenance")
}

fn record_migration(
    conn: &Connection,
    migration: &Migration,
    direction: MigrationDirection,
    atlas_version: &str,
) -> Result<()> {
    conn.execute(
        "INSERT INTO schema_migrations(version, name, direction, atlas_version, applied_at)
         VALUES (?1, ?2, ?3, ?4, strftime('%Y-%m-%dT%H:%M:%SZ', 'now'))",
        params![
            migration.version,
            migration.name,
            direction.as_str(),
            atlas_version
        ],
    )
    .map_err(|e| AtlasError::Db(e.to_string()))?;
    Ok(())
}

fn insert_metadata_if_absent(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO metadata(key, value) VALUES (?1, ?2)",
        params![key, value],
    )
    .map_err(|e| AtlasError::Db(e.to_string()))?;
    Ok(())
}

fn write_metadata(conn: &Connection, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO metadata(key, value) VALUES (?1, ?2)",
        params![key, value],
    )
    .map_err(|e| AtlasError::Db(e.to_string()))?;
    Ok(())
}

fn atlas_version_banner() -> String {
    format!("atlas v{}", env!("CARGO_PKG_VERSION"))
}
