//! SQL migrations for session store database.

use atlas_db_utils::{Migration, MigrationSet};

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "001_initial_schema",
        up_sql: include_str!("migrations/001_initial_schema.sql"),
    },
    Migration {
        version: 2,
        name: "002_global_memory",
        up_sql: include_str!("migrations/002_global_memory.sql"),
    },
    Migration {
        version: 3,
        name: "003_decision_memory",
        up_sql: include_str!("migrations/003_decision_memory.sql"),
    },
    Migration {
        version: 4,
        name: "004_decision_memory_lookup_index",
        up_sql: include_str!("migrations/004_decision_memory_lookup_index.sql"),
    },
    Migration {
        version: 5,
        name: "005_decision_memory_fts",
        up_sql: include_str!("migrations/005_decision_memory_fts.sql"),
    },
];

pub const LATEST_VERSION: i32 = 5;

pub const MIGRATION_SET: MigrationSet = MigrationSet {
    db_kind: "session",
    migrations: MIGRATIONS,
};
