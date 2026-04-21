//! SQL migrations for the content store database.

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
        sql: include_str!("migrations/002_trigram_and_vocabulary.sql"),
    },
    Migration {
        version: 3,
        sql: include_str!("migrations/003_retrieval_index_state.sql"),
    },
];
