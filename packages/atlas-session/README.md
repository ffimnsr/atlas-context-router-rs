# atlas-session

Session identity, event ledger, and resume snapshots for Atlas context memory. Derives stable session IDs from repo + worktree + frontend anchors and persists session metadata across runs. Must not depend on the graph database or content storage.

## Public Surface

- **`SessionId`** — stable session identity
  - Derived from repo + worktree + frontend state
  - Deterministic and collision-resistant

- **`SessionStore`** — main store interface
  - `create()` — open or create session database
  - `record_event()` — append bounded event history
  - `resume_snapshot()` — build/consume session state snapshots
  - `get_session_status()` — metadata and event counts

- **Identity and lifecycle**
  - Session derivation from anchors
  - Resume-snapshot serialization
  - Per-session cleanup and retention

Each `SessionStore` instance owns one thread-confined SQLite connection per concurrency policy.
