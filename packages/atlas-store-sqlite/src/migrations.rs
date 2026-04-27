//! All SQL DDL for atlas graph schema, applied via shared migration runner.

use atlas_db_utils::{Migration, MigrationSet};

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "001_initial_schema",
        up_sql: include_str!("migrations/001_initial_schema.sql"),
    },
    Migration {
        version: 2,
        name: "002_flow_and_community_tables",
        up_sql: include_str!("migrations/002_flow_and_community_tables.sql"),
    },
    Migration {
        version: 3,
        name: "003_retrieval_chunks",
        up_sql: include_str!("migrations/003_retrieval_chunks.sql"),
    },
    Migration {
        version: 4,
        name: "004_fix_flow_memberships_and_community_nodes",
        up_sql: include_str!("migrations/004_fix_flow_memberships_and_community_nodes.sql"),
    },
    Migration {
        version: 5,
        name: "005_file_owners",
        up_sql: include_str!("migrations/005_file_owners.sql"),
    },
    Migration {
        version: 6,
        name: "006_graph_build_state",
        up_sql: include_str!("migrations/006_graph_build_state.sql"),
    },
    Migration {
        version: 7,
        name: "007_graph_build_budget_state",
        up_sql: include_str!("migrations/007_graph_build_budget_state.sql"),
    },
    Migration {
        version: 8,
        name: "008_history_tables",
        up_sql: include_str!("migrations/008_history_tables.sql"),
    },
    Migration {
        version: 9,
        name: "009_historical_file_graphs",
        up_sql: include_str!("migrations/009_historical_file_graphs.sql"),
    },
    Migration {
        version: 10,
        name: "010_history_lifecycle",
        up_sql: include_str!("migrations/010_history_lifecycle.sql"),
    },
    Migration {
        version: 11,
        name: "011_history_indexed_ref",
        up_sql: include_str!("migrations/011_history_indexed_ref.sql"),
    },
    Migration {
        version: 12,
        name: "012_snapshot_membership_blobs",
        up_sql: include_str!("migrations/012_snapshot_membership_blobs.sql"),
    },
    Migration {
        version: 13,
        name: "013_postprocess_state",
        up_sql: include_str!("migrations/013_postprocess_state.sql"),
    },
];

pub const LATEST_VERSION: i32 = 13;

pub const MIGRATION_SET: MigrationSet = MigrationSet {
    db_kind: "worldtree",
    migrations: MIGRATIONS,
};
