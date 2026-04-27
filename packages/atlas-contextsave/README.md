# atlas-contextsave

Context save/restore coordinator for Atlas context memory integration. Composes `atlas-contentstore` and `atlas-session` to provide higher-level save and restore operations. This crate must not depend on the graph database.

## Public Surface

- **`SaveContext`** — unified save/restore coordinator
  - `save_artifact()` — route to content store with session metadata
  - `restore_snapshot()` — rebuild session and content state
  - `search_across_sessions()` — query saved artifacts with filters
  - `cleanup()` — age-based retention and lifecycle

- **Reexports**
  - `ContentStore` — artifact storage interface
  - `SessionId` — stable session identity
  - `OutputRouting` — transport-specific output control

Higher-level composition layer keeping graph database decoupled.
