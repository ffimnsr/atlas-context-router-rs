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
//! `rusqlite::Connection` is `!Send`. Each Atlas store struct wraps exactly one
//! `Connection` and must not be shared across threads. Rayon parallel sections
//! in `atlas-engine` only parse files (no `Connection` access); the store is
//! written sequentially after the rayon section completes. No pool is required
//! for the current single-writer architecture.
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

use atlas_core::{AtlasError, Result};
use rusqlite::Connection;

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
