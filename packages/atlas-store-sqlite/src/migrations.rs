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
    Migration {
        version: 3,
        sql: include_str!("migrations/003_retrieval_chunks.sql"),
    },
    Migration {
        version: 4,
        sql: include_str!("migrations/004_fix_flow_memberships_and_community_nodes.sql"),
    },
    Migration {
        version: 5,
        sql: include_str!("migrations/005_file_owners.sql"),
    },
    Migration {
        version: 6,
        sql: include_str!("migrations/006_graph_build_state.sql"),
    },
    Migration {
        version: 7,
        sql: include_str!("migrations/007_graph_build_budget_state.sql"),
    },
    Migration {
        version: 8,
        sql: include_str!("migrations/008_history_tables.sql"),
    },
    Migration {
        version: 9,
        sql: include_str!("migrations/009_historical_file_graphs.sql"),
    },
];
