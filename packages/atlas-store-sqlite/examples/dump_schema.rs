//! Dump the worldtree schema so checked-in fixtures can be regenerated.
//!
//! Two modes mirror the two checked-in snapshot families:
//!
//! * `latest <version>` — same path as `Store::migrate`, includes framework
//!   tables; regenerate `schema.sql` and `schema_versions/<version>.sql`;
//! * `fixture <version>` — same path as the lifecycle-test helper, without
//!   framework tables; regenerate a historical `schema_versions/<version>.sql`.
//!
//! Usage:
//!
//! ```text
//! cargo run --quiet -p atlas-store-sqlite --example dump_schema -- latest 20 \
//!     > packages/atlas-store-sqlite/src/migrations/schema.sql
//! cargo run --quiet -p atlas-store-sqlite --example dump_schema -- fixture 20 \
//!     > packages/atlas-store-sqlite/src/migrations/schema_versions/020.sql
//! ```

use atlas_db_utils::{migrate_database_to, set_user_version};
use atlas_store_sqlite::migrations::MIGRATION_SET;
use rusqlite::Connection;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "latest".to_owned());
    let version = std::env::args()
        .nth(2)
        .map(|value| {
            value
                .parse::<i32>()
                .expect("target version must be an integer")
        })
        .unwrap_or_else(|| {
            MIGRATION_SET
                .migrations
                .last()
                .expect("at least one migration")
                .version
        });

    let dump = match mode.as_str() {
        "latest" => latest_dump(version)?,
        "fixture" => fixture_dump(version)?,
        other => return Err(format!("unknown mode '{other}'; use latest or fixture").into()),
    };
    print!("{dump}");
    Ok(())
}

/// Mirror `Store::migrate_to`: framework tables plus migrations.
fn latest_dump(target: i32) -> Result<String, Box<dyn std::error::Error>> {
    let mut conn = Connection::open_in_memory()?;
    atlas_db_utils::apply_atlas_pragmas(&conn)?;
    migrate_database_to(&mut conn, &MIGRATION_SET, target)?;
    schema_dump(&conn)
}

/// Mirror the store lifecycle-test helper `apply_migrations_through`:
/// `metadata` plus migrations, no framework tables.
fn fixture_dump(target: i32) -> Result<String, Box<dyn std::error::Error>> {
    let conn = Connection::open_in_memory()?;
    atlas_db_utils::apply_atlas_pragmas(&conn)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS metadata (
             key   TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );",
    )?;
    for migration in MIGRATION_SET
        .migrations
        .iter()
        .filter(|migration| migration.version <= target)
    {
        conn.execute_batch(migration.up_sql)?;
        conn.execute(
            "INSERT OR REPLACE INTO metadata (key, value) VALUES ('schema_version', ?1)",
            [migration.version.to_string()],
        )?;
    }
    set_user_version(&conn, target)?;
    schema_dump(&conn)
}

fn schema_dump(conn: &Connection) -> Result<String, Box<dyn std::error::Error>> {
    let user_version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let mut stmt = conn.prepare(
        "SELECT type, name, sql
         FROM sqlite_master
         WHERE sql IS NOT NULL
         ORDER BY CASE type
             WHEN 'table' THEN 0
             WHEN 'index' THEN 1
             WHEN 'trigger' THEN 2
             WHEN 'view' THEN 3
             ELSE 4
         END,
         name",
    )?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    let mut dump = vec![
        format!("-- schema_version: {user_version}"),
        format!("PRAGMA user_version = {user_version};"),
        String::new(),
    ];
    for (object_type, name, sql) in rows {
        dump.push(format!("-- {object_type}: {name}"));
        dump.push(format!("{};", normalize_schema_sql(&sql)));
        dump.push(String::new());
    }
    Ok(dump.join("\n").trim_end().to_string() + "\n")
}

fn normalize_schema_sql(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}
