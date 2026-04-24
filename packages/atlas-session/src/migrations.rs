//! SQL migrations for session store database.

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
        sql: include_str!("migrations/002_global_memory.sql"),
    },
    Migration {
        version: 3,
        sql: include_str!("migrations/003_decision_memory.sql"),
    },
    Migration {
        version: 4,
        sql: include_str!("migrations/004_decision_memory_lookup_index.sql"),
    },
    Migration {
        version: 5,
        sql: include_str!("migrations/005_decision_memory_fts.sql"),
    },
];
