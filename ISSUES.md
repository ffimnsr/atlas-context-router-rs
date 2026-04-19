# Atlas — Detailed Rust TODO for a code-review-graph Reimplementation

## Goal

Reimplement the core of `code-review-graph` in Rust with a cleaner architecture than the current Python monolith.

The primary behavior to preserve is:

- build a repository code graph
- incrementally update it from git diffs
- persist graph data in SQLite
- query graph structure and impact radius
- assemble review context from changed files and neighboring nodes
- expose a CLI first, with MCP later

The upstream repo’s real kernel is the repository scanner, parser layer, SQLite graph store, incremental updater, impact analysis, review-context/query layer, and thin transport surfaces. Flows, communities, embeddings, visualization, wiki, registry, install automation, and similar extras are secondary layers and should not block v1.

---

## Product Name and CLI

- [ ] Use binary name: `atlas`
- [ ] Use hidden work dir: `.atlas/`
- [ ] Use DB path: `.atlas/worldview.sqlite`
- [ ] Use config path later: `.atlas/config.toml`
- [ ] Use CLI commands:
  - [ ] `atlas init`
  - [ ] `atlas build`
  - [ ] `atlas update`
  - [ ] `atlas detect-changes`
  - [ ] `atlas status`
  - [ ] `atlas query`
  - [ ] `atlas impact`
  - [ ] `atlas review-context`
  - [ ] `atlas serve` (later, MCP/stdin or JSON-RPC style)

---

## Phase 0 — Core Architecture Decisions

### 0.1 Freeze v1 scope

- [ ] Include in v1:
  - [ ] repo root detection
  - [ ] tracked-file collection
  - [ ] git diff change detection
  - [ ] parser abstraction
  - [ ] first language handlers
  - [ ] SQLite graph store
  - [ ] batch file graph replacement
  - [ ] recursive SQL impact traversal
  - [ ] review context assembly
  - [ ] FTS5 keyword search
  - [ ] CLI
- [ ] Explicitly defer:
  - [ ] embeddings
  - [ ] communities
  - [ ] flows
  - [ ] wiki
  - [ ] visualization/export
  - [ ] multi-repo registry
  - [ ] install hooks
  - [ ] auto-watch mode
  - [ ] refactor/apply-refactor
  - [ ] evaluation harness
  - [ ] cloud providers

### 0.2 Freeze compatibility policy

- [ ] Preserve upstream behavior where it matters:
  - [ ] qualified-name semantics
  - [ ] incremental build/update flow
  - [ ] SQLite-first persistence
  - [ ] impact radius from changed-file seed nodes
  - [ ] review/query usefulness
- [ ] Permit deliberate redesign where it improves maintainability:
  - [ ] split giant parser into per-language modules
  - [ ] split graph store/query/review into separate crates/modules
  - [ ] use repo-relative paths internally instead of absolute paths where possible
- [ ] Document every intentional compatibility break

### 0.3 Choose Rust crate strategy

- [x] Start with one Cargo workspace
- [x] Create crates:
  - [x] `packages/atlas-cli`
  - [x] `packages/atlas-core`
  - [x] `packages/atlas-repo`
  - [x] `packages/atlas-parser`
  - [x] `packages/atlas-store-sqlite`
  - [x] `packages/atlas-search`
  - [x] `packages/atlas-review`
  - [x] `packages/atlas-impact`
  - [ ] `packages/atlas-mcp` (later)
- [x] Keep public API narrow between crates

---

## Phase 1 — Rust Project Foundation

### 1.1 Create workspace

- [x] `cargo new --bin packages/atlas-cli`
- [x] `cargo new --lib packages/atlas-core`
- [x] `cargo new --lib packages/atlas-repo`
- [x] `cargo new --lib packages/atlas-parser`
- [x] `cargo new --lib packages/atlas-store-sqlite`
- [x] `cargo new --lib packages/atlas-search`
- [x] `cargo new --lib packages/atlas-review`
- [x] `cargo new --lib packages/atlas-impact`

### 1.2 Choose core dependencies

- [x] CLI:
  - [x] `clap`
  - [ ] `clap_complete` (later)
- [x] Errors:
  - [x] `thiserror`
  - [x] `anyhow` for CLI layer only
- [x] Serialization:
  - [x] `serde`
  - [x] `serde_json`
- [x] Paths/hash/time:
  - [x] `camino`
  - [ ] `sha2`
  - [ ] `time`
- [x] SQLite:
  - [x] `rusqlite` with bundled SQLite + FTS5 support
- [x] Logging:
  - [x] `tracing`
  - [x] `tracing-subscriber`
- [ ] Concurrency:
  - [ ] `rayon` or `crossbeam`
  - [ ] prefer std threads first if simpler
- [ ] Tree-sitter:
  - [ ] `tree-sitter`
  - [ ] language crates as needed
- [ ] Git integration:
  - [ ] start with `std::process::Command`
  - [ ] consider `git2` later only if necessary

### 1.3 CI and quality gates

- [ ] Add:
  - [ ] `cargo fmt --check`
  - [ ] `cargo clippy --all-targets --all-features -- -D warnings`
  - [ ] `cargo test --workspace`
- [ ] Add Linux CI
- [ ] Add SQLite/FTS5 smoke test in CI
- [ ] Add fixture-based regression tests

---

## Phase 2 — Domain Model

The current project is fundamentally a code graph persisted in SQLite, with nodes, edges, metadata, impact traversals, and FTS-backed search. Preserving that data model is one of the strongest parity choices for the rewrite.

### 2.1 Define node kinds

- [x] `File`
- [x] `Package`
- [x] `Module`
- [x] `Import`
- [x] `Class`
- [x] `Interface`
- [x] `Struct`
- [x] `Enum`
- [x] `Function`
- [x] `Method`
- [x] `Variable`
- [x] `Constant`
- [x] `Trait`
- [x] `Test`

### 2.2 Define edge kinds

- [x] `Contains`
- [x] `Imports`
- [x] `Calls`
- [x] `Defines`
- [x] `Implements`
- [x] `Extends`
- [x] `Tests`
- [x] `References`
- [x] `TestedBy`

### 2.3 Define `Node`

- [ ] Create `NodeId` type
- [x] Include:
  - [x] `id: i64`
  - [x] `kind: NodeKind`
  - [x] `name: String`
  - [x] `qualified_name: String`
  - [x] `file_path: String`
  - [x] `line_start: u32`
  - [x] `line_end: u32`
  - [x] `language: String`
  - [x] `parent_name: Option<String>`
  - [x] `params: Option<String>`
  - [x] `return_type: Option<String>`
  - [x] `modifiers: Option<String>`
  - [x] `is_test: bool`
  - [x] `file_hash: String`
  - [x] `extra_json: serde_json::Value`

### 2.4 Define `Edge`

- [x] Include:
  - [x] `id: i64`
  - [x] `kind: EdgeKind`
  - [x] `source_qn: String`
  - [x] `target_qn: String`
  - [x] `file_path: String`
  - [x] `line: Option<u32>`
  - [x] `confidence: f32`
  - [x] `confidence_tier: Option<String>`
  - [x] `extra_json: serde_json::Value`

### 2.5 Define supporting types

- [x] `FileRecord`
- [x] `GraphStats`
- [x] `ChangedFile`
- [x] `ImpactResult`
- [x] `ReviewContext`
- [x] `SearchQuery`
- [x] `ScoredNode`

---

## Phase 3 — SQLite Schema and Store

The upstream implementation already treats SQLite as the durable center of the system, with WAL mode, explicit transactions, and atomic file-slice replacement. That should be preserved.

### 3.1 Open database and pragmas

- [x] Create DB at `.atlas/codegraph.sqlite`
- [x] On open, set:
  - [x] `PRAGMA journal_mode=WAL;`
  - [x] `PRAGMA synchronous=NORMAL;`
  - [x] `PRAGMA foreign_keys=ON;`
  - [x] `PRAGMA busy_timeout=5000;`
- [x] Use one write connection policy for mutation-heavy operations
- [ ] Add startup integrity check command later

### 3.2 Migrations

- [x] Create migration runner
- [x] Add schema version table
- [x] Make migrations idempotent
- [ ] Add golden-schema tests

### 3.3 Tables

- [x] `metadata`
- [x] `files`
- [x] `nodes`
- [x] `edges`
- [x] `nodes_fts`
- [ ] reserve later:
  - [ ] `flows`
  - [ ] `flow_memberships`
  - [ ] `communities`

### 3.4 `metadata` table

- [x] `key TEXT PRIMARY KEY`
- [x] `value TEXT NOT NULL`

### 3.5 `files` table

- [x] `path TEXT PRIMARY KEY`
- [x] `language TEXT`
- [x] `hash TEXT NOT NULL`
- [x] `size INTEGER`
- [x] `indexed_at TEXT NOT NULL`

### 3.6 `nodes` table

- [x] `id INTEGER PRIMARY KEY`
- [x] `kind TEXT NOT NULL`
- [x] `name TEXT NOT NULL`
- [x] `qualified_name TEXT NOT NULL UNIQUE`
- [x] `file_path TEXT NOT NULL`
- [x] `line_start INTEGER`
- [x] `line_end INTEGER`
- [x] `language TEXT`
- [x] `parent_name TEXT`
- [x] `params TEXT`
- [x] `return_type TEXT`
- [x] `modifiers TEXT`
- [x] `is_test INTEGER NOT NULL DEFAULT 0`
- [x] `file_hash TEXT`
- [x] `extra_json TEXT`

### 3.7 `edges` table

- [x] `id INTEGER PRIMARY KEY`
- [x] `kind TEXT NOT NULL`
- [x] `source_qualified TEXT NOT NULL`
- [x] `target_qualified TEXT NOT NULL`
- [x] `file_path TEXT`
- [x] `line INTEGER`
- [x] `confidence REAL DEFAULT 1.0`
- [x] `confidence_tier TEXT`
- [x] `extra_json TEXT`

### 3.8 Indexes

- [x] `idx_nodes_kind`
- [x] `idx_nodes_file_path`
- [x] `idx_nodes_qualified_name`
- [x] `idx_nodes_language`
- [x] `idx_edges_kind`
- [x] `idx_edges_source`
- [x] `idx_edges_target`
- [x] `idx_edges_file_path`

### 3.9 FTS5 table

- [x] Create `nodes_fts` virtual table
- [x] Index:
  - [x] `qualified_name`
  - [x] `name`
  - [x] `kind`
  - [x] `file_path`
  - [x] `language`
  - [x] `params`
  - [x] `return_type`
  - [x] `modifiers`
- [x] Start with FTS only
- [x] Keep hybrid/vector search out of v1

### 3.10 Store API

- [x] `open(path)`
- [x] `migrate()`
- [ ] `replace_file_graph(file_path, file_hash, nodes, edges)`
- [ ] `replace_batch(parsed_files)`
- [ ] `delete_file_graph(file_path)`
- [ ] `nodes_by_file(file_path)`
- [ ] `edges_by_file(file_path)`
- [ ] `find_dependents(changed_files)`
- [ ] `impact_radius(changed_files, max_depth, max_nodes)`
- [ ] `search(query)`
- [x] `stats()`

### 3.11 Transaction semantics

- [ ] Replace one file graph atomically:
  - [ ] begin immediate transaction
  - [ ] delete old FTS rows
  - [ ] delete old edges for file
  - [ ] delete old nodes for file
  - [ ] upsert file row
  - [ ] insert nodes
  - [ ] insert edges
  - [ ] insert FTS rows
  - [ ] commit
- [ ] Add rollback tests
- [ ] Add lock-contention tests

---

## Phase 4 — Repository Scanner and Git Diff

The upstream project’s primary promise includes full build plus incremental update from git diff. That makes repo scanning and change detection part of the actual product kernel, not glue code.

### 4.1 Repo root detection

- [ ] Implement `find_repo_root(start: &Utf8Path) -> Result<Utf8PathBuf>`
- [ ] First try `git rev-parse --show-toplevel`
- [ ] Fallback: walk parent dirs for `.git`
- [ ] Normalize returned path

### 4.2 Path normalization

- [ ] Convert to repo-relative paths for persistence
- [ ] Normalize separators to `/`
- [ ] Resolve `.` and `..`
- [ ] Decide symlink policy
- [ ] Add Windows casing normalization tests

### 4.3 Ignore handling

- [ ] Support git-tracked files first via `git ls-files`
- [ ] Add `.atlasignore` later
- [ ] Respect upstream-style ignore file compatibility later if needed
- [ ] Ignore by default:
  - [ ] `.git`
  - [ ] `node_modules`
  - [ ] `vendor`
  - [ ] `dist`
  - [ ] `build`
  - [ ] `.next`
  - [ ] `target`
  - [ ] `.venv`
  - [ ] `__pycache__`

### 4.4 File collection

- [ ] `collect_files(repo_root)`
- [ ] Use `git ls-files`
- [ ] Optional recursive submodule handling later
- [ ] Skip unsupported extensions
- [ ] skip binary files
- [ ] skip giant files
- [ ] configurable file size threshold

### 4.5 File hashing

- [ ] SHA-256 file hash
- [ ] skip unchanged files on full build
- [ ] persist hash in `files`

### 4.6 Change detection

- [ ] `changed_files(repo_root, base_ref)`
- [ ] support:
  - [ ] `origin/main...HEAD`
  - [ ] explicit base ref
  - [ ] `--staged`
  - [ ] `--working-tree`
- [ ] parse `git diff --name-status`
- [ ] handle:
  - [ ] added
  - [ ] modified
  - [ ] deleted
  - [ ] renamed
  - [ ] copied

### 4.7 Deleted and renamed files

- [ ] delete stale file graph on delete
- [ ] MVP rename behavior:
  - [ ] remove old path
  - [ ] parse new path as fresh file
- [ ] later:
  - [ ] preserve stable node identity across rename if hash unchanged

---

## Phase 5 — Parser Abstraction

The upstream parser is both the most important subsystem and the most monolithic file. The right Rust design is a per-language handler model behind a common parser interface.

### 5.1 Parser interface

- [ ] `supports(path) -> bool`
- [ ] `parse(repo_root, abs_path, src) -> ParsedFile`
- [ ] `language_name()`
- [ ] `extract_nodes()`
- [ ] `extract_edges()`

### 5.2 Parser registry

- [ ] register handlers
- [ ] resolve parser by extension
- [ ] expose supported languages list
- [ ] fail gracefully on unknown languages

### 5.3 Language strategy

- [ ] v1 first-class languages:
  - [ ] Rust
  - [ ] Go
  - [ ] Python
  - [ ] JavaScript
  - [ ] TypeScript
- [ ] v1.1 later:
  - [ ] Java
  - [ ] C#
  - [ ] PHP
- [ ] treat notebooks and framework-specific formats as later work

### 5.4 Tree-sitter integration

- [ ] wire core Tree-sitter parser
- [ ] load per-language grammars
- [ ] standardize AST walking helpers
- [ ] standardize line-span extraction
- [ ] standardize text slice extraction
- [ ] standardize fallback behavior on parse failure

### 5.5 Parser output shape

- [ ] always emit a `File` node
- [ ] emit symbol nodes
- [ ] emit containment edges
- [ ] emit imports edges
- [ ] emit calls edges
- [ ] emit tested-by/tests edges where possible
- [ ] include unresolved edges if exact resolution is unavailable

---

## Phase 6 — First Language: Rust

### 6.1 Rust extension support

- [ ] `.rs`

### 6.2 Rust node extraction

- [ ] modules
- [ ] functions
- [ ] impl methods
- [ ] structs
- [ ] enums
- [ ] traits
- [ ] constants
- [ ] statics
- [ ] tests

### 6.3 Rust edge extraction

- [ ] `Contains`
- [ ] `Calls`
- [ ] `Implements` via `impl Trait for Type`
- [ ] `References` for `use`/type refs later
- [ ] `Tests` / `TestedBy` for `#[cfg(test)]` and `#[test]`

### 6.4 Rust qualified-name scheme

- [ ] file node: `<relative-path>`
- [ ] module node: `<relative-path>::module::<name>`
- [ ] function node: `<relative-path>::fn::<name>`
- [ ] method node: `<relative-path>::method::<Type>.<name>`
- [ ] struct node: `<relative-path>::struct::<name>`
- [ ] enum node: `<relative-path>::enum::<name>`
- [ ] trait node: `<relative-path>::trait::<name>`

### 6.5 Rust parser tests

- [ ] free functions
- [ ] nested modules
- [ ] trait impls
- [ ] generic functions
- [ ] methods on impl blocks
- [ ] test modules
- [ ] macro-heavy files
- [ ] line-span accuracy

---

## Phase 7 — Additional Language Handlers

### 7.1 Go

- [ ] package node
- [ ] functions
- [ ] methods
- [ ] structs
- [ ] interfaces
- [ ] imports
- [ ] call edges

### 7.2 Python

- [ ] modules
- [ ] functions
- [ ] classes
- [ ] methods
- [ ] imports
- [ ] decorators
- [ ] tests

### 7.3 JavaScript/TypeScript

- [ ] functions
- [ ] classes
- [ ] methods
- [ ] imports/exports
- [ ] call expressions
- [ ] TS type/interface nodes
- [ ] later TS path alias resolution

### 7.4 Call-target resolution tiers

- [ ] Tier 1:
  - [ ] capture textual callee target only
- [ ] Tier 2:
  - [ ] resolve same-file symbols
- [ ] Tier 3:
  - [ ] resolve same-package/module symbols
- [ ] Tier 4:
  - [ ] resolve imports where practical
- [ ] Never block parse success on perfect call resolution

---

## Phase 8 — Full Build Pipeline

### 8.1 `atlas build`

- [ ] find repo root
- [ ] open DB
- [ ] run migrations
- [ ] collect tracked files
- [ ] filter supported files
- [ ] read + hash each file
- [ ] skip unchanged files
- [ ] parse file
- [ ] replace file graph in DB
- [ ] summarize:
  - [ ] scanned count
  - [ ] skipped count
  - [ ] parsed count
  - [ ] nodes inserted
  - [ ] edges inserted
  - [ ] elapsed time

### 8.2 Concurrency model

- [ ] concurrent file parsing
- [ ] single writer thread for SQLite
- [ ] bounded queue between parser workers and DB writer
- [ ] memory cap for queued parsed files
- [ ] backpressure instead of unbounded buffering

### 8.3 Failure handling

- [ ] continue on per-file parse failure
- [ ] surface file parse errors in summary
- [ ] add `--fail-fast`
- [ ] keep DB consistent on crashes

---

## Phase 9 — Incremental Update Pipeline

The upstream project’s incremental update flow is one of the highest-value behaviors to preserve. It re-parses changed files plus dependent files, then replaces only affected graph slices.

### 9.1 `atlas update`

- [ ] discover changed files
- [ ] if no explicit list, call git diff
- [ ] find dependent files from graph
- [ ] merge + dedupe targets
- [ ] remove deleted files from graph
- [ ] parse changed + dependent files
- [ ] batch replace graph slices
- [ ] print update summary

### 9.2 Dependent invalidation

- [ ] implement `find_dependents(changed_files)`
- [ ] start conservative:
  - [ ] files importing changed file package/module
  - [ ] callers/callees by edge links
- [ ] tolerate over-invalidation in v1
- [ ] avoid under-invalidation where possible

### 9.3 Update modes

- [ ] `atlas update --base origin/main`
- [ ] `atlas update --staged`
- [ ] `atlas update --working-tree`
- [ ] `atlas update --files path1 path2`

---

## Phase 10 — Impact Radius

The upstream system already uses a recursive SQLite CTE seeded from nodes in changed files. That SQL-first traversal should be preserved in Rust because it avoids rebuilding the full graph in memory.

### 10.1 Seed selection

- [ ] map changed files to node qualified names
- [ ] load seed nodes into temp table
- [ ] preserve changed node set separately from impacted node set

### 10.2 Recursive traversal

- [ ] forward through source -> target edges
- [ ] backward through target -> source edges
- [ ] depth-limited recursion
- [ ] node-count cap
- [ ] dedupe visited nodes

### 10.3 Impact result shape

- [ ] changed nodes
- [ ] impacted nodes
- [ ] impacted files
- [ ] relevant edges among those nodes

### 10.4 CLI

- [ ] `atlas impact --base origin/main`
- [ ] `atlas impact --files ...`
- [ ] `atlas impact --max-depth 3`
- [ ] `atlas impact --max-nodes 200`
- [ ] `atlas impact --json`

### 10.5 Tests

- [ ] one-hop graph
- [ ] cyclic graph
- [ ] disconnected graph
- [ ] depth cap behavior
- [ ] max node cap behavior
- [ ] deleted seed files
- [ ] seed file with no nodes

---

## Phase 11 — Search

The upstream search layer uses FTS5 and ranking heuristics; embeddings are explicitly optional and belong later, not in the first release.

### 11.1 Basic FTS search

- [ ] search `nodes_fts`
- [ ] join back to `nodes`
- [ ] order by BM25
- [ ] limit results
- [ ] return scored nodes

### 11.2 Search filters

- [ ] by kind
- [ ] by language
- [ ] by file path
- [ ] by test status
- [ ] by repo subpath

### 11.3 Ranking heuristics

- [ ] exact name boost
- [ ] exact qualified-name boost
- [ ] function/method/class boost
- [ ] same-directory boost
- [ ] same-language boost
- [ ] changed-file boost later

### 11.4 CLI

- [ ] `atlas query "ReplaceFileGraph"`
- [ ] `atlas query "impact radius" --kind function`
- [ ] `atlas query "parser" --language rust`
- [ ] `atlas query "foo" --json`

---

## Phase 12 — Review Context Assembly

The main user benefit of the upstream project is not just building the graph, but generating minimal useful context around code changes. That review/query layer belongs in core scope.

### 12.1 Minimal context

- [ ] input:
  - [ ] changed files
  - [ ] max depth
  - [ ] max nodes
- [ ] output:
  - [ ] changed node summaries
  - [ ] key impacted neighbors
  - [ ] critical edges
  - [ ] relevant file excerpts later

### 12.2 Review context

- [ ] identify touched functions/methods/classes
- [ ] list callers/callees/importers/tests
- [ ] include impact-radius result
- [ ] rank by relevance
- [ ] avoid dumping entire graph
- [ ] provide machine-readable JSON and concise text output

### 12.3 Risk/change summaries

- [ ] changed files list
- [ ] changed symbol count
- [ ] public API node changes
- [ ] test coverage adjacency
- [ ] large function touched
- [ ] cross-module/cross-package impact

### 12.4 CLI

- [ ] `atlas review-context --base origin/main`
- [ ] `atlas review-context --files ...`
- [ ] `atlas review-context --json`
- [ ] `atlas detect-changes --base origin/main`

---

## Phase 13 — CLI UX

### 13.1 Clap commands

- [x] root command with global flags:
  - [x] `--repo`
  - [x] `--db`
  - [x] `--verbose`
  - [x] `--json`
- [x] subcommands:
  - [x] `init`
  - [x] `build`
  - [x] `update`
  - [x] `status`
  - [x] `detect-changes`
  - [x] `query`
  - [x] `impact`
  - [x] `review-context`

### 13.2 Output styles

- [x] human-readable output
- [x] structured JSON output
- [ ] stable machine schema for automation
- [x] concise error messages
- [x] rich verbose diagnostics when requested

### 13.3 Status command

- [x] DB path
- [x] repo root
- [x] indexed file count
- [x] node count
- [x] edge count
- [x] nodes by kind
- [x] languages present
- [x] last build/update time
- [ ] changed files since base

---

## Phase 14 — Testing Strategy

The upstream report highlights parser fidelity and install/hook fragility as the real high-risk areas, not SQLite itself. For the Rust rewrite, parser and incremental-update tests should therefore be first-class.

### 14.1 Unit tests

- [ ] node/edge serialization
- [ ] qualified-name generation
- [ ] path normalization
- [ ] hash stability
- [ ] CLI arg parsing

### 14.2 SQLite tests

- [ ] migration creates schema
- [ ] WAL mode enabled
- [ ] file graph replacement works
- [ ] delete file graph works
- [ ] FTS search works
- [ ] impact CTE works
- [ ] lock/retry behavior

### 14.3 Repo tests

- [ ] repo root detection
- [ ] tracked-file collection
- [ ] change detection
- [ ] rename handling
- [ ] deleted file handling

### 14.4 Parser golden tests

- [ ] Rust fixtures
- [ ] Go fixtures
- [ ] Python fixtures
- [ ] JS/TS fixtures
- [ ] call edges
- [ ] imports
- [ ] tests detection
- [ ] bad syntax handling
- [ ] line ranges

### 14.5 Integration tests

- [ ] `atlas build` on sample repo
- [ ] `atlas update` after edits
- [ ] `atlas impact` returns expected nodes
- [ ] `atlas review-context` returns stable useful output
- [ ] `atlas query` returns expected ranked matches

### 14.6 Cross-platform tests

- [ ] Linux
- [ ] Windows path/casing behavior
- [ ] macOS path handling
- [ ] git command behavior on each

---

## Phase 15 — Performance and Operational Hardening

### 15.1 Build performance

- [ ] measure files/sec
- [ ] measure nodes/sec
- [ ] measure DB writes/sec
- [ ] benchmark parser workers vs writer bottleneck
- [ ] tune batch sizes

### 15.2 Query performance

- [ ] benchmark FTS query latency
- [ ] benchmark impact-radius latency
- [ ] benchmark review-context latency

### 15.3 Memory and reliability

- [ ] cap parse queue size
- [ ] avoid loading giant repos into memory
- [ ] add partial-failure reporting
- [ ] add crash-safe file replacement semantics

### 15.4 Diagnostics

- [ ] `atlas doctor` later
- [ ] `atlas db check` later
- [ ] tracing spans around build/update phases
- [ ] optional metrics export later

---

## Phase 16 — MCP / Serve Layer

The upstream repo exposes a stdio MCP server, but the report makes clear this should stay a thin wrapper over the domain services rather than becoming the architecture center.

### 16.1 Core MCP scope

- [ ] `build_or_update_graph`
- [ ] `get_minimal_context`
- [ ] `get_impact_radius`
- [ ] `get_review_context`
- [ ] `query_graph`
- [ ] `traverse_graph`
- [ ] `list_graph_stats`
- [ ] `detect_changes`

### 16.2 Transport design

- [ ] keep service layer transport-independent
- [ ] add stdio server later
- [ ] avoid long-running tool deadlocks
- [ ] wrap blocking work in dedicated worker threads if needed

### 16.3 Serve command

- [ ] `atlas serve`
- [ ] expose only core tools in first version
- [ ] add prompts later, not first

---

## Phase 17 — Later Features

### 17.1 Strong candidates for v1.1 / v1.2

- [ ] watch mode
- [ ] docs lookup
- [ ] large-function finder
- [ ] test adjacency queries
- [ ] architecture overview
- [ ] flow tracing
- [ ] communities

### 17.2 Explicitly late-stage

- [ ] embeddings
- [ ] cloud providers
- [ ] wiki generation
- [ ] visualization
- [ ] export formats
- [ ] registry
- [ ] install automation
- [ ] refactor/apply-refactor
- [ ] eval harness

---

## MVP Definition

Release 1 is done when this works end-to-end:

- [ ] `atlas init`
- [ ] `atlas build`
- [ ] `atlas status`
- [ ] `atlas query "some symbol"`
- [ ] `atlas update --base origin/main`
- [ ] `atlas impact --base origin/main`
- [ ] `atlas review-context --base origin/main`

And the system has:

- [ ] multi-language parsing for a small v1 language set
- [ ] SQLite graph persistence
- [ ] file-slice replacement
- [ ] recursive impact-radius SQL traversal
- [ ] review-context assembly
- [ ] FTS5 search
- [ ] CI on Linux + Windows

---

## Recommended Implementation Order

### Slice 1 — foundation

- [x] workspace
- [x] error types
- [x] logging
- [x] SQLite open/migrate
- [x] CLI scaffold

### Slice 2 — storage

- [ ] schema
- [ ] insert/replace/delete
- [ ] stats
- [ ] FTS
- [ ] basic search

### Slice 3 — repo

- [ ] repo root
- [ ] tracked files
- [ ] hashing
- [ ] git diff parsing

### Slice 4 — parser

- [ ] parser trait
- [ ] Tree-sitter bootstrap
- [ ] Rust language handler
- [ ] Go language handler
- [ ] node/edge extraction

### Slice 5 — build/update

- [ ] full build pipeline
- [ ] single-writer DB loop
- [ ] incremental update
- [ ] dependent invalidation

### Slice 6 — graph intelligence

- [ ] impact-radius SQL
- [ ] query helpers
- [ ] review-context assembly
- [ ] detect-changes summary

### Slice 7 — polish

- [ ] JSON outputs
- [ ] more parsers
- [ ] benchmarks
- [ ] Windows hardening
- [ ] serve/MCP

---

## Final Rule

Keep Atlas centered on this chain:

- repo scan
- parse
- persist graph
- update incrementally
- search/traverse
- build review context

Do not let optional features delay that core path.
