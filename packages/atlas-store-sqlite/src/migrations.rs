//! All SQL DDL for the atlas graph schema, applied via the migration runner.
//!
//! Migrations are identified by a monotonically increasing integer version.
//! Each entry is (version, sql). The runner applies any migration whose version
//! is greater than the stored `schema_version` in the `metadata` table.

pub struct Migration {
    pub version: i32,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: include_str!("migrations/001_initial_schema.sql"),
    },
    Migration {
        version: 2,
        sql: include_str!("migrations/002_flow_and_community_tables.sql"),
    },
];
