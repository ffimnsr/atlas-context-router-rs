# Plan: Markdown Continuity Storage

Status: design approved; implementation pending. Surfaces: `packages/atlas-session`, `packages/atlas-contentstore`, `packages/atlas-mcp`, `packages/atlas-cli`, `packages/atlas-adapters`.

## Motivation

SQLite is the wrong home for durable, human-readable continuity records:

- corruption risk — a damaged `session.db` can take memories, decisions, and overview
  records with it, and repair means out-of-band tooling;
- no version control — binary `.db` files do not diff or review;
- opaque state — agents and humans cannot inspect what the agent knows without
  opening a database.

OpenViking demonstrates the alternative: context as navigable files, sessions as
readable documents, directory structure as retrieval context. Atlas adopts that
paradigm for its continuity layer while keeping the code graph in SQLite where
relational queries earn their keep.

## Resolved decisions

1. **Committed to repo git:** `.atlas/memories/`, `.atlas/decisions/`,
   `.atlas/feedback/`, `.atlas/memoir/`, `.atlas/overview/`, `.atlas/artifacts/`.
   **Not committed (local-only):** `.atlas/sessions/` (events and resume contain
   tool output transcripts), `.atlas/tasks/`, `.atlas/experience/`, all `*.db`
   files, lock files.
2. **Global-scope memories stay in the workspace `.atlas/`** with
   `scope: global` frontmatter, matching today's per-repo storage; no home-dir root.
3. **Session events ledger is markdown** (`events.md`), chosen because the ledger
   is written once per hook event and read rarely (wake-up snapshot, compaction).

## Principles

1. Markdown files under `.atlas/` are the **source of truth** for durable
   continuity records.
2. SQLite files under `.atlas/` are **derived, disposable indexes** only. A corrupt
   or deleted index is regenerated from markdown by `atlas index rebuild`. No data
   loss is possible by construction.
3. `worldtree.db` is untouched: static code graph facts stay relational.
4. MCP and CLI tool contracts stay byte-stable. Only the backend swaps.
5. Reads never write. Truth files change only when their semantic content
   changes; access bookkeeping and computed state live in the derived index.

## Target layout

```
.atlas/
├── memories/                  # committed — memory map
│   └── <topic-slug>/
│       └── <memory-id>.md
├── decisions/                 # committed
│   └── <decision-id>.md
├── feedback/                  # committed
│   └── <record-id>.md
├── memoir/                    # committed
│   └── <graph-name>/
│       ├── _graph.md
│       └── <concept-slug>.md
├── overview/                  # committed
│   └── <subject_kind>/
│       └── <subject_id>.md
├── sessions/                  # local-only
│   └── <session-id>/
│       ├── events.md
│       └── resume.md
├── tasks/                     # local-only
│   └── <task-id>.md
├── experience/                # local-only
│   └── <edge-slug>.md
├── artifacts/                 # committed — moved out of context.db
│   └── <source-id>.md
├── locks/                     # local-only — central advisory lock files
├── tmp/                       # local-only — in-flight atomic-write temps
└── *.db                       # derived indexes, local-only
```

No generated files live inside the committed directories; derived state always
lives in a `*.db` index. This keeps git diffs meaningful.

## Record formats

### Memory entry (`.atlas/memories/<topic-slug>/<id>.md`)

```markdown
---
id: fa1b8f2d5e66ab81f14ec09af06d5fa41e47a92116ae6d74887a7225493bed73
title: Storage direction: markdown truth, sqlite derived cache
topic: architecture
scope: project            # project | session | frontend | global
importance: high          # critical | high | normal | low
tags: [storage, sqlite, migration]
session_id: null
frontend: null
created_at: 2026-11-19T13:26:36Z
updated_at: 2026-11-19T13:26:36Z
source_id: fb86b7e8c8452f7f76e0ad2bbd8ad1a7
links:
  - { id: <other-memory-id>, relation: refines }
  - { id: <older-memory-id>, relation: supersedes }
metadata: { category: decision }
---
Body text.
```

Column mapping from the `memories` table: all columns become frontmatter fields
except `last_accessed_at` and `decay_score`, which move to the derived
`memory_access` index so reads never rewrite files; `body` becomes the file
body; `memory_supersessions` rows become `supersedes` link entries. Relation
vocabulary reuses the memoir controlled vocabulary plus `supersedes`. `tags`
is a new field; `links` is the memory map. Consolidation writes only the new
consolidated file — the active set is derived at query time from `supersedes`
links, so superseded members are never rewritten.

### Decision (`.atlas/decisions/<id>.md`)

Frontmatter: `decision_id`, `session_id`, `repo-root` (implicit, dropped),
`query_text`, `source_ids` (list, pointing at `.atlas/artifacts/<id>.md`),
`related_files`, `related_symbols`, `created_at`, `updated_at`.
Body sections: `## Summary`, `## Rationale`, `## Conclusion`.

### Feedback (`.atlas/feedback/<id>.md`)

Frontmatter: `id`, `tool_name`, `analysis_kind`, `related_symbol`,
`related_file`, `source_id`, `created_at`, `metadata`.
Body sections: `## Predicted`, `## Actual`, `## Correction`.
`feedback_records_fts` becomes a derived index.

### Memoir graph (`.atlas/memoir/<graph>/`)

`_graph.md` frontmatter: `id`, `scope`, `name`, `description`, `created_at`,
`updated_at`, `metadata`. Concept files frontmatter: `id`, `name`, `kind`,
`source_id`, plus `relations: [{ relation: depends_on, target: <concept-slug>,
confidence: 1.0 }]`. Controlled relation vocabulary is unchanged; validation
stays in the service layer. `memoir_delete` removes the directory.

### Overview record (`.atlas/overview/<subject_kind>/<subject_id>.md`)

Frontmatter: `id`, `subject_id`, `subject_kind`, `package`, `module`,
`canonical_file_path`, `qualified_name`, `node_kind`, `line_start`, `line_end`,
`content_hash`, `title`, `source_kind`, `analyzer_name`, `analyzer_version`,
`model_name`, `commit_sha`, `graph_last_indexed_at`, `confidence`,
`source_id`, `supersedes` (id of the record this one replaces), `created_at`,
`updated_at`. `freshness` is computed at read time from `commit_sha` plus
current graph state and is never stored, so freshness sweeps rewrite nothing;
`overview_refresh_plan --apply` records marks only in the derived index.
Body: `## Abstract (L0)` (one-sentence summary), `## Overview (L1)`
(responsibilities, flow, inputs, outputs, invariants, gotchas, examples as
markdown lists), and L2 pointers (file spans and `source_id` artifact links).
`overview_show --level` slices these sections directly off the file without
reading past the section boundary.
An active record is one with no `supersedes_by` reader flag; superseded files
stay for audit, matching today's `supersedes_id` chain semantics.
`overview_record_fts` becomes derived.

### Session ledger (`.atlas/sessions/<id>/events.md`) — local-only

Frontmatter: `session_id`, `repo_root`, `frontend`, `worktree_id`,
`created_at`, `updated_at`, `last_compaction_at`.
One `###` section per event:

```
### e3089 2026-11-19T13:23:36Z session-start p=4
<!-- event_hash: 5f0c... -->
```json
{ "prompt": "..." }
```
```

Appends are buffered per process and flushed on `stop`/`session-end`, on a
small buffer cap, or on resume snapshot build; short-lived hook processes flush
at exit (see Read/write discipline). Code fences are computed per payload —
the longest backtick run inside the payload plus one, minimum four — so tool
output containing fence markers never breaks the ledger.
`compact_session` rewrites the whole file
with the same curation rules (dedup, per-type caps, priority promotion).
`resume.md` holds the latest snapshot as a fenced JSON block with frontmatter
`event_count`, `consumed`, `updated_at`. Resume snapshot and wake-up builds
read a bounded tail of the ledger plus compaction frontmatter rather than
parsing the full file, so long uncompacted sessions do not inflate snapshot
cost.

### Artifact (`.atlas/artifacts/<source_id>.md`) — moved out of `context.db`

Frontmatter: `source_id`, `source_type`, `label`, `session_id`, `created_at`,
plus graph-facing links `symbols: [qualified names]` and `files: [paths]` when
the artifact references code (review context, investigations). Body is the raw
payload. Chunks, `chunks_fts`, trigram, and vocabulary indexes are re-derived
from the body.

## Derived-only stores

`memory_access` (`last_accessed_at`, access counts, `decay_score`),
experience-edge `use_count` and validation state, `memory_embeddings`,
`global_*_access` counters, `global_workflow_patterns`,
`overview_record_fts`, `feedback_records_fts`, `chunks`/`chunks_fts`/trigram/
vocabulary, `retrieval_index_state`, `embedding_provider_registry`,
`overview_packets` — all derived, all rebuildable from markdown truth (counters
rebuild by scanning the events ledger; embedding regeneration is opt-in because
it calls an external provider). All live in a local `*.db` and are deleted
regeneratively. Counter derivation from the ledger also means session cleanup
(`delete_session`, `cleanup_stale_sessions`) expires that session's counter
contributions — an intended semantic change from today's independent counter
tables, documented here deliberately.

## Corruption and concurrency mechanics

- **Atomic writes:** temp file under `.atlas/tmp/` (same filesystem as every
  target directory by construction), `fsync`, `rename`. A crash leaves the
  previous file intact and the leftover fragment lands in `.atlas/tmp/`, never
  in a committed directory, so `git status` stays clean.
- **Cross-process writers exist** (CLI hook process + MCP server). Advisory
  lock files centralize under `.atlas/locks/<store>.lock` (`fs4` or
  equivalent; no globals, no `static mut` — lock acquired per call,
  dependency-injected paths) — never inside committed directories, where they
  would become git-tracked noise. Writers serialize only; readers take no
  lock because rename atomicity already protects them.
- **Recovery:** derived db corrupt or missing → `atlas index rebuild` rescans
  markdown. Truth files are plain text; per-file corruption is visible in any
  diff and repaired by hand if ever needed.
- **Curation** (`compact_session`) rewrites `events.md` under the lock with the
  current curation rules; no semantic change.

## Read/write discipline

Truth files are rewritten only when their semantic content changes. Everything
that mutates on a read or a background job lives in derived state.

1. **Reads never write.** `recall_memories`, `get_memory`, `overview_show`, and
   similar lookups update no files. Access bookkeeping (`last_accessed_at`,
   access counts, `decay_score`) lives in the derived `memory_access` index;
   recall still ranks with the same recency weights, reading the index, and
   updates access rows in one batched transaction per recall call, never one
   write per returned memory.
2. **Computed, not stored.** Overview `freshness` derives at read time from
   `commit_sha` plus current graph state; no freshness sweep rewrites committed
   files. Experience-edge `use_count` and validation timestamps stay in the
   derived index.
3. **Lock-free reads, writer-only locks.** Rename atomicity means readers never
   observe torn files; only writer-writer serialization takes the advisory
   lock. Recall pays no lock cost.
4. **Bounded reads.** Frontmatter sits at the head of each file; ranking parses
   frontmatter only, and full bodies are read solely for returned entries.
5. **Batched ledger flushes.** The MCP server buffers event appends in memory
   and flushes on `stop`/`session-end`, a small buffer cap, or resume snapshot
   build; the CLI hook (short-lived) flushes at process exit. Losing a buffered
   tail on crash matches today's best-effort session semantics.
6. **Incremental derived indexing.** Index updates compare content hashes and
   refresh only changed files (`chunks`, FTS, trigram); `atlas index rebuild
   --full` regenerates everything and `--embeddings` opts in to provider-backed
   vectors.
7. **Canonical serialization.** Frontmatter writes use a fixed field order and
   stable wrapping; bodies are preserved verbatim. Re-serializing an unchanged
   record is byte-identical, so tool-driven rewrites produce minimal git diffs
   instead of key-shuffle noise.
8. **Frontmatter cache.** Long-lived processes (the MCP server) keep a bounded
   in-process frontmatter cache (LRU) keyed by path plus mtime; every lookup
   stats the file first, so cross-process writes invalidate naturally. Recall,
   wake-up packs, and memoir inspect avoid re-parsing warm entries.
9. **Deferred artifact indexing.** `save_context_artifact` writes truth
   immediately but defers chunk/FTS refresh through a bounded dirty-set
   flushed on the next search or a background batch; tool latency never pays
   indexing cost. Dirty state persists as a per-directory mtime watermark, so a
   crash between write and flush reconciles with a cheap stat walk at the
   next open and search correctness never depends on unflushed process memory.
10. **Tiered loading (L0/L1/L2).** L0 is the frontmatter (`title`, `topic`,
   `tags`, `importance`) read as a bounded head via the cache; L1 is the
   body's summary sections, sliced at the next heading boundary; L2 is the
   full body plus its links (artifact `source_id`, file spans, graph nodes),
   fetched only for returned entries. `overview_show` and `overview_search`
   keep their `level` disclosure params as public contract. Recall ranks on
   L0, previews L1, and reads L2 only for entries it returns. Topic-level
   tiering comes from consolidation output — the kept consolidated memory is
   the topic's L0/L1 — not from generated abstract files, keeping committed
   directories free of derived content.

## Git policy and `.atlas/.gitignore`

The root `.gitignore` keeps only repo-wide rules (the `*.md` whitelist for
README, ISSUES, and plan docs). Atlas state tracking ships inside the state
directory: `atlas init` and `atlas install` write `.atlas/.gitignore`
idempotently, `--force` restores it after user edits, and
`atlas install --validate-only` reports a missing or unmanaged file:

```gitignore
# atlas-managed local-state git policy v1 — durable markdown records stay committed.
/*
!/.gitignore
!/memories/
!/decisions/
!/feedback/
!/memoir/
!/overview/
!/artifacts/
!/memories/**/*.md
!/decisions/**/*.md
!/feedback/**/*.md
!/memoir/**/*.md
!/overview/**/*.md
!/artifacts/**/*.md
```

`.atlas/config.toml`, `.atlas/hooks/`, `sessions/`, `tasks/`, `experience/`,
lock and tmp files, and `*.db` indexes stay untracked (`/*` covers them).
The deeper negations override a root `*.md` rule, so durable markdown records
stay committed without root `.gitignore` entries. Global-scope memories commit
inside the repo that stored them, per decision 2.

## Migration

One-shot exporter: `atlas migrate markdown [--dry-run]`.

1. Read each session.db table listed in the disposition below.
2. Write the markdown tree under `.atlas/` using bulk relaxed fsync (one fsync
   per directory batch); the verification pass below preserves correctness.
3. Verify round-trip: re-parse every file, compare records field-for-field.
4. On success, drop the migrated tables and rebuild derived indexes from
   markdown.
5. Keep the pre-migration `.db` as `.atlas/session.db.bak` until the user
   confirms, then `atlas migrate markdown --finalize` removes it.
6. No-op cleanly when no legacy tables exist, so fresh repositories skip
   migration entirely.
7. The bulk relaxed-fsync import path is shared with `overview ingest` batch
   imports.

## Disposition

| Current store | Target | Role |
|---|---|---|
| `memories`, `memory_supersessions` | `.atlas/memories/**` | truth; supersession = link |
| `memories.last_accessed_at`/`decay_score`, `experience_edges.use_count` | derived index | read-path bookkeeping, never written to truth files |
| `decision_memory` | `.atlas/decisions/**` | truth |
| `feedback_records` | `.atlas/feedback/**` | truth; FTS derived |
| `memoir_graphs/concepts/relations` | `.atlas/memoir/**` | truth |
| `overview_records` | `.atlas/overview/**` | truth; FTS derived |
| `durable_tasks` | `.atlas/tasks/**` | truth (local-only) |
| `experience_edges` (+deps) | `.atlas/experience/**` | truth (local-only) |
| `session_meta`, `session_events`, `session_resume` | `.atlas/sessions/**` | truth (local-only) |
| `global_*_access`, `global_workflow_patterns` | derived index | counters from events ledger |
| `memory_embeddings` | derived index | rebuilt from bodies |
| context.db `sources`, `chunks` | `.atlas/artifacts/**` | truth (local-only) |
| `chunks_fts`, trigram, vocabulary | derived index | rebuilt from bodies |
| `worldtree.db` (all tables) | unchanged | graph truth |

## Phases

Each phase lands independently with tests, clippy, and fmt clean.

- **M0 — markdown store foundation.** New crate `packages/atlas-store-markdown`
  (mandatory new crate: `atlas-session/src/store/mod.rs` is already past the 1K
  LOC edit threshold). Frontmatter parse/serialize, atomic write, locks, slug
  and id derivation, bounded read, golden round-trip tests.
- **M1 — memories.** Route store/recall/list/get/delete/consolidate/decay
  through the markdown backend with byte-stable contracts and unchanged ranking
  weights. Add `tags` + `links`, `memory_link` (MCP + CLI parity), tag filters in
  recall. One-shot migration for the memories family.
- **M2 — remaining durable records.** Decisions, feedback, memoir, overview,
  tasks, experience edges — same pattern per family, one checklist chunk each.
- **M3 — session ledger.** Append-only `events.md`, `resume.md`, derived
  counters; re-route hook writers (`atlas-adapters`) and the MCP event path.
- **M4 — artifacts.** Write-through `.atlas/artifacts/**`, re-derived chunk
  indexes, `atlas index rebuild`, `purge_saved_context` deletes files.
- **M5 — cleanup.** Remove dead migrations and sqlite paths per the upgrade
  policy (no compat shims). Update `docs/architecture.md`,
  `docs/threat-model-serve.md`, `SHIPPED.md`, `MCP_TOOLS.md`, README storage
  contract sections. Switch `.gitignore`.

## Testing and validation

- Unit tests per backend module: frontmatter round-trip, atomic write survives a
  simulated crash (temp file left, target unchanged), lock contention across
  threads.
- Integration tests with `tempfile::TempDir` repos, including a two-process
  writer scenario for the events ledger.
- Golden trees: seeded session.db → expected markdown tree → re-parse equality.
- Migration dry-run test asserts field-for-field round-trip before any table
  drop.
- Per phase: `cargo test --workspace --quiet`, clippy
  `--workspace --all-targets --all-features -- -D warnings`, `cargo fmt --all`.
- Read-path purity test: `recall_memories` and decay leave memory file bytes and
  mtimes untouched; only the derived `memory_access` index changes.
- Byte-stability test: parsing and re-serializing an unchanged record file is
  byte-identical (canonical field order, stable wrapping, verbatim bodies).
- Frontmatter cache test: an mtime bump invalidates a cached entry; unchanged
  files serve from cache without re-parse.
- Ledger fence test: an event payload containing triple backticks round-trips
  through the computed-fence writer and parser.
- Crash-leftover test: an interrupted write leaves only a `.atlas/tmp/`
  fragment; committed directories and `git status` stay clean.
- Watermark reconciliation test: reopening after an unflushed write indexes
  exactly the files newer than the persisted watermark.
- No fixed sleeps in tests; bounded waits and explicit lock release per the
  flaky-test rules.
- Per-phase edge-case coverage (malformed input, slug and id collisions,
  crash windows, cross-process races, dangling references) is tracked as
  dedicated checklist blocks under each phase in `ISSUES.md`, and per-phase
  acceptance-criteria blocks there define the gate tested after each phase
  update (seeded-corpus byte-stable contracts, purity assertions, rebuild
  parity, git-clean checks).

## Non-goals

- No new fourth database; derived indexes reuse existing `.db` files.
- No change to `worldtree.db` schema or graph build/update.
- No backward compatibility layer for the old sqlite-backed continuity tables
  once migration ships (upgrade policy: remove deprecated paths).
- No vector-store changes beyond re-pointing `memory_embeddings` at markdown
  bodies as its source.
