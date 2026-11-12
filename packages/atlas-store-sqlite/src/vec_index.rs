//! Optional `sqlite-vec` acceleration for retrieval-chunk vectors.
//!
//! The `retrieval_chunks.embedding` BLOB remains the source of truth. This
//! module maintains a lazy, per-dimension `vec0` index (`retrieval_vec_<dim>`)
//! so KNN queries use the C/SIMD path when the statically linked extension is
//! available. Every operation degrades to the caller's BLOB scan on error; a
//! missing or stale index is rebuilt from the BLOB column.
//!
//! Scoped queries pass the exact in-scope chunk ids through `rowid IN (...)`, so
//! subpath, kind, language, and multi-repo filters keep the accelerated path
//! without approximating the scope as vec0 constraints.

use std::sync::Once;

use rusqlite::{Connection, params};

const TABLE_PREFIX: &str = "retrieval_vec_";

/// Maximum number of chunk ids bound into one KNN query.
///
/// The bundled SQLite allows 32,766 statement variables; 8,000 keeps each
/// statement well clear of that limit while bounding the SQL text size.
const MAX_IDS_PER_QUERY: usize = 8_000;

/// sqlite-vec rejects `k` values above this cap.
const MAX_KNN_K: usize = 4_096;

/// Register sqlite-vec as a process-wide SQLite auto-extension.
///
/// Must run before opening a connection. Registration applies to future
/// connections only, so callers invoke this before `Connection::open*`.
pub(crate) fn register_extension() {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| {
        // SAFETY: `sqlite3_auto_extension` stores a process-global function
        // pointer. `sqlite3_vec_init` is statically linked by the sqlite-vec
        // crate and has the required SQLite extension entrypoint signature.
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute::<
                *const (),
                unsafe extern "C" fn(
                    *mut rusqlite::ffi::sqlite3,
                    *mut *mut i8,
                    *const rusqlite::ffi::sqlite3_api_routines,
                ) -> i32,
            >(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
    });
}

pub(crate) fn f32_blob(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn table_name(dim: usize) -> String {
    format!("{TABLE_PREFIX}{dim}")
}

fn create_table(conn: &Connection, name: &str, dim: usize) -> rusqlite::Result<()> {
    conn.execute_batch(&format!(
        "CREATE VIRTUAL TABLE {name} USING vec0(
             source_repo_id text partition key,
             embedding float[{dim}] distance_metric=cosine
         )"
    ))
}

fn table_exists(conn: &Connection, name: &str) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [name],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count > 0)
}

fn embedded_count(conn: &Connection, dim: usize, repo_id: Option<&str>) -> rusqlite::Result<i64> {
    match repo_id {
        Some(repo_id) => conn.query_row(
            "SELECT COUNT(*) FROM retrieval_chunks
             WHERE embedding IS NOT NULL AND length(embedding) = ?1
               AND source_repo_id = ?2",
            params![dim as i64 * 4, repo_id],
            |row| row.get(0),
        ),
        None => conn.query_row(
            "SELECT COUNT(*) FROM retrieval_chunks
             WHERE embedding IS NOT NULL AND length(embedding) = ?1",
            [dim as i64 * 4],
            |row| row.get(0),
        ),
    }
}

fn rebuild(conn: &Connection, name: &str, dim: usize) -> rusqlite::Result<()> {
    conn.execute(&format!("DELETE FROM {name}"), [])?;
    conn.execute(
        &format!(
            "INSERT INTO {name} (rowid, source_repo_id, embedding)
             SELECT id, source_repo_id, vec_f32(embedding) FROM retrieval_chunks
             WHERE embedding IS NOT NULL AND length(embedding) = ?1"
        ),
        [dim as i64 * 4],
    )?;
    Ok(())
}

/// Ensure the per-dimension vec0 table exists and mirrors the BLOB column.
fn ensure_index(conn: &Connection, dim: usize) -> rusqlite::Result<()> {
    let name = table_name(dim);
    if !table_exists(conn, &name)? {
        create_table(conn, &name, dim)?;
        rebuild(conn, &name, dim)?;
        return Ok(());
    }

    let expected = embedded_count(conn, dim, None)?;
    let actual: i64 = conn.query_row(&format!("SELECT COUNT(*) FROM {name}"), [], |row| {
        row.get(0)
    })?;
    if actual != expected {
        rebuild(conn, &name, dim)?;
    }
    Ok(())
}

/// Insert or replace one embedding in the vec0 index.
///
/// Errors are intentionally returned to the caller so vector persistence can
/// still succeed via the BLOB column and queries can fall back.
pub(crate) fn upsert(conn: &Connection, chunk_id: i64, blob: &[u8]) -> rusqlite::Result<()> {
    if blob.is_empty() || !blob.len().is_multiple_of(4) {
        return Ok(());
    }
    let dim = blob.len() / 4;
    let name = table_name(dim);
    if !table_exists(conn, &name)? {
        create_table(conn, &name, dim)?;
    }
    let source_repo_id: String = conn.query_row(
        "SELECT source_repo_id FROM retrieval_chunks WHERE id = ?1",
        [chunk_id],
        |row| row.get(0),
    )?;
    conn.execute(&format!("DELETE FROM {name} WHERE rowid = ?1"), [chunk_id])?;
    conn.execute(
        &format!(
            "INSERT INTO {name} (rowid, source_repo_id, embedding)
             VALUES (?1, ?2, vec_f32(?3))"
        ),
        params![chunk_id, source_repo_id, blob],
    )?;
    Ok(())
}

fn vector_tables(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master
         WHERE type = 'table' AND name LIKE 'retrieval_vec_%'",
    )?;
    let names = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .filter_map(Result::ok)
        .collect();
    Ok(names)
}

/// Remove the given chunk ids from every vec0 index table.
pub(crate) fn delete_ids(conn: &Connection, ids: &[i64]) -> rusqlite::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    for table in vector_tables(conn)? {
        for batch in ids.chunks(400) {
            let placeholders = vec!["?"; batch.len()].join(", ");
            conn.execute(
                &format!("DELETE FROM {table} WHERE rowid IN ({placeholders})"),
                rusqlite::params_from_iter(batch.iter()),
            )?;
        }
    }
    Ok(())
}

/// Delete vec0 rows for one repo-scoped node before its chunks are removed.
pub(crate) fn delete_for_node(
    conn: &Connection,
    source_repo_id: &str,
    node_qn: &str,
) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(
        "SELECT id FROM retrieval_chunks
         WHERE source_repo_id = ?1 AND node_qn = ?2",
    )?;
    let ids: Vec<i64> = stmt
        .query_map(params![source_repo_id, node_qn], |row| row.get(0))?
        .filter_map(Result::ok)
        .collect();
    delete_ids(conn, &ids)
}

/// Run a KNN query through vec0.
///
/// `repo_id` restricts the search to one partition. `chunk_ids` restricts it to
/// an exact set of in-scope chunk rows, which lets scoped queries (subpath,
/// directory, exclusions, kinds, languages, multi-repo) keep the accelerated
/// path: SQL resolves the scope to chunk ids, and vec0 ranks only those rows.
///
/// Returns `None` when the extension, index, or a KNN statement is unavailable,
/// so callers keep the exact BLOB-scan path. Scores are cosine similarity
/// (`1 - distance`).
pub(crate) fn try_knn(
    conn: &Connection,
    query: &[f32],
    limit: usize,
    repo_id: Option<&str>,
    chunk_ids: Option<&[i64]>,
) -> Option<(Vec<(String, f64)>, usize)> {
    try_knn_batched(conn, query, limit, repo_id, chunk_ids, MAX_IDS_PER_QUERY)
}

/// [`try_knn`] with an explicit batch size for the `rowid IN (...)` filter.
///
/// Exposed so tests can exercise the batch-merge path without seeding tens of
/// thousands of chunks; production callers always use [`MAX_IDS_PER_QUERY`].
pub(crate) fn try_knn_batched(
    conn: &Connection,
    query: &[f32],
    limit: usize,
    repo_id: Option<&str>,
    chunk_ids: Option<&[i64]>,
    batch_size: usize,
) -> Option<(Vec<(String, f64)>, usize)> {
    if query.is_empty() || limit == 0 || limit > MAX_KNN_K {
        return None;
    }
    let dim = query.len();
    ensure_index(conn, dim).ok()?;
    let name = table_name(dim);
    let k = limit.min(MAX_KNN_K);

    let Some(chunk_ids) = chunk_ids else {
        let candidates = embedded_count(conn, dim, repo_id).ok()? as usize;
        let hits = knn_query(conn, &name, query, k, repo_id, None)?;
        return Some((hits, candidates));
    };

    let mut ids = chunk_ids.to_vec();
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        return Some((Vec::new(), 0));
    }

    // Top-`k` of a union equals the top-`k` of the union of each batch's
    // top-`k`, so splitting a large id set stays exact.
    let mut hits = Vec::new();
    for batch in ids.chunks(batch_size.max(1)) {
        hits.extend(knn_query(conn, &name, query, k, repo_id, Some(batch))?);
    }
    hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    hits.truncate(limit);
    Some((hits, ids.len()))
}

/// Execute one KNN statement against a vec0 table.
fn knn_query(
    conn: &Connection,
    name: &str,
    query: &[f32],
    k: usize,
    repo_id: Option<&str>,
    chunk_ids: Option<&[i64]>,
) -> Option<Vec<(String, f64)>> {
    let mut sql = format!(
        "SELECT rc.node_qn, v.distance
         FROM {name} v
         JOIN retrieval_chunks rc ON rc.id = v.rowid
         WHERE v.embedding MATCH vec_f32(?) AND k = ?"
    );
    let mut bind: Vec<rusqlite::types::Value> = vec![f32_blob(query).into(), (k as i64).into()];
    if let Some(repo_id) = repo_id {
        sql.push_str(" AND v.source_repo_id = ?");
        bind.push(repo_id.to_owned().into());
    }
    if let Some(chunk_ids) = chunk_ids {
        let placeholders = vec!["?"; chunk_ids.len()].join(", ");
        sql.push_str(&format!(" AND v.rowid IN ({placeholders})"));
        bind.extend(chunk_ids.iter().map(|id| (*id).into()));
    }

    let mut stmt = conn.prepare(&sql).ok()?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(bind.iter()), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })
        .ok()?;
    let mut hits: Vec<(String, f64)> = rows
        .collect::<rusqlite::Result<Vec<_>>>()
        .ok()?
        .into_iter()
        .map(|(qn, distance)| (qn, 1.0 - distance))
        .collect();
    hits.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    Some(hits)
}
