# atlas-db-utils

Shared SQLite connection helpers (PRAGMAs, application_id, VACUUM) for Atlas stores. Provides the canonical PRAGMA set so every Atlas SQLite connection (`worldtree.db`, `context.db`, `session.db`) applies consistent configuration immediately after opening.

## Public Surface

- **`apply_atlas_pragmas()`** — apply canonical settings to all Atlas connections
  - WAL mode for concurrent readers during writes
  - Foreign key enforcement
  - Memory-mapped I/O and caching tuning
  - Busy timeout and checkpoint configuration

- **Connection validation**
  - Application ID checking for schema verification
  - Integrity checks and recovery policies

Ensures the three Atlas SQLite stores cannot drift; defines once, used everywhere.
