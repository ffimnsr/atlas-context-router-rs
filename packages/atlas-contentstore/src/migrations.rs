//! SQL migrations for content store database.

use atlas_db_utils::{Migration, MigrationSet};

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "001_initial_schema",
        up_sql: include_str!("migrations/001_initial_schema.sql"),
    },
    Migration {
        version: 2,
        name: "002_trigram_and_vocabulary",
        up_sql: include_str!("migrations/002_trigram_and_vocabulary.sql"),
    },
    Migration {
        version: 3,
        name: "003_retrieval_index_state",
        up_sql: include_str!("migrations/003_retrieval_index_state.sql"),
    },
    Migration {
        version: 4,
        name: "004_chunk_id",
        up_sql: include_str!("migrations/004_chunk_id.sql"),
    },
    Migration {
        version: 5,
        name: "005_source_identity",
        up_sql: include_str!("migrations/005_source_identity.sql"),
    },
];

pub const LATEST_VERSION: i32 = 5;

pub const MIGRATION_SET: MigrationSet = MigrationSet {
    db_kind: "context",
    migrations: MIGRATIONS,
};
