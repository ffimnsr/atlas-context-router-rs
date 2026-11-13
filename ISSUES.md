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

- Remaining focused follow-up patches: Shared Parser Query Migration Patch remainder and Minimal-Context Escalation Metadata Patch. The retrieval follow-up, runtime classification, and reachability patches are shipped; see SHIPPED.md. Runtime policy surface is deferred.

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

### SQLite Index Audit Patch

Static audit of every SQL statement in `atlas-store-sqlite`, `atlas-contentstore`, `atlas-session`, `atlas-db-utils`, and engine doctor paths. Plans were checked with `EXPLAIN QUERY PLAN` against live `.atlas/worldtree.db`, `.atlas/context.db`, and `.atlas/session.db`. Unused-index findings are static (no query telemetry); re-check with plans before dropping indexes used by planned history queries.

Observed full scans on hot paths: `nodes.name`, `nodes.file_path LIKE` (case-insensitive LIKE cannot use BINARY indexes), `files.path` (unscoped lookup), `overview_records.supersedes_id` correlated `NOT EXISTS`, `overview_records.source_id`, `feedback_records` ordering, `session_meta.updated_at`, `memoir_relations.repo_root`, and content `sources.id LIKE` / `created_at` ordering.

#### P0 — missing indexes for hot lookups

- [x] add graph migration `packages/atlas-store-sqlite/src/migrations/021_graph_index_hot_paths.sql` registering `idx_nodes_name_order ON nodes(name, file_path, line_start)` to replace the full scan in `nodes_by_name` (`packages/atlas-store-sqlite/src/store/context.rs`)
- [x] add `idx_nodes_name_nocase ON nodes(name COLLATE NOCASE)` in migration 021 to index `nodes_by_name_prefix` `name LIKE ? ESCAPE '\'`; keep it separate from the BINARY name index because default SQLite LIKE is case-insensitive
- [x] add session migration `packages/atlas-session/src/migrations/016_session_index_hot_paths.sql` creating `idx_overview_records_supersedes ON overview_records(supersedes_id) WHERE supersedes_id IS NOT NULL`
- [x] replace `ACTIVE_PREDICATE` correlated scan cost in `packages/atlas-session/src/store/overview/store.rs` by confirming the new `supersedes_id` index removes `SCAN newer` from the plan
- [x] add `idx_overview_records_source_id ON overview_records(source_id) WHERE source_id IS NOT NULL` in session migration 016 for the repair query at `packages/atlas-session/src/store/overview/store.rs`

#### P1 — missing indexes for ordering, filters, and LIKE prefixes

- [x] add `idx_nodes_file_path_order ON nodes(file_path, line_start, qualified_name)` and drop `idx_nodes_file_path` in graph migration 021 to remove temp sorts from `list_nodes` and `nodes_by_file`
- [x] add `idx_nodes_repo_file_path_order ON nodes(source_repo_id, file_path, line_start, qualified_name)` and drop `idx_nodes_source_repo_file_path` in graph migration 021 for repo-scoped file/node reads
- [x] add `idx_nodes_file_path_nocase ON nodes(file_path COLLATE NOCASE)` and `idx_nodes_repo_file_path_nocase ON nodes(source_repo_id, file_path COLLATE NOCASE)` in graph migration 021 for `query_graph` subpath/exclusion `LIKE` filters in `packages/atlas-store-sqlite/src/store/search.rs`
- [x] add `idx_files_path ON files(path)` in graph migration 021 for unscoped `SELECT hash FROM files WHERE path = ?1` and `single_source_repo_id_for_value` lookups in `packages/atlas-store-sqlite/src/store/mutation.rs`
- [x] replaced `idx_files_repo_path_nocase` plan with a half-open BINARY range in `file_paths_with_prefix_for_repo` (`packages/atlas-store-sqlite/src/store/mutation.rs`); `EXPLAIN QUERY PLAN` showed the NOCASE index can never win against the ordered covering `files(source_repo_id, path)` primary key, so the range scan on that key is used instead and no NOCASE twin is created
- [x] add `idx_files_indexed_at ON files(indexed_at DESC)` in graph migration 021 for `recently_indexed_files` and `MAX(indexed_at)` queries
- [x] add `idx_edges_target_kind ON edges(target_qualified, kind)` and drop `idx_edges_target` in graph migration 021 for dead-code anti-joins and caller lookups
- [x] add `idx_commits_repo_author_time ON commits(repo_id, author_time DESC, commit_sha DESC)` and drop `idx_commits_repo_id` in graph migration 021 for `list_commits` and history status ordering
- [x] add `idx_session_meta_updated_at ON session_meta(updated_at DESC)` in session migration 016 for `list_sessions` and `cleanup_stale_sessions`
- [x] add `idx_feedback_repo_created ON feedback_records(repo_root, created_at DESC, id DESC)` in session migration 016 for `recent_feedback` and feedback search ordering
- [x] add `idx_experience_edges_repo_freshness ON experience_edges(repo_root, freshness)` and `idx_experience_edges_repo_validated ON experience_edges(repo_root, last_validated_at)` in session migration 016 for activation and pruning
- [x] add `idx_memoir_relations_repo_source ON memoir_relations(repo_root, source_concept_id)` and `idx_memoir_relations_repo_target ON memoir_relations(repo_root, target_concept_id)` in session migration 016 for the repo-scoped relation walk in `packages/atlas-session/src/store/memory_recall_aux.rs`
- [x] add content migration `packages/atlas-contentstore/src/migrations/011_content_index_hot_paths.sql` creating `idx_sources_created_at ON sources(created_at)` for oldest-source lookup, cleanup, and `ORDER BY created_at DESC`
- [x] add `idx_sources_session_created ON sources(session_id, created_at DESC)` in content migration 011 for session-scoped source listing
- [x] skipped `idx_sources_id_nocase`; `recent_source_ids_by_prefix` always carries `ORDER BY created_at DESC LIMIT n`, and the planner always prefers the ordered covering `idx_sources_created_at` scan over an id-prefix index (verified with bound parameters plus `ANALYZE`)

#### P2 — lower-impact missing indexes

- [x] add `idx_edges_repo_confidence_tier ON edges(source_repo_id, confidence_tier)` in graph migration 021 for `count_edges_with_confidence_tier` and `edges_with_confidence_tier`
- [x] add `idx_edges_order ON edges(source_qualified, target_qualified, kind, file_path)` and drop `idx_edges_source` in graph migration 021 for `list_all_edges` ordered export
- [x] add `idx_nodes_file_parent ON nodes(file_path, parent_name)` in graph migration 021 for `containment_siblings`
- [x] add `idx_memories_repo_active ON memories(repo_root) WHERE superseded_by IS NULL` in session migration 016 for recall/embedding coverage counts
- [x] add `idx_memories_repo_updated ON memories(repo_root, updated_at DESC)` in session migration 016 for `list_memories` ordering
- [x] add `idx_memories_repo_source ON memories(repo_root, source_id)` in session migration 016 for source-id recall joins
- [x] replace `idx_durable_tasks_updated_task` with `idx_durable_tasks_updated_created_task ON durable_tasks(updated_at DESC, created_at DESC, task_id DESC)` in session migration 016 to match cursor pagination
- [x] add `idx_session_events_event_type ON session_events(event_type, session_id)` in session migration 016 for global-memory focus lookups that filter `event_type` without a session id
- [x] add `idx_retrieval_chunks_repo_embedded ON retrieval_chunks(source_repo_id) WHERE embedding IS NOT NULL` in graph migration 021 for repo-scoped `embedded_count`

#### Remove duplicate indexes

- [x] drop graph `idx_files_source_repo_path`; it duplicates the `files(source_repo_id, path)` primary-key autoindex
- [x] drop graph `idx_repos_root_path`; it duplicates the `repos(root_path)` unique autoindex
- [x] drop graph `idx_graph_snapshots_repo_id`; it is a prefix of the `graph_snapshots(repo_id, commit_sha)` unique autoindex
- [x] drop graph `idx_historical_nodes_file_hash` and `idx_historical_edges_file_hash`; both are prefixes of their table primary keys
- [x] drop graph `idx_snapshot_files_snapshot_id`, `idx_snapshot_edges_snapshot_id`, `idx_snapshot_nodes_snapshot_id`, and `idx_snapshot_membership_blobs_snapshot_id`; all are prefixes of their table primary keys
- [x] drop graph `idx_node_history_repo_id` and `idx_edge_history_repo_id`; both are prefixes of their table primary keys
- [x] drop session `idx_memoir_concepts_graph`; it duplicates the `memoir_concepts(graph_id, name)` unique autoindex
- [x] drop session `idx_memoir_graphs_repo_scope`; it is a prefix of the `memoir_graphs(repo_root, scope, name)` unique autoindex
- [x] drop session `idx_experience_edges_source`; it is a prefix of the `experience_edges(repo_root, source_uri, target_uri, query_text)` unique autoindex
- [x] drop session `idx_overview_records_repo`; it is a prefix of `idx_overview_records_subject` and every other repo-leading overview index
- [x] drop content `idx_chunks_source_id`; it is a prefix of the `chunks(source_id, chunk_index)` unique autoindex
- [x] drop content `idx_sources_session`; it is a prefix of `idx_sources_session_agent`

#### Remove indexes with no current query usage

- [x] drop graph `idx_commits_indexed_ref`, `idx_commits_author_time`, and `idx_commits_committer_time`; no query filters or orders by them alone after `idx_commits_repo_author_time` lands
- [x] drop graph `idx_graph_snapshots_commit_sha`; every snapshot lookup filters `repo_id` too and uses the unique `(repo_id, commit_sha)` index
- [x] drop graph `idx_snapshot_membership_blobs_file_hash`, `idx_snapshot_nodes_file_hash`, and `idx_snapshot_edges_file_hash` after confirming no history query will filter snapshot membership by `file_hash`
- [x] drop graph `idx_node_history_qualified_name`, `idx_node_history_introduction_commit`, `idx_node_history_removal_commit`, `idx_edge_history_source_qn`, `idx_edge_history_target_qn`, `idx_edge_history_introduction_commit`, and `idx_edge_history_removal_commit` unless planned history query work will use them
- [x] drop graph `idx_flows_kind`, `idx_communities_algorithm`, and `idx_communities_parent`; taxonomy queries fetch by id/name or list all rows
- [x] drop graph `idx_postprocess_state_state` and `idx_postprocess_state_updated_at_ms`; postprocess state is only read by `repo_root`
- [x] drop graph `idx_edges_kind` unless an explicit edge-kind-only query is added
- [x] drop session `idx_overview_records_commit`, `idx_overview_records_file`, `idx_overview_records_qname`, `idx_overview_records_freshness`, `idx_overview_records_subject_kind`, and `idx_overview_packets_repo`; overview filtering currently happens in Rust after `load_records`, and packets are read by `packet_id`
- [x] drop session `idx_decision_memory_session_updated`; every decision query includes `repo_root`
- [x] drop session `idx_memories_repo_accessed`; no query filters or orders by `last_accessed_at`
- [x] drop session `idx_memory_supersessions_new`; supersession rows are only inserted
- [x] drop content `idx_chunks_chunk_id`; every chunk-id lookup is paired with `source_id` and uses the `(source_id, chunk_id)` unique index
- [x] drop content `idx_sources_repo_id`; no query filters `sources.repo_id`

#### Planner and schema maintenance

- [x] added `Store::optimize_for_reads` (`packages/atlas-store-sqlite/src/store/maintenance.rs`) running `ANALYZE; PRAGMA optimize;`, called from the `run_metrics` build/update wrappers and from `postprocess.rs` on success; `update.rs` was left untouched because it exceeds the 1K LOC edit threshold
- [x] run `PRAGMA optimize` after `ANALYZE` in the same lifecycle path (`Store::optimize_for_reads`)
- [x] regenerate `packages/atlas-store-sqlite/src/migrations/schema.sql` with `packages/atlas-store-sqlite/examples/dump_schema.rs` after graph migration 021
- [x] add `021_graph_index_hot_paths` to the migration registry in `packages/atlas-store-sqlite/src/migrations.rs`
- [x] add `016_session_index_hot_paths` to the session migration registry
- [x] add `011_content_index_hot_paths` to the content-store migration registry

#### Validation

- [x] add a graph-store test that asserts `EXPLAIN QUERY PLAN SELECT ... FROM nodes WHERE name = ?` uses `idx_nodes_name_order`
- [x] add a graph-store test that asserts prefix `LIKE` uses `idx_nodes_name_nocase` and subpath `LIKE` uses `idx_nodes_file_path_nocase`
- [x] add a graph-store test that asserts `SELECT hash FROM files WHERE path = ?1` uses `idx_files_path`
- [x] add a session-store test that asserts the overview `ACTIVE_PREDICATE` plan contains no `SCAN newer`
- [x] add migration tests that assert redundant index names are absent from `sqlite_master` after upgrade
- [x] run `cargo test --quiet -p atlas-store-sqlite store::tests` after graph migration changes
- [x] run `cargo test --quiet -p atlas-session store::tests` after session migration changes
- [x] run `cargo test --quiet -p atlas-contentstore` after content migration changes
- [x] run `cargo fmt --all` and `cargo clippy --workspace --all-targets --all-features -- -D warnings` after the migration batch

#### Implementation notes and verified non-goals

- default `LIKE` plans only show the index range once the pattern parameter is bound, so plan tests must bind before `EXPLAIN QUERY PLAN`; unbound plans legitimately show `SCAN`
- `idx_files_path`, `idx_files_indexed_at`, `idx_edges_order`, `idx_commits_repo_author_time`, `idx_nodes_repo_file_path_order`, `idx_nodes_file_parent`, `idx_edges_repo_confidence_tier`, and `idx_retrieval_chunks_repo_embedded` were confirmed with bound-parameter plans on live-data copies plus `ANALYZE`
- `idx_memories_repo_active` is usable but the planner may still prefer the covering `idx_memories_repo_source` for `COUNT(*)` on a repo; the test accepts any repo-scoped `memories` index and only forbids a full scan
- `session_meta` is small enough that the planner sometimes scans plus sorts instead of using `idx_session_meta_updated_at`; the index stays because the cost is negligible and it serves the `updated_at < ?` cleanup cutoff
- `memoir_relations` repo-scoped OR-walks may still show `SCAN` while the table has no rows; both repo indexes exist for growth

---

### Atlas Tokenizer Patch

Full design: `docs/PLAN-TOKENIZER.md`. Goal: standalone `atlas-tokenizer` crate porting OpenAI `tiktoken`, opt-in provider first, default only after stability gates.

#### Phase 0 — Decisions

- [x] do not commit vocab assets; download from pinned upstream URLs like official `tiktoken`
- [x] reference exact upstream `tiktoken_ext/openai_public.py` and `tiktoken/load.py` by commit
- [x] pin upstream tiktoken commit `3591ff175d6a80efbe4fcc7f0e219ddd4b8c52f1`


#### Phase 1 — Crate Scaffold

- [ ] add `packages/atlas-tokenizer` with `Cargo.toml`, `README.md`, and `LICENSE-TIKTOKEN`
- [ ] add `atlas-tokenizer` to workspace members and `[workspace.dependencies]`
- [ ] add `#![forbid(unsafe_code)]` and `#![deny(missing_docs)]` to `packages/atlas-tokenizer/src/lib.rs`
- [ ] add `cargo check -p atlas-tokenizer --no-default-features` gate

#### Phase 2 — BPE Core

- [ ] add `packages/atlas-tokenizer/src/rank_file.rs` parser with malformed-input and size-cap tests
- [ ] add `packages/atlas-tokenizer/src/bpe.rs` byte-pair encode/decode with merge edge-case tests
- [ ] add property test asserting `decode_bytes(encode_ordinary(x)) == x.as_bytes()`

#### Phase 3 — Encodings and Model Map

- [ ] add encoding constants with exact upstream regex patterns and special tokens
- [ ] add `model.rs` longest-prefix model-to-encoding map
- [ ] add `registry.rs` explicit registry plus `OnceLock` default
- [ ] add `special_tokens.rs` and `model_map.rs` tests

#### Phase 4 — Download and Cache

- [ ] add `packages/atlas-tokenizer/src/assets.rs` with `UPSTREAM_COMMIT`, per-encoding URL + sha256, and exact upstream GitHub file references
- [ ] add `packages/atlas-tokenizer/src/download.rs` with HTTPS-only fetch, timeouts, redirect scheme check, 32 MiB cap, and sha256 verification
- [ ] add `packages/atlas-tokenizer/src/cache.rs` with upstream-compatible `sha1(url)` key, env dir precedence, atomic temp+rename, and corrupt-entry recovery
- [ ] add `download_cache.rs` tests with a local fixture HTTP server; no real network
- [ ] add `tokenizer_file` local path and `download` feature-off behavior
- [ ] add ignored upstream smoke test; verify warm-cache run works with networking disabled

#### Phase 5 — Parity and Bench

- [ ] add maintainer golden generator for official Python `tiktoken` at the pinned commit
- [ ] commit golden JSONL for all supported encodings
- [ ] add `tests/parity.rs` and reach zero mismatches for `cl100k_base` and `o200k_base`
- [ ] add criterion encode/decode bench and record baselines
- [ ] add tokenizer fuzz target under workspace `fuzz/`

#### Phase 6 — Integration

- [ ] add `Tiktoken` variants to `atlas_token_count::TokenCounter` and `TokenCountMethod`
- [ ] add `provider = "tiktoken"` to `ContextTokenizerConfig` with encoding/model/file validation
- [ ] build tiktoken counter in `Config::token_counter` with existing fallback semantics
- [ ] add optional `encoding` field to token accounting in `atlas-review/src/context/payload.rs`
- [ ] update config template, runtime config JSON, README, and `SHIPPED.md`
- [ ] add CLI and MCP tokenizer quality-gate cases for `provider = "tiktoken"`

#### Phase 7 — Stability Switch and Legacy Removal

- [ ] once atlas-tokenizer is stable, change `ContextTokenizerConfig::default()` to tiktoken and pick the default encoding from parity and bench results
- [ ] remove `provider = "heuristic"` as a selectable provider; keep byte-based counting only as automatic fallback unless stability review says otherwise
- [ ] remove `provider = "tokenizers"` and the `tokenizers` dependency
- [ ] remove the `paste` advisory ignore from `deny.toml` after dependency removal
- [ ] run full workspace tests, clippy, fmt, deny, and bench regression check

---

## Additional Backlog

- [ ] add CI-visible parser cache hit-ratio metric and fail when cache reuse drops below configured threshold
- [ ] add thin LSP shim that maps Atlas query/context/impact/reference flows onto standard LSP requests
- [ ] add a `memoir_delete` MCP/CLI surface so throwaway smoke-test concept graphs can be removed without direct DB edits

---

## Open Defects — MCP Tool Sweep (2026-11-13)

Run context: full `AGENTS_MCP_TESTS.md` sweep on `main` @ `f2a290b`, clean working tree, graph healthy (956 files / 26730 nodes / 103433 edges). 77 of 80 tools exercised. Tools 57 `purge_saved_context`, 79 `build_graph`, 80 `update_graph` were skipped as destructive. Each item states observed behavior, most likely root cause (confirm before editing), required fix, and the regression test to add.

### Retrieval and paging correctness

- [x] `read_saved_context`: return non-empty content and a strictly advancing `next_chunk_offset` when `max_bytes` is smaller than the first chunk. Fixed by emitting a UTF-8-safe prefix of the clipped chunk and advancing `next_chunk_offset` to `chunk_index + 1`; the response now also sets `chunk_clipped`. Tests cover `max_bytes=1` and `max_bytes=100`.
- [x] `get_context`: stop silently dropping callers when `token_budget` is small. Fixed the payload trim priority so `Caller`/`Callee` units outrank looser `ImpactNeighbor`/`TestAdjacent` units; node/edge/file drop counts are already exposed as `nodes_dropped`, `edges_dropped`, and `files_dropped`.
- [x] `find_similar_functions`: make the documented 0.40 similarity floor reachable, or lower the default. Default `similarity_low_threshold` is now `0.22`, matching the scorer's internal floor; a near-duplicate pair is found at the default threshold in `atlas-reasoning` tests.
- [x] `save_context_artifact` doc contract: aligned Tool 52 expectations with the inline threshold (about 360 B); payloads at or below it return `raw_inline` with `source_id:null` and `summary.stored:false`.

### Analysis quality

- [x] `analyze_patterns`: emit hub, bottleneck, and isolated-structure findings, not only repeated chains. Fixed by capping each category to a fair share before the score-ranked trim, so high-scoring repeated chains can no longer evict centrality/isolated/orphan/deep findings.
- [x] `postprocess_graph`: honor `changed_only` when the change set is empty. Now returns `noop:true`, `noop_reason:'no_changed_files'`, and empty `planned_stages` in both dry-run and real paths.
- [x] `postprocess_graph`: stage ids documented in Tool 25 now match the enum (`flows`, `communities`, `architecture_metrics`, `query_hints`, `large_function_summaries`).
- [x] `analyze_safety`: return structured symbol-not-found errors. The shared classifier now maps `node not found` to `symbol_not_found`; it also recognizes `unknown memoir relation`, ambiguous subject errors, and unknown overview subject kinds as `invalid_input`.
- [x] `memory_ops`: write an audit row for the initial store. `store_memory_at` now seeds `op_history_json` with a `store` entry in the same insert, so a fresh memory reports `entry_count:1`.

### Session and memory boundaries

- [x] `cross_session_search`: treat `agent_id` as a hard partition filter. `merge_agent_partitions` now defaults to `false`, and the response records `query.effective_agent_filter`; isolation is covered by a two-partition test.
- [x] `memory_store`: reject or persist `patch_file`. A plain store now persists `metadata.related_file`; the EUA patch path still uses the file as patch input.
- [x] `compact_session`: make repeated compaction idempotent and report `no_op`. `CurationResult` now carries `no_op`, derived from the actual delete/promote sets; a test asserts the second and third compactions are no-ops on an unchanged ledger.
- [x] `get_session_status`: report snapshot existence as false once consumed. `resume_snapshot_exists` is now derived from `!consumed`; a consume-then-status test covers the transition.

### Provenance and trust metadata

- [x] `status` / `atlas_provenance`: derive file and node counts from one source. `provenance_meta` and `explain_query` now use `graph_stats()`, which excludes synthetic metadata rows like `status`; a store test asserts equal counts.
- [x] Freshness disagreement: `pending_graph_relevant_changes` now diffs from the recorded indexed ref instead of only the working tree, so committed-but-unindexed changes are detected and `status`/`doctor` agree with overview freshness. A deleted recorded ref falls back to the working-tree diff.

### Doc drift — align `AGENTS_MCP_TESTS.md` with actual response shapes

- [x] Update Tool 52/53/47 field names: `has_resume_snapshot`→`resume_snapshot_exists`, `saved_source_count`→`source_count`, `frequently_accessed_symbols`/`frequently_accessed_files`→`frequent_symbols`/`frequent_files`, and `status=='ok'`→`summary.recorded=='ok'`.
- [x] Update Tool 34/30/25 field names: `line_count`→`loc`, `factor_contributions`→`top_factors`, `stages`→`planned_stages`; note that `assess_risk` with `verbose:false` still includes per-factor evidence.
- [x] Update Tool 11/24/49 shapes: token list lives at `tokenization.fts_tokens`, regex validity at `regex_plan.valid`, intent under `query_intent.kind` with `query_intent.intent` carrying the higher-level intent, `embed_chunks` echoes no `limit` field, and `compact_session` returns flat `before_counts`/`after_counts`/`removed_events`/`promoted_events` instead of `curation_stats`.
- [x] Update Tool 63/59 minor contracts: `experience_inspect` accepts `incoming` (not `inbound`) and rejects unknown directions as plain text; `memory_store` returns `id`/`scope`/`importance` nested under `memory` and only accepts enum importance values.
- [x] Update Tool 44 expectation: `get_docs_section` with an unrelated missing heading returns `resolved:false` plus `candidates:[]`; only near-miss headings produce suggestions.
- [x] Update Tool 76 expectation: `tool_help` on an unknown tool returns a plain `unknown MCP tool '<name>'` message with no suggestion list.
- [x] Refresh stale line references in Tools 42–43 so they point at current symbol locations (`classify_graph_store_error` L68, `is_schema_mismatch_error` L44 in `packages/atlas-core/src/health.rs`).
- [x] Fix the `analyze_patterns` and `find_similar_functions` expectations in Tools 33 and 36 to match category diversity and the default `0.22` similarity floor.

---

## Open Defects — MCP Tool Sweep (2026-11-13, run 2 @ `8db5186`)

Run context: full `AGENTS_MCP_TESTS.md` sweep with the graph healthy (962 files / 26877 nodes / 104034 edges, `doctor` 17/17, schemas graph 21/21, content 11/11, session 16/16). Tools 79 `build_graph` and 80 `update_graph` were skipped as destructive; `purge_saved_context` was attempted only in fail-closed forms. Suites ran per category (health, search, traversal, review/context, analysis A/B, content, embedding+introspection, session A/B, memoir, overview, resources/CLI). Items tagged **[verified]** were reproduced directly by the parent agent; the rest were observed by subagents and need a repro pass before editing.

### Confirmed this run

- [x] **fix(get_impact_radius)**: impact traversal returns no dependents for a file with real callers. **[verified]** `change_source={kind:files, files:["packages/atlas-core/src/health.rs"]}`, depth 2, max_nodes 20 → `changed_symbols` = 20, `affected_files`/`affected_nodes`/`impacted_symbols` = `[]`, `summary.seed_budget_count` = 0, `budget_hit:true` while `truncated:false`. `cross_file_links` reports `packages/atlas-core/src/readiness.rs` as a dependent, so the reverse tier filter is dropping every `same_package` dependent. Check the impact seed/tier selection in `packages/atlas-review/src/context/build.rs` and `packages/atlas-mcp/src/tools/context_ops/impact.rs`. **Fixed:** two root causes in `Store::impact_radius` / `Store::traverse_from_qnames`: (1) the recursive CTE capped total rows at `max_nodes`, so a 26-symbol seed file spent the whole budget on seeds; the CTE now over-reads `max_nodes + seed_count` and splits the budget between changed symbols and impact neighbours when the seed set meets the cap. (2) traversal output included expression-text edge targets (e.g. array literals) that are not graph nodes, so the first `LIMIT` rows were non-nodes and the real dependents were dropped; the CTE now keeps only qualified names that exist in `nodes`. `seed_budgets` reports seeds, and `atlas impact --files packages/atlas-core/src/health.rs --max-nodes 20` now returns dependents (`readiness.rs`, `kinds.rs`, ...). Regressions: `impact_radius_finds_dependents_when_seed_file_exceeds_node_cap`, `traverse_from_qnames_finds_dependents_when_seed_set_meets_node_cap`.
- [x] **fix(read_saved_context)**: indexed artifacts are returned lossily, so JSON is no longer reconstructable. **[verified]** saved a 956 B JSON payload (`source_id=c536ac758208e80e6df546d148d029af`); read returned 1067 B of concatenated chunk text: `"status":"ok"` → `"ok"`, `"nodes_checked":25000` → `25000`. Persist the raw payload at save time and serve it verbatim from `packages/atlas-mcp/src/session_tools/artifact_io.rs` instead of re-joining chunk bodies. **Fixed:** content migration 012 adds `sources.raw_content`; `index_artifact` persists the verbatim payload; `read_saved_context` serves it byte-for-byte and pages by byte offset (`content_verbatim:true`, `paging_unit:"bytes"`). Legacy rows keep the chunk-join path. Regression: `read_saved_context_round_trips_json_payload_verbatim`.
- [x] **fix(explain_query)**: reported tokenization disagrees with the real FTS index. **[verified]** `tokenization.fts_tokens` = `["classify_graph_store_error"]` (1 token), but `nodes_fts` splits `_`, so `MATCH '"classify_graph_store_error"'` and `MATCH '"classify" AND "store"'` both return rows (2 and 3 respectively). Query results are still correct; only the explanation is wrong. `tokenize_fts` in `packages/atlas-search/src/execution.rs:132` keeps `_` as a token char (`!c.is_alphanumeric() && c != '_'`). **Fixed:** `tokenize_fts` now mirrors the `unicode61` tokenizer and splits on `_`; the multi-token warning only fires for whitespace-separated text so snake_case identifiers do not produce noise. Regression: `tokenize_fts_splits_underscores_like_unicode61`.
- [x] **fix(search_files)**: `max_results` is declared but never read, and discovery listing is clamped below every declared limit. **[verified in source]** `packages/atlas-mcp/src/discovery/search_files.rs` has no `max_results` lookup (unlike `search_templates.rs:72` and `search_text_assets.rs:93`), and all three route through `budgets.resolve_limit(..., ReviewContextExtraction, ...)`, which caps at 20 even when 100 is requested. Observed: `truncated:true` at 20 with no paging arg. Decouple discovery listing from the review-context budget policy and register `max_results` for `search_files`. **Fixed:** `max_results` (default 100) now flows through `resolve_limit`, `summary.requested_limit` is emitted, and the input schema/description expose the arg. Truncation is now reported only when a matching file is actually dropped. Regression: `search_files_honors_requested_max_results_above_default`.
- [x] **fix(debug_graph)**: wire field names differ from the documented and CLI shapes. **[verified in source]** `packages/atlas-mcp/src/tools/health.rs:1157` emits `node_counts_by_kind` (and `edge_counts_by_kind`) while the test guide and `atlas debug-graph --json` use `nodes_by_kind` / `edges_by_kind`. Align the MCP payload and `registry/output_schemas/health.rs`. **Fixed:** payload, output schema, and test now use `nodes_by_kind` / `edges_by_kind`. The tool was extracted to `packages/atlas-mcp/src/tools/graph_debug.rs` because `tools/health.rs` exceeded the 1K LOC rule.
- [x] **fix(purge_saved_context)**: tool cannot run in clients that do not advertise `elicitation.form`. Observed plain-text error `client does not advertise elicitation.form capability` for all three Tool 57 cases, plus `atlas session purge` is not a CLI subcommand. Either gate the tool with an explicit capability requirement in the registry/`tool_help` output, or allow a non-interactive purge path for bounded `keep_days` cleanup. (`packages/atlas-mcp/src/elicitation.rs:30`) **Fixed:** added `confirm: true` for non-interactive age-based cleanup; when the client lacks `elicitation.form` and `confirm` is absent the tool now returns a structured `invalid_input` error with retry guidance instead of a bare capability string. `purge_bridge_files` and `confirm` are now in the typed input schema. `atlas session purge` remains unadded (covered by the session CLI surface item).
- [x] **fix(save_context_artifact)**: sanitizer rewrites benign repeated payloads before routing. **[verified]** a payload containing four 64-char repeated runs came back with every value replaced by `[REDACTED]`, and a 451 B payload stayed `raw_inline` although the guide documents the inline cutoff at ~360 B. Confirm the redaction heuristic and the real `route_output` threshold in `packages/atlas-mcp/src/session_tools/saved_context.rs:447` (against the content store config). **Fixed (observability + doc drift):** redaction is intentional for 32+ hex runs, but the tool now reports `redaction_applied` plus a warning so silent payload rewrites are visible. The real inline cutoff is `ContentStoreConfig.small_output_bytes` (512 B), not ~360 B; the test guide was corrected.

### Session continuity and memory

- [x] **fix(compact_session)**: repeated compaction is not a no-op, contradicting the earlier fix note above. Observed second call 33→32 events, `decayed_events`/`removed_events` = 2, `no_op:false`. Compaction appears to append curation events that the next pass decays. `packages/atlas-session/src/store/curation.rs`. **Fixed:** compaction now short-circuits to `no_op:true` when no event is newer than `session_meta.last_compaction_at`, so an unchanged ledger cannot decay the summary row it just wrote. A ledger that genuinely received new events still compacts (the 33→32 observation came from events recorded between calls).
- [x] **fix(resume_session)**: a consumed snapshot row still suppresses rebuilds, so `get_session_status.resume_snapshot_exists` reports `false` while `resume_session` keeps serving a stale `existing_snapshot` (live anchor from 2026-11-12 with event_count 24 against 62+ live events). `packages/atlas-session/src/store/resume.rs`. **Fixed:** `resume_session` only reuses a snapshot when `consumed == false`; otherwise it rebuilds from live events (`snapshot_status:"built_snapshot"`). Regression: `resume_session_rebuilds_after_snapshot_consumed`.
- [x] **fix(search_saved_context)**: each query writes a decision memory (`reuse prior decision during saved-context lookup: …`), which crowds real decisions in `wake_up` (7/7 recalled decisions were these rows). `packages/atlas-mcp/src/session_tools/saved_context.rs:304`. **Fixed:** the side-effect write was removed; linked decisions are still returned in the response payload.
- [x] **fix(wake_up CLI)**: `atlas wake-up --json` omits `summary.recorded`, `event_recorded`, and `agent_id`, and never writes a session-start event, so MCP and CLI diverge. `packages/atlas-cli/src/commands/wake_up.rs`. **Fixed:** the CLI now records `session-start` through the shared event service and emits `agent_id`, `event_recorded`, and `summary.recorded`; `AgentEventSource::CliFallback` was added for source attribution. Golden `wake_up.json` updated.
- [x] **fix(session CLI surface)**: `session save-artifact`, `read-artifact`, `search`, and `stats` subcommands are absent; `session compact`/`resume` accept no `--session` selector; `session status`/`compact` JSON shapes differ from MCP. `packages/atlas-cli/src/cli/subcommands.rs`, `packages/atlas-cli/src/commands/session.rs`. **Fixed:** all four subcommands were added and delegate to `atlas_mcp::cli_tools` wrappers so argument contracts and payloads match MCP; `--session` was added to `status`, `resume`, and `compact`; `session compact` now prints the MCP payload. `session status` keeps its CLI-oriented shape (includes global memory summary) — flag if strict shape parity is required.
- [x] **fix(memory CLI surface)**: `atlas memory global` / `atlas memory decisions` do not exist (decisions live under `atlas session decisions`), `atlas memory ops <id> --json` exits 1 with no `found` field, `atlas memory store` takes text positionally and rejects the documented flag form, and `atlas experience <uri>` requires the `inspect` subcommand. **Fixed:** added `memory global` and `memory decisions` (MCP-backed), `memory ops` now returns `{found:false}` instead of exiting 1, `memory store` accepts `--text`, and `atlas experience <uri>` is a shorthand for `experience inspect <uri>`.
- [x] **fix(experience_inspect)**: `experience_edges` has 0 rows repo-wide, so Tool 63 assertions are unexercisable; no code path currently records experience edges. Either wire a producer or document the surface as report-only/empty until populated. **Documented:** the registry description now states the ledger stays empty until a retrieval/update workflow records edges and that an empty `edges` list is a valid report. No producer was wired (that needs a separate retrieval-workflow design).

### Review context and change analysis

- [x] **fix(get_review_context)**: `exact_dir` is parsed into the request but never applied to changed-file seeds; a `packages/atlas-mcp` scope still returned 32 out-of-scope files. `packages/atlas-review/src/context/build.rs`. **Fixed:** `build_review_context` now prunes `changed_paths` through `ContextScope` before seeding traversal and reports the excluded count in the `changed_files` seed budget; the MCP adapter mirrors the same scope filter on `changed_files` and its independent impact query. Regression: `review_context_exact_dir_prunes_changed_file_seeds`.
- [x] **fix(get_review_context)**: `impacted_neighbors` and `critical_edges` are never populated, and truncation accounting is inconsistent (`emitted_node_count:40`, `emitted_edge_count:70` against an 8-node / 0-edge payload). **Fixed:** the neighbor/edge derivation was already present; the empty result came from the impact-traversal bugs above. `traversal_budget.emitted_node_count` / `emitted_edge_count` are now recomputed from the final packaged arrays after response-budget trimming, and `critical_edges` mirrors the emitted edges. Regression: `get_review_context_reports_neighbors_and_consistent_traversal_counts`.
- [x] **fix(get_minimal_context/get_review_context/explain_change)**: changed-symbol mapping has no per-file fair share, so markdown heading nodes always win and `.rs` files contribute zero changed symbols on a 32-file delta. **Fixed:** `Store::impact_radius` orders seed nodes deterministically and round-robins the seed budget across changed files (`fair_share_nodes_by_file`), so a docs file cannot consume every changed-symbol slot. Regressions: `impact_radius_fair_shares_seed_budget_across_changed_files`, `review_context_fair_shares_changed_symbols_across_files`.
- [x] **fix(explain_change)**: markdown headings count as untested symbols, producing false `coverage_gaps` and escalating a docs-only delta to `risk:medium`; payload also exceeds the 32 KB cap (40 KB) while `max_nodes` does not bound it. **Fixed:** uncovered changes are restricted to testable symbol kinds (`is_testable_change_kind`), and the explain payload now runs through `enforce_mcp_response_budget` with the resulting budget injected into the response. Regressions: `docs_only_change_reports_no_uncovered_symbols`, `explain_change_docs_only_delta_reports_no_coverage_gaps`.
- [x] **fix(get_context)**: for a multi-file target the docs asset consumes the whole merged node budget (`file_count:1` vs 2 files), and `linked_decisions` is charged inside `token_budget`, evicting graph callers. **Fixed:** per-file seed fairness keeps both files represented (see the review-context item), and linked decisions are attached only after the response budget is enforced, accepting only rows that fit in the remaining headroom and warning when any are omitted. Regression: `review_context_fair_shares_changed_symbols_across_files`.

### Analysis quality

- [x] **fix(analyze_patterns)**: the per-category fair-share cap is computed from `config.max_findings/4` rather than the requested limit, so a small `limit` (3-5) still returns only `pattern_repeated_chain`; diversity appears only at `limit:50`. `packages/atlas-reasoning/src/engine/insights.rs`. **Fixed:** `pattern_report_with_limit` (and `analyze_patterns_with_limit`) derive the per-category cap from `min(requested_limit, config.max_findings)`; MCP and CLI pass the requested page size. Regression: `pattern_report_limit_drives_category_fair_share`.
- [x] **fix(find_complex_functions)**: `mode=complex` still reports `threshold_kind:"loc"` / `governing_threshold:80` and omits `details`, even when every hit is gated by cyclomatic or cognitive complexity. `packages/atlas-reasoning/src/engine/large_functions.rs`. **Fixed:** `governing_threshold` only considers metrics that actually met their threshold and picks the largest excess, so a complex-only hit reports its complexity metric; candidates are still emitted in the compact payload. Regression added to `large_function_file_scope_and_mode_filtering_work`.
- [x] **fix(find_similar_functions)**: the code default is `0.22` but `.atlas/config.toml:77` still sets `similarity_low_threshold = 0.4`, so the default call returns zero matches and the hint text still advertises the 0.4 floor. Decide whether the config value is intentional. **Fixed:** the local `.atlas/config.toml` now uses the documented default `0.22`; the hint text reads the effective threshold from config, so it follows automatically.
- [x] **fix(analyze_dead_code)**: `summary.blocker_count` (222) does not match the emitted `blockers`/`candidates[].blockers` arrays; count and arrays must come from one source. **Fixed:** `blocker_count` now reports the emitted blocker rows (0 in summary mode), while `total_blocker_count` keeps the full-set total. Regression: `analyze_dead_code_blocker_count_matches_emitted_blockers`.
- [x] **fix(analyze_architecture)**: `verbose:false` still returns 50 findings (~17 KB MCP, ~44 KB CLI) while claiming `details_omitted:true`; module couplings are only counts, never listed. **Fixed:** both compact and verbose payloads now list bounded `modules` (coupling, cohesion, in/out dependency counts) and `module_edges` (aggregated evidence counts), and the output schema allows the new fields. Regression: `analyze_architecture_compact_lists_module_couplings`.
- [x] **fix(analyze_metrics)**: `limit:5` yields only 3 unique symbols because `distribution` and `node_metrics` rows duplicate each other, no cyclomatic metric is surfaced, and `graph_summary` reports 13008 nodes / 30847 edges against the 26877 / 104034 health counts (scope undocumented). **Fixed:** distribution outliers that duplicate a node-metric row are skipped, `metric_distributions` (including cyclomatic) is emitted in compact and verbose payloads, and `graph_summary` now reports `scope:"post_filter"` plus `raw_node_count`/`raw_edge_count`. Regression: `analyze_metrics_compact_surfaces_distributions_and_scope`.
- [x] **fix(analyze_dependency)**: suggested cleanups are file-only strings, duplicated, and carry no caller qualified name. **Fixed:** blocking references are deduped by qualified name and each cleanup names the caller and its file. Regression updated in `dependency_removal_blocked_by_reference`.
- [x] **fix(list_symbols)**: synthetic Cargo package owners are unreachable. The DB holds 25 `package::cargo:packages/*/Cargo.toml` rows, but `SymbolFilter.include_synthetic=false` excludes `.atlas/synthetic%` (`packages/atlas-store-sqlite/src/store/graph.rs:316`) and no MCP/CLI surface exposes the flag. Add `include_synthetic` to the registry descriptor, `tools/graph.rs::list_symbols`, and the CLI. **Fixed:** `include_synthetic` is wired through the MCP tool, its input/output schemas, and `atlas symbols --include-synthetic`. Regression: `list_symbols_include_synthetic_opt_in`.
- [x] **fix(concept_clusters)**: a multi-seed run returns a pseudo-cluster whose members are exactly the seed files; also normalize `coupling_metric.total_strength` so empty results report `0.0` instead of `-0.0`. **Fixed:** seed files are no longer emitted as cluster members, and `cross_file_links` coupling metrics use a negative-zero-normalizing rounder. Regressions: `cluster_by_shared_symbols_never_returns_seed_only_clusters`, `round_metric_normalizes_negative_zero`.
- [x] **fix(label_components)**: `packages/atlas-core/src/health.rs::fn::select_graph_health_error_code` is labeled only `diagnostics` (0.85) instead of keeping the `core`/path label too. **Fixed:** `packages/atlas-core/` now contributes a `core` label at 1.0 alongside path-derived labels. Regression: `label_components_keeps_core_label_alongside_diagnostics`.

### Overview, memoir, resources

- [x] **fix(overview_export)**: a canonical file path subject is always ambiguous because `resolve_subjects` matches both `by_qname` and `by_file` and ignores `request.scope`; file subjects therefore export nothing. Caller/callee arrays also include a file node and an expression call target. **Fixed:** subject resolution is scope-aware, file-node qualified names no longer collide with file subjects, and caller/callee arrays drop file/import nodes and unresolvable expression targets. Regressions: `package_export_and_ambiguous_subject_failure`, `overview::tests` export coverage.
- [x] **fix(overview_ingest)**: a string `subject` is rejected (`invalid type: string, expected struct OverviewSubject`) although the guide documents a string, and inline `analysis` JSON is treated as a filesystem path alias. Also `body_routed:true` disagrees with `budget.source_body_routed:false`. **Fixed:** `OverviewSubjectInput` accepts structured subjects or file/qname/package/module references, inline JSON is accepted for `analysis`/`input`, and routing metadata is written into the store-owned budget block. Regression: `ingest_accepts_string_subject_references`.
- [x] **fix(overview_show)**: an unknown subject returns `matches:[]` with no candidate suggestions even for near-miss tags, and the CLI error path prints plain text with exit 1 instead of a structured envelope. **Fixed:** misses return near-match records plus `candidates`/`retry_guidance`; CLI JSON delegates to the MCP envelope and text mode prints candidates without a hard error.
- [x] **fix(overview_refresh_plan)**: `apply:true` reports `applied_count:1` while no freshness value changes (only `updated_at` moves); the count should reflect real transitions. **Fixed:** freshness updates are skipped when the target state already matches, and `applied_count` only counts real transitions.
- [x] **fix(uri/resources symbol route)**: `atlas://repo/{repo_id}/symbol/{qn}` rejects slash-containing qualified names (`-32602 unknown repo uri form`), `repo/current` is not resolved as an alias, symbol views read `match`/`symbol` instead of `best_match` (all metadata null, empty L2 `read_hint`), and `%20` in a docs heading is not decoded. **Fixed:** symbol tails are joined, `repo/current` resolves to the active repo, symbol views read `best_match`, and docs URI segments are percent-decoded. Regression: `parse_symbol_uri_accepts_raw_qualified_name_paths`.
- [x] **fix(provenance resource)**: `atlas://graph/provenance` exposes no `repo_id` and no `commit_sha`. **Fixed:** `inject_provenance` now stamps `repo_id` and `commit_sha` on every provenance payload.
- [x] **fix(CLI surface gaps)**: `communities detect` is absent and `postprocess --stage communities` yields 0 communities for 26877 nodes; `atlas history` has the wrong shape and an empty index with an opaque error; `atlas uri` requires a `read` subcommand and rejects `repo/current`; `atlas flows create` rejects the documented `--name` flag and has no trace command; `feedback adjust` is absent. **Fixed:** Louvain detection is wired into `communities detect` and the postprocess stage, `history <file> --symbol` shorthand and empty-index guidance were added, `atlas uri <uri>` works without `read`, `flows create --name` and `flows trace` exist, and `feedback adjust` reports deterministic counters. Regression: `louvain_separates_two_dense_cliques`.
- [x] **fix(memoir CLI args)**: `link` requires `--relation` (positional rejected), `inspect` takes graph via `--graph` instead of positionally, and `add-concept` requires `--description` where MCP treats it as optional. **Fixed:** `link` accepts positional or flag relations, `inspect` accepts `<GRAPH> <CONCEPT>` or `<CONCEPT> --graph`, and `add-concept` treats description as optional. Regressions: `parse_memoir_link_accepts_positional_relation`, `parse_memoir_inspect_accepts_graph_positionally`.
- [x] **fix(memoir errors)**: memoir failures return bare text instead of a structured error payload with `code`/`retry_guidance`, although `tool_help` advertises those fields. **Fixed:** memoir misses/exists errors classify as invalid input with memoir-specific retry guidance.

### CLI parity

- [x] **fix(status parity)**: MCP `retrieval_index` omits `content_schema_version` / `content_schema_supported` / `content_schema_ok`, which the CLI emits. **Fixed:** the MCP summary runs the shared content-schema probe and emits all three fields.
- [x] **fix(get_metrics naming)**: histograms expose `mean`/`max`/`p50..p99` while the guide documents `mean_ms`/`max_ms`; `get_metrics` also carries 4-6k tokens with no verbose switch. **Fixed:** histogram summaries serialize `min_ms`/`max_ms`/`mean_ms`; compact mode omits bucket arrays and `verbose: true` restores them. Regression: `compact_snapshot_omits_histogram_buckets`.
- [x] **fix(CLI JSON envelopes)**: `resolve_symbol`, `query_graph`, `explain_query`, `get_docs_section`, `get_impact_radius`, `get_review_context`, `overview_search`, and the whole `insights` family emit different `data` shapes from MCP `structuredContent`. **Fixed for** `resolve_symbol`, `query_graph`, `explain_query`, `get_docs_section`, `overview_search`, and all `insights` subcommands, which now emit the MCP `structuredContent` payload (with narrow legacy aliases where the CLI contract still requires them). `impact` and `review-context` intentionally keep their richer CLI workflow/multi-repo envelope because MCP does not expose those fields yet.
- [x] **fix(CLI payload budgets)**: `atlas insights patterns --limit 3` emits 2.6 MB (10.5 MB at `--limit 50`), `infer-modules` 1.21 MB, `duplicates` 415 KB, `reasoning remove` 366 KB - none honor a byte budget, unlike the MCP 32 KB cap. **Fixed:** insight JSON now routes through the MCP 32 KB ceiling; `analyze remove`/`refactor remove-dead` use a shared bounded JSON printer that halves oversized arrays and marks `payload_truncated`.
- [x] **fix(discovery limit parity)**: MCP discovery reports `result_limit:20` while CLI content commands report `100` for the same query. **Fixed:** discovery tools honor the requested `max_results` directly (default 100) instead of inheriting the review-context cap. Regression: `search_content_honors_requested_max_results_without_review_budget_clamp`.

### Latency and protocol

- [ ] **fix(MCP worldtree latency)**: worldtree-backed MCP calls take ~5.2-5.3 s from a direct-stdio harness (`status` 5275 ms, `get_docs_section` 5260 ms, health resource 5303 ms) while the same CLI commands take 89-350 ms. CLIs are also compute-bound at 7.5-36 s for `insights risk`/`architecture`/`patterns`/`duplicates`/`similar-functions`. Profile the dispatch readiness gate and store open path in `packages/atlas-mcp/src/tools/dispatch.rs`.
- [ ] **fix(MCP protocol negotiation)**: a direct-stdio `initialize` requesting `2026-07-28` was answered with `2025-11-25`, although `MCP_PROTOCOL_VERSION` is `2026-07-28` and both versions are supported (`packages/atlas-mcp/src/spec.rs`). Reproduce with a raw client and confirm the handler's selection logic (`rmcp_server/handler.rs`).

### Doc drift — rerun after the shapes above settle

- [ ] Tool 5/12 thresholds are stale: `struct` 928 (expected >1000), `references` 2570 (expected >10000), rust nodes 11363 (expected >15000). Restate them relative to observed parser output instead of absolute floors.
- [ ] Tools 44/67/68/69/77 carry invalid CLI invocation forms: `docs-section` needs `--heading`, `memoir link` needs `--relation`, `memoir inspect` needs `--graph`, `overview export` needs `--subject`, `man` requires `<NAMESPACE> <TOOL_NAME>`.
- [ ] Tools 6.6/6.8/6.9/6.10 document flags that do not exist (`flows create --name`, `history <file> --symbol`, `uri <URI>`, `feedback adjust`) and Tool 6.5 references an absent fixture graph.
- [ ] Tool 51.2 says events land in `.atlas/context.db`; they are written to `.atlas/session.db`.
- [ ] Tool 47.3 states a total item cap while `wake_up.max_items` is per-list.
- [ ] Tool 67 says `relates_to`; the whitelist uses `related_to`.
- [ ] Tool 14 claims counts must match a CLI traversal, but no CLI subcommand exposes `traverse_graph`'s node/edge/budget shape.
- [ ] Tool 52 inline cutoff: observed `raw_inline` at 451 B and `indexed_preview` at 956 B, so the documented ~360 B figure is wrong.

### Test-run leftovers (cleanup pending)

- [ ] remove sweep fixtures: memories `mcp-sweep-*`, memoir graphs `mcp-sweep-arch|dup|cli|session`, overview records `10f4a60b…`/`f609da2a…`/`2c7d0ad4…`, saved artifacts `mcp-sweep-*` (incl. `c536ac758208e80e6df546d148d029af`), sessions `mcp-sweep-48a`/`50c`/`57b`, and flow `mcp-sweep-ReadinessHealthFlow` (id 1) in `worldtree.db`; `memoir_delete` does not exist yet (see Additional Backlog).
