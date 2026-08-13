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

- **Shared memory model (ICM-A)**
  - `MemoryImportance` / `MemoryScope` — strict enums with exact values
  - `MemoryRecord` / `NewMemory` — one record shape shared by CLI and MCP
  - `memories` table — continuity-side persistence, validated by `atlas db check`

- **Memory curation (ICM-B)**
  - `MemoryDecayPolicy` — retention days per importance, critical protected by default
  - `decay_reports()` / `stale_memories()` / `prune_memories()` — score, list, and prune without touching saved-context artifacts
  - `memory_health()` — deterministic stale/duplicated/orphaned/oversized/noisy findings
  - `consolidate_memories()` — deterministic grouping, superseded markers, and `memory_supersessions` link rows

- **Feedback records (ICM-C)**
  - `NewFeedback` / `FeedbackRecord` — predicted vs actual corrections with symbol/file/kind context
  - `search_feedback()` — FTS5 over predicted/actual/correction/symbol/file with LIKE fallback
  - `feedback_stats()` / `feedback_matching()` — deterministic summaries and confidence-adjustment evidence

- **Identity and lifecycle**
  - Session derivation from anchors
  - Resume-snapshot serialization
  - Per-session cleanup and retention

Each `SessionStore` instance owns one thread-confined SQLite connection per concurrency policy.
