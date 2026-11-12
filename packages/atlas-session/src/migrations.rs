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
    Migration {
        version: 6,
        name: "006_durable_tasks",
        up_sql: include_str!("migrations/006_durable_tasks.sql"),
    },
    Migration {
        version: 7,
        name: "007_memories",
        up_sql: include_str!("migrations/007_memories.sql"),
    },
    Migration {
        version: 8,
        name: "008_durable_task_input_requests",
        up_sql: include_str!("migrations/008_durable_task_input_requests.sql"),
    },
    Migration {
        version: 9,
        name: "009_memory_decay_consolidation",
        up_sql: include_str!("migrations/009_memory_decay_consolidation.sql"),
    },
    Migration {
        version: 10,
        name: "010_feedback_records",
        up_sql: include_str!("migrations/010_feedback_records.sql"),
    },
    Migration {
        version: 11,
        name: "011_memoir_graph",
        up_sql: include_str!("migrations/011_memoir_graph.sql"),
    },
    Migration {
        version: 12,
        name: "012_overview_memory",
        up_sql: include_str!("migrations/012_overview_memory.sql"),
    },
    Migration {
        version: 13,
        name: "013_memory_ops_and_fts",
        up_sql: include_str!("migrations/013_memory_ops_and_fts.sql"),
    },
    Migration {
        version: 14,
        name: "014_experience_edges",
        up_sql: include_str!("migrations/014_experience_edges.sql"),
    },
    Migration {
        version: 15,
        name: "015_memory_embeddings",
        up_sql: include_str!("migrations/015_memory_embeddings.sql"),
    },
];

pub const LATEST_VERSION: i32 = 15;

pub const MIGRATION_SET: MigrationSet = MigrationSet {
    db_kind: "session",
    migrations: MIGRATIONS,
};
