# Atlas — Stateful Coding Agent Backend

Instruction for all the items in this file:
- Keep each checklist item scoped to one small workable chunk.
- Describe exact code, command, schema field, validation rule, or test to add/change.
- Do not combine multiple implementation steps into one checklist item if they can be merged separately.
- Prefer additive wording like "add", "replace", "update", "remove", "validate", "test".
- Avoid broad goals without concrete implementation detail.

## Goal

Create a cli for stateful coding agent backend.

The primary behavior to preserve is:

- build a repository code graph
- incrementally update it from git diffs
- persist graph data in SQLite
- query graph structure and impact radius
- assemble review context from changed files and neighboring nodes
- expose a CLI first, with MCP later
- make sure to ALWAYS sync CLI tool and MCP tooling (including its flags)

For terms that are easy to misread in this document:

- `flow`: named ordered path or scenario over existing graph nodes, for example `http request -> handler -> service -> repository`, `changed symbol -> direct callers -> affected tests`, or `review path for this PR`. This is metadata over graph, not new edge kind and not runtime tracing requirement in v1.
- `flow membership`: join row in `flow_memberships` that says one node participates in one flow, with optional `position`, `role`, and metadata. `membership` here never means user/team/account membership.
- `community`: unordered cluster of related nodes/modules/files found by some graph algorithm or heuristic, for example SCC/cycle cluster, package cluster, or architecture slice. Community says "these belong together"; flow says "these form ordered path".
- `embeddings`: optional vector search data for retrieval/ranking only. Not required for core build/update/query path.

### Core Design Rule

- Avoid feature growth without signal quality gains
- Prioritize better ranking
- Prioritize better context
- Prioritize better signals

---

## Roadmap Layout

- Remaining focused follow-up patches: Shared Parser Query Migration Patch remainder, Minimal-Context Escalation Metadata Patch, Atlas Tokenizer implementation phases, and Markdown Continuity Storage. The retrieval follow-up, runtime classification, reachability, and SQLite index audit patches are shipped; see SHIPPED.md. Runtime policy surface is deferred.

---

## Follow-Up Patches

Use these patch sections for focused improvements that cut across existing roadmap phases without rewriting phase scope.

### Shared Parser Query Migration Patch

Query-backed extraction shipped for C, C++, C#, Java, Scala, JavaScript, TypeScript, Python, Ruby, PHP, Bash, HTML, and CSS. See SHIPPED.md for the shared query contract and language coverage. JSON, TOML, and Markdown are documented manual exceptions.

Remaining:

- [ ] migrate the Go parser to `packages/atlas-parser/queries/go.scm` with shared `@atlas.*` captures
- [ ] document why Go remains manual if query migration cannot preserve current qualified names, receiver/call behavior, or golden outputs
- [ ] confirm every non-Rust parser has either an Atlas-owned query file or a documented reason to remain manual
- [ ] run `cargo test -p atlas-parser` after the final migration batch
- [ ] run `cargo clippy --workspace --all-targets --all-features -- -D warnings` after the final migration batch
- [ ] run `cargo fmt --all` after the final migration batch

---

### Minimal-Context Escalation Metadata Patch

The canonical minimal-first workflow and its agent-facing surfaces are shipped: installed AGENTS instructions, `review_change` and `inspect_symbol` prompts, README, and MCP tool guidance all document the same order. VR3 also emits evidence-sufficiency and escalation metadata for context and review results.

Do not add internal minimal-context enforcement to higher-level tools. It duplicates retrieval already performed by those tools, adds latency and tokens, and conflicts with explicit tool contracts such as review, impact, and refactor planning that need their full target set.

Rules:

- keep escalation evidence-driven and deterministic
- do not add a second retrieval pass inside tools that already resolve their targets
- keep default output stable when no escalation is justified
- keep prompt, README, installed instructions, and tool descriptions aligned

Implementation structure:

#### Patch E3 — Escalation hints on minimal context and neighbors

- [ ] add `next_tools: Vec<String>` and `recommended_escalation_reason: Option<String>` to `get_minimal_context` output in `packages/atlas-mcp/src/tools/context_ops/minimal.rs`, skipped from JSON when empty or absent
- [ ] map `summary.truncated = true` to `next_tools = ["get_review_context"]` and reason `truncated`
- [ ] map `deleted_files_present` to `next_tools` including `get_impact_radius` and reason `deleted_files`
- [ ] map `test_impact` to `next_tools` including `get_review_context` and reason `missing_tests`
- [ ] leave both fields empty when the result is complete and unambiguous
- [ ] add the same fields to `symbol_neighbors` output in `packages/atlas-mcp/src/tools/graph.rs` when budget metadata reports truncation
- [ ] map neighbor truncation to `next_tools = ["get_context", "traverse_graph"]` and reason `truncated`
- [ ] update MCP output schemas for `get_minimal_context` and `symbol_neighbors` with optional `next_tools` and `recommended_escalation_reason`
- [ ] add tests for complete output, truncated output, deleted-file output, and test-impact output
- [ ] audit MCP tool descriptions for `get_review_context`, `get_impact_radius`, `explain_change`, `get_context`, and `traverse_graph`; add minimal-first wording where missing
- [ ] add a drift test that asserts the same escalation order appears in `packages/atlas-cli/src/install/instructions.rs`, `packages/atlas-mcp/src/prompts.rs`, and README

#### Rejected enforcement

- [ ] do not require `get_review_context`, `explain_change`, `get_impact_radius`, `analyze_*`, or refactor tools to run `get_minimal_context` internally; VR3 sufficiency already escalates when evidence is missing

#### Patch completion criteria

- [ ] `get_minimal_context` and `symbol_neighbors` emit escalation hints only when justified
- [ ] complete results keep default output byte-stable
- [ ] context and review tools keep VR3 sufficiency metadata
- [ ] prompt, installed instructions, README, and tool descriptions agree on escalation order
- [ ] no tool performs a hidden second retrieval pass

---

### Atlas Tokenizer Patch

Full design: `docs/PLAN-TOKENIZER.md`. Goal: standalone `atlas-tokenizer` crate porting OpenAI `tiktoken`, opt-in provider first, default only after stability gates.

#### Phase 1 — Crate Scaffold

- [x] add `packages/atlas-tokenizer` with `Cargo.toml`, `README.md`, and `LICENSE-TIKTOKEN`
- [x] add `atlas-tokenizer` to workspace members and `[workspace.dependencies]`
- [x] add `#![forbid(unsafe_code)]` and `#![deny(missing_docs)]` to `packages/atlas-tokenizer/src/lib.rs`
- [x] add `cargo check -p atlas-tokenizer --no-default-features` gate

#### Phase 2 — BPE Core

- [x] add `packages/atlas-tokenizer/src/rank_file.rs` parser with malformed-input and size-cap tests
- [x] add `packages/atlas-tokenizer/src/bpe.rs` byte-pair encode/decode with merge edge-case tests
- [x] add property test asserting `decode_bytes(encode_ordinary(x)) == x.as_bytes()`

#### Phase 3 — Encodings and Model Map

- [x] add encoding constants with exact upstream regex patterns and special tokens
- [x] add `model.rs` longest-prefix model-to-encoding map
- [x] add `registry.rs` explicit registry plus `OnceLock` default
- [x] add `special_tokens.rs` and `model_map.rs` tests

#### Phase 4 — Download and Cache

- [x] add `packages/atlas-tokenizer/src/assets.rs` with `UPSTREAM_COMMIT`, per-encoding URL + sha256, and exact upstream GitHub file references
- [x] add `packages/atlas-tokenizer/src/download.rs` with HTTPS-only fetch, timeouts, redirect scheme check, 32 MiB cap, and sha256 verification
- [x] add `packages/atlas-tokenizer/src/cache.rs` with upstream-compatible `sha1(url)` key, env dir precedence, atomic temp+rename, and corrupt-entry recovery
- [x] add `download_cache.rs` tests with a local fixture HTTP server; no real network
- [x] add `tokenizer_file` local path and `download` feature-off behavior
- [x] add ignored upstream smoke test; verify warm-cache run works with networking disabled

#### Phase 5 — Parity and Bench

- [x] add maintainer golden generator for official Python `tiktoken` at the pinned commit
- [x] commit golden JSONL for all supported encodings
- [x] add `tests/parity.rs` and reach zero mismatches for `cl100k_base` and `o200k_base`
- [x] add criterion encode/decode bench and record baselines
- [x] add tokenizer fuzz target under workspace `fuzz/`

#### Phase 6 — Integration

- [x] add `Tiktoken` variants to `atlas_token_count::TokenCounter` and `TokenCountMethod`
- [x] add `provider = "tiktoken"` to `ContextTokenizerConfig` with encoding/model/file validation
- [x] build tiktoken counter in `Config::token_counter` with existing fallback semantics
- [x] add optional `encoding` field to token accounting in `atlas-review/src/context/payload.rs`
- [x] update config template, runtime config JSON, README, and `SHIPPED.md`
- [x] add CLI and MCP tokenizer quality-gate cases for `provider = "tiktoken"`

#### Phase 7 — Stability Switch and Legacy Removal

- [x] once atlas-tokenizer is stable, change `ContextTokenizerConfig::default()` to tiktoken and pick the default encoding from parity and bench results
- [x] remove `provider = "heuristic"` as a selectable provider; keep byte-based counting only as automatic fallback unless stability review says otherwise
- [x] remove `provider = "tokenizers"` and the `tokenizers` dependency
- [x] remove the `paste` advisory ignore from `deny.toml` after dependency removal
- [x] run full workspace tests, clippy, fmt, deny, and bench regression check

---

### Markdown Continuity Storage Patch

Full design: `docs/PLAN-MARKDOWN-STORAGE.md`. Goal: durable continuity records become markdown truth under `.atlas/`; sqlite keeps only derived, rebuildable indexes. `worldtree.db` unchanged. Resolved: commit only `memories/`, `decisions/`, `feedback/`, `memoir/`, `overview/` dirs; sessions/events, tasks, artifacts, and experience edges stay local. Global-scope memories stay in workspace `.atlas/`. Session events ledger is markdown.

Implementation note: SQLite tables for these families are retained as
**derived, rebuildable indexes** (plan principle 2). Truth lives in markdown;
`atlas migrate markdown` writes/verifies it, and `atlas index rebuild`
regenerates artifact retrieval indexes. Session appends flush immediately
(buffer cap 1) and resume builds read the derived index instead of parsing the
ledger, which meets the bounded-read intent without an in-memory buffer.

Second-pass integrity additions:

- `SessionStore::open` imports markdown into the derived tables before any read,
  so hand edits survive, and deleting `session.db` rebuilds session meta,
  events, resume snapshots, decisions, feedback, memoir graphs, overviews,
  tasks, and experience edges from `.atlas/` instead of losing continuity.
- Memory recall reconciles `memory_index` against `.atlas/memories/**` by
  persisted `(mtime, size)` identity, so new, edited, and deleted memory files
  are indexed without reading unchanged bodies. Malformed files are skipped per
  path and never block other records.
- `atlas migrate markdown` opens the store with markdown reconciliation
  disabled, so `--dry-run` writes nothing; backups use `VACUUM INTO` for a
  consistent WAL snapshot.
- Memoir deletion removes the committed markdown directory, and artifact truth
  is written before derived chunk indexing so a failed index never loses the
  payload.

Known remaining optimizations (not correctness blockers): `overview_show
--level` still projects from stored record fields rather than slicing the file,
and artifact save still performs synchronous chunk indexing. L0/L1/L2 semantics
and byte-stable outputs are preserved.

#### Phase M0 — markdown store foundation

- [x] add `packages/atlas-store-markdown` crate with frontmatter parse/serialize, bounded read, and slug/id derivation helpers; new crate is mandatory because `packages/atlas-session/src/store/mod.rs` exceeds the 1K LOC edit threshold
- [x] add atomic write using temps under `.atlas/tmp/` (same filesystem as targets), `fsync`, then `rename`; crash-simulation test asserts the target stays intact and the leftover fragment lands in `.atlas/tmp/`, never in committed directories
- [x] add advisory writer locks centralized under `.atlas/locks/<store>.lock` (vetted `fs4`-style crate) so lock files never appear inside committed directories; two-writer contention test using `tempfile::TempDir`, no globals, locks acquired per call
- [x] make reads lock-free by relying on rename atomicity; add a test asserting a concurrent read during a write sees either the old or the new file content, never a torn file
- [x] add golden round-trip tests for each record frontmatter type (memory, decision, feedback, memoir, overview, task, experience, artifact, session)
- [x] add canonical frontmatter serialization (fixed field order, stable wrapping, verbatim bodies) with a byte-stability test asserting parse-then-serialize of an unchanged record is byte-identical
- [x] add an in-process frontmatter cache keyed by path plus mtime for long-lived MCP server reads, invalidated per lookup by an mtime stat, with a cache test covering cross-process invalidation
- [x] add workspace member entry and run `cargo check`, clippy `-D warnings`, and fmt gates

#### Phase M0 — edge cases

- [x] add frontmatter edge-case tests: titles with colons, quotes, and newlines; unicode titles, tags, and topics (CJK, emoji); tags containing slashes; empty-string versus absent optional fields serialize canonically
- [x] add body edge-case tests: body starting with `---`, body containing frontmatter-shaped lines, empty body, single very long line, CRLF endings, and a UTF-8 BOM all round-trip byte-identical
- [x] add slug/id edge-case tests: two topics colliding to the same slug, empty and unicode-only titles, deterministic id derivation for identical inputs, and duplicate id detection across topic directories
- [x] add lock edge-case tests: a writer process killed mid-write releases the OS lock (flock dies with the process), a stale lock never blocks a fresh writer, and a read during a compaction rewrite sees old-or-new content
- [x] add cache edge-case tests: file replaced by a shorter one (rename), file deleted between stat and open (TOCTOU returns not-found, not a panic), same-mtime rewrite detected via size, and the LRU cap evicts correctly when a directory scan exceeds it
- [x] add tmp edge-case tests: stale `.atlas/tmp/` fragments from a prior crash are cleaned on store open and never appear inside committed directories

#### Phase M0 — acceptance criteria

- [x] all record-type golden round-trip tests pass and parse-then-serialize of every golden file is byte-identical
- [x] crash-simulation suite passes: a killed writer leaves the previous target intact and the fragment lands only in `.atlas/tmp/`
- [x] two-writer contention test passes with no lost update; the concurrent-read test observes only old-or-new content, never torn
- [x] no lock or temp file ever exists inside a committed directory (path assertions across the whole test suite)
- [x] frontmatter cache tests pass: mtime bump invalidates cross-process, the LRU cap evicts, and TOCTOU returns not-found without panic
- [x] `cargo check`, workspace clippy `-D warnings`, and fmt pass for the new crate with no unsafe code

#### Phase M1 — memories

- [x] route `store_memory`, `get_memory`, `list_memories`, `delete_memory`, `recall_memories`, decay, and consolidation through the markdown backend keeping `SessionStore` method contracts and ranking weights unchanged
- [x] add `tags` and `links` frontmatter fields; validate link targets exist and relations come from the memoir vocabulary plus `supersedes`
- [x] replace `memory_supersessions` rows with `supersedes` link entries written at consolidation time
- [x] move `last_accessed_at`, access counts, and `decay_score` into a derived `memory_access` index so recall and decay never rewrite memory files; add a read-path purity test asserting recall leaves file bytes and mtimes unchanged; batch access-row updates in one transaction per recall call, never one write per returned memory
- [x] derive the active memory set at query time from `supersedes` links so consolidation writes only the new consolidated file and never rewrites superseded members
- [x] keep recall ranking on L0 frontmatter only, with a bounded-read assertion that candidate bodies are not read during ranking
- [x] add `memory_link` MCP tool and `atlas memory link` CLI command with parity tests per the CLI/MCP sync rule
- [x] add `tags` filter argument to `memory_recall` MCP tool and `atlas memory recall` CLI
- [x] add `atlas migrate markdown [--dry-run]` exporting the memories family from `session.db` with field-for-field round-trip verification before any table drop; use bulk relaxed fsync with a final verification pass and no-op cleanly when no legacy tables exist
- [x] drop `memories`, `memory_supersessions`, and `memory_embeddings` tables after migration; rebuild embeddings from markdown bodies on demand behind an opt-in `--embeddings` flag
- [x] replace `memories_table_schema_matches_golden` and `memories_indexes_match_golden` with golden frontmatter and directory-tree tests

#### Phase M1 — edge cases

- [x] add link edge-case tests: self-link rejected, link to a deleted memory tolerated at read and reported by memory health, invalid relation rejected, duplicate link entries collapse, and a `supersedes` cycle leaves active-set derivation deterministic
- [x] add supersedes-chain test: A superseded by B superseded by C leaves only C active, and recall excludes A and B
- [x] add hand-edit resilience test: malformed frontmatter or invalid scope/importance in one file fails only that record with its path in the error, never panics, and never blocks other records in the same directory
- [x] add consolidation edge-case tests: single-member group is a no-op, overflow bodies route to artifacts with `source_id` set, an interrupted consolidation leaves the old active set unchanged (single-write atomicity), and hand-edited members still consolidate
- [x] add recall edge-case tests: read-only memory directory still recalls (derived access-index write failure degrades to a warning, never a failed recall), empty tags filter matches nothing, and recall ranking on seeded pre-migration rows matches the sqlite golden ranking exactly
- [x] add field-validation edge-case tests: session-scope memory without a session id and frontend-scope without a frontend are rejected; duplicate tags collapse

#### Phase M1 — acceptance criteria

- [x] seeded-corpus contract test passes: `memory_store`, `recall_memories`, `list_memories`, `get_memory`, and `delete_memory` outputs byte-stable against the sqlite backend before the table drop
- [x] recall ranking on seeded pre-migration rows matches the legacy sqlite golden ranking exactly
- [x] read-path purity verified: a full recall plus decay cycle leaves every memory file's bytes and mtimes unchanged
- [x] link behavior verified: invalid, self, and cyclic links rejected or deterministic, and an A→B→C `supersedes` chain leaves only C active in recall
- [x] `memory_link` MCP and CLI golden outputs match the parity rule and `tags` filters work on both surfaces
- [x] `atlas migrate markdown --dry-run` on a seeded `session.db` produces the golden tree with field-for-field round-trip equality; the post-drop re-run is a no-op
- [x] `git status` on a migrated repo shows only the expected memory files

#### Phase M2 — remaining durable records

- [x] route `decision_memory` store/search through `.atlas/decisions/<id>.md` with Summary/Rationale/Conclusion body sections; keep `search_decisions` output byte-stable
- [x] route `feedback_records` through `.atlas/feedback/<id>.md` with Predicted/Actual/Correction body sections; make `feedback_records_fts` derived and rebuildable
- [x] route memoir graphs, concepts, and relations through `.atlas/memoir/<graph>/` (`_graph.md` plus concept files with frontmatter relations); keep `memoir_inspect` and `memoir_delete` contracts
- [x] route `overview_records` through `.atlas/overview/<subject_kind>/<subject_id>.md` with L0/L1 body sections and `supersede` frontmatter chains; keep `overview_show`, `overview_search`, `overview_refresh_plan`, and `overview_ingest` output stable
- [x] keep the `level: l0|l1|l2` disclosure params slicing markdown sections at heading boundaries, with a bounded-read test asserting l0/l1 requests never read the full file
- [x] share the bulk relaxed-fsync import path between `atlas migrate markdown` and `overview ingest` batch imports
- [x] route `durable_tasks` through `.atlas/tasks/<id>.md` with checkbox bodies; tasks stay local-only
- [x] route `experience_edges` through `.atlas/experience/<slug>.md`; keep `experience_inspect` output stable; edges stay local-only
- [x] extend `atlas migrate markdown` to each family in this phase and drop the matching `session.db` tables after verified round-trip

#### Phase M2 — edge cases

- [x] add memoir edge-case tests: concept names colliding case-insensitively rejected, relation to a missing concept rejected, self-relation rejected, confidence 0 and 1 accepted while out-of-range rejected, and `memoir_delete` dry-run counts match the actual delete
- [x] add overview supersede-chain test: A superseded by B superseded by C leaves only C active and search/show return only C by default
- [x] add overview level edge-case tests: `--level l0` on a record missing its `## Abstract` section degrades gracefully, `--level l1` slicing includes nested `###` subsections but stops at the next `##`, and ingest with a mismatched packet id is rejected
- [x] add overview freshness edge-case tests: commit matching HEAD is fresh, older commit is possibly stale, deleted subject file reports orphaned, and external ingest without graph verification reports unverified
- [x] add decision/feedback edge-case tests: dangling artifact `source_id` tolerated at read, empty query returns the bounded recent list, and malformed body sections fall back to raw text
- [x] add task edge-case test: checkbox update preserves surrounding body bytes and unknown task id returns not-found
- [x] add experience edge-case tests: source URI equal to target URI rejected, duplicate (repo, source, target, query) upserts instead of duplicating, and an empty inspect report is valid output

#### Phase M2 — acceptance criteria

- [x] seeded-corpus contract tests pass for decisions, feedback, memoir, overview, tasks, and experience edges against the sqlite backend before drops
- [x] `overview_show` and `overview_search` `level` params slice markdown, and l0/l1 bounded-read assertions prove those levels never read the full file
- [x] supersede chains leave only the newest record active, and freshness computed at read returns the same verdicts as stored legacy values on the seeded corpus
- [x] `memoir_delete` dry-run counts equal actual delete counts
- [x] per-family migration round-trips are verified before each table drop, and re-running migration per family is a no-op

#### Phase M3 — session ledger

- [x] write session events to append-only `.atlas/sessions/<id>/events.md` as one `###` section per event with fenced JSON payload and `event_hash` dedup markers, under the store lock
- [x] rewrite `compact_session` curation as whole-file rewrite with the existing dedup, per-type caps, and priority promotion rules
- [x] store resume snapshots in `.atlas/sessions/<id>/resume.md` with frontmatter `event_count`, `consumed`, `updated_at` and fenced JSON body
- [x] buffer event ledger appends per process and flush on `stop`/`session-end`, a small buffer cap, or resume snapshot build; short-lived hook processes flush at exit, and crash loss of a buffered tail matches existing best-effort session semantics
- [x] compute event code fences per payload (longest backtick run inside the payload plus one, minimum four) so payloads containing triple backticks never break the ledger; add a round-trip test for such payloads
- [x] make resume snapshot and wake-up builds read a bounded ledger tail plus compaction frontmatter instead of parsing the full events file
- [x] make `global_symbol_access`, `global_file_access`, and `global_workflow_patterns` derived counters rebuilt by scanning the events ledger; document that session cleanup now expires that session's counter contributions (intended semantic change from independent counter tables)
- [x] re-route hook and adapter writers in `packages/atlas-adapters/src/hooks.rs`, `packages/atlas-adapters/src/bridge.rs`, and `packages/atlas-mcp/src/session_events.rs` to the markdown ledger
- [x] drop `session_events`, `session_resume`, `session_meta`, and the global access tables after migration

#### Phase M3 — edge cases

- [x] add event edge-case tests: duplicate `event_hash` appends once, a payload of only backticks survives the computed fence, control characters round-trip via JSON escapes, and out-of-order timestamps parse in file order
- [x] add ledger-tail edge-case tests: a bounded tail cut mid-section discards only the partial tail section, `events.md` missing while `resume.md` exists rebuilds a minimal snapshot from the resume event count, and a corrupted resume JSON block rebuilds rather than panics
- [x] add compaction edge-case tests: compaction flushes the pending buffer before rewriting, a zero-event session compacts to an empty ledger, and compaction concurrent with a hook append from another process serializes on the writer lock
- [x] add resume edge-case tests: `mark_resume_consumed` twice stays idempotent, an oversized snapshot trims via the existing trim path, and the session directory is created on first append
- [x] add counter edge-case tests: rebuild with an empty ledger yields zeros, deleting a session expires its counter contributions (the documented semantic change), and workflow 3-gram counting handles command lists shorter than three

#### Phase M3 — acceptance criteria

- [x] `record_session_event`, `resume_session`, and wake-up flows return byte-stable structured output on a seeded event corpus
- [x] the two-process writer test passes with no lost or duplicated events under contention, and `event_hash` dedup holds across processes
- [x] the resume snapshot built from the markdown ledger is semantically equal to the legacy sqlite snapshot for the same seeded events
- [x] compaction on a seeded ledger produces the same curation outcome (per-type caps, promotion, dedup) as the legacy implementation
- [x] bounded-tail assertion passes: resume build and wake-up do not parse the full events file
- [x] counters rebuilt from the ledger equal the legacy counter values on seeded data

#### Phase M4 — context.db artifacts

- [x] write artifacts through to `.atlas/artifacts/<source_id>.md` with frontmatter `label`, `source_type`, `session_id`, `created_at`, and optional `symbols`/`files` graph links
- [x] re-chunk markdown bodies into the derived `chunks`, `chunks_fts`, trigram, and vocabulary indexes incrementally via content-hash compare per changed artifact; add `atlas index rebuild` with `--full` for full regeneration and opt-in `--embeddings` for provider-backed vectors
- [x] defer artifact derived-index refresh through a bounded dirty-set flushed on the next search or a background batch, so `save_context_artifact` tool latency never pays chunking and FTS cost synchronously; persist dirty state as a per-directory mtime watermark so a crash between write and flush reconciles with a cheap stat walk at next open
- [x] make `purge_saved_context` delete markdown files plus their derived rows
- [x] drop `sources` and `chunks` truth storage from `context.db` after verified round-trip; keep only derived index tables

#### Phase M4 — edge cases

- [x] add artifact round-trip edge-case tests: bodies with CRLF, a UTF-8 BOM, no trailing newline, and an empty body all round-trip byte-identical through write and derived chunking
- [x] add artifact write edge-case tests: two processes saving same-label artifacts get distinct ids and files, multi-MB artifacts stay within the write latency budget, and a label containing newlines serializes safely
- [x] add watermark edge-case tests: a crash between truth write and watermark persist reconciles exactly the newer files at reopen, and re-indexing an already-indexed file is a no-op (idempotent chunk upsert)
- [x] add rebuild edge-case tests: deleting `context.db` and rebuilding reproduces identical chunk boundaries and search results, and an artifact referenced by an overview `source_id` deleted after indexing flags a dangling L2 link without failing search
- [x] add purge edge-case tests: the `keep_days` boundary behaves as documented, purging removes both file and derived rows, and `chunk_offset` paging stays stable across an incremental reindex

#### Phase M4 — acceptance criteria

- [x] `save_context_artifact`, `search_saved_context`, and `read_saved_context` outputs byte-stable against the sqlite backend before the truth-table drop
- [x] artifact bodies round-trip byte-identical including CRLF, BOM, no-trailing-newline, and empty-body cases
- [x] delete `context.db`, run `atlas index rebuild`, and search results are identical to pre-delete with deterministic chunk boundaries
- [x] watermark reconciliation indexes exactly the files newer than the persisted watermark after a simulated crash, and re-indexing an unchanged file is a no-op
- [x] artifact save latency excludes indexing (deferred dirty-set) and the next search sees the new artifact
- [x] `purge_saved_context` removes both file and derived rows, and reading a purged `source_id` returns not-found

#### Phase M5 — cleanup and docs

- [x] remove dead session and content migrations for dropped tables per the upgrade policy; no compat shims
- [x] update `docs/architecture.md`, `docs/threat-model-serve.md`, `README.md`, `SHIPPED.md`, and `MCP_TOOLS.md` storage contract sections to the markdown-truth model
- [x] switch `.gitignore` to the committed-durable-rules block from `docs/PLAN-MARKDOWN-STORAGE.md` and verify `git check-ignore` passes for one committed and one local path
- [x] record shipped phases in `SHIPPED.md` and tick this checklist as phases land

#### Phase M5 — edge cases

- [x] add migration edge-case tests: a legacy db with unknown extra tables ignores them, a partially migrated store (memories done, decisions pending) resumes per family idempotently, and re-running `atlas migrate markdown` after the drop phase is a no-op
- [x] add end-to-end purity test: run the full M0–M4 flow in a `tempfile` repo and assert `git status` shows only expected durable markdown paths — no locks, temps, or db files
- [x] add restore-path test: copying `session.db.bak` back and re-running migration reproduces the same markdown tree byte-for-byte
- [x] extend `doctor`/`db_check` with markdown-tree integrity: orphan links, dangling artifact and `source_id` references, and malformed frontmatter reported per file path without failing the whole check

#### Phase M5 — acceptance criteria

- [x] full workspace `cargo test`, clippy `-D warnings`, and fmt pass; dead migrations removed; `doctor` and `db_check` green including markdown-tree integrity reporting
- [x] the end-to-end purity test passes: the full M0–M4 flow in a `tempfile` repo leaves `git status` showing only expected durable markdown paths
- [x] restoring `session.db.bak` and re-running migration reproduces the markdown tree byte-for-byte
- [x] a fresh-repo first run creates no legacy truth tables; both `.db` files are index-only from the first write
- [x] `docs/architecture.md`, `docs/threat-model-serve.md`, `README.md`, `SHIPPED.md`, and `MCP_TOOLS.md` describe the markdown-truth model with no stale references to dropped tables
- [x] SHIPPED.md records each phase as it lands and this checklist is ticked

#### Validation (every phase)

- [x] `cargo test --workspace --quiet` green
- [x] `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean
- [x] `cargo fmt --all` applied

---

## Additional Backlog

- [ ] add CI-visible parser cache hit-ratio metric and fail when cache reuse drops below configured threshold
- [ ] add thin LSP shim that maps Atlas query/context/impact/reference flows onto standard LSP requests