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
