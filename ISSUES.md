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
  - [x] `atlas serve` (later, MCP/stdin or JSON-RPC style)

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
- [x] Tree-sitter:
  - [x] `tree-sitter`
  - [x] language crates as needed
- [ ] Git integration:
  - [ ] use `git2`
  - [ ] fallback to `std::process::Command`
- [ ] For any hashmaps use `hashbrown` crate


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
- [x] `replace_file_graph(file_path, file_hash, nodes, edges)`
- [x] `replace_batch(parsed_files)`
- [x] `delete_file_graph(file_path)`
- [x] `nodes_by_file(file_path)`
- [x] `edges_by_file(file_path)`
- [x] `find_dependents(changed_files)`
- [x] `impact_radius(changed_files, max_depth, max_nodes)`
- [x] `search(query)`
- [x] `stats()`

### 3.11 Transaction semantics

- [x] Replace one file graph atomically:
  - [x] begin immediate transaction
  - [x] delete old FTS rows
  - [x] delete old edges for file
  - [x] delete old nodes for file
  - [x] upsert file row
  - [x] insert nodes
  - [x] insert edges
  - [x] insert FTS rows
  - [x] commit
- [ ] Add rollback tests
- [ ] Add lock-contention tests

---

## Phase 4 — Repository Scanner and Git Diff

The upstream project’s primary promise includes full build plus incremental update from git diff. That makes repo scanning and change detection part of the actual product kernel, not glue code.

### 4.1 Repo root detection

- [x] Implement `find_repo_root(start: &Utf8Path) -> Result<Utf8PathBuf>`
- [x] First try `git rev-parse --show-toplevel`
- [x] Fallback: walk parent dirs for `.git`
- [x] Normalize returned path

### 4.2 Path normalization

- [x] Convert to repo-relative paths for persistence
- [x] Normalize separators to `/`
- [x] Resolve `.` and `..`
- [ ] Decide symlink policy
- [ ] Add Windows casing normalization tests

### 4.3 Ignore handling

- [x] Support git-tracked files first via `git ls-files`
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

- [x] `collect_files(repo_root)`
- [x] Use `git ls-files`
- [ ] Optional recursive submodule handling later
- [ ] Skip unsupported extensions
- [x] skip binary files
- [x] skip giant files
- [x] configurable file size threshold

### 4.5 File hashing

- [x] SHA-256 file hash
- [x] skip unchanged files on full build
- [x] persist hash in `files`

### 4.6 Change detection

- [x] `changed_files(repo_root, base_ref)`
- [x] support:
  - [x] `origin/main...HEAD`
  - [x] explicit base ref
  - [x] `--staged`
  - [x] `--working-tree`
- [x] parse `git diff --name-status`
- [x] handle:
  - [x] added
  - [x] modified
  - [x] deleted
  - [x] renamed
  - [x] copied

### 4.7 Deleted and renamed files

- [x] delete stale file graph on delete
- [x] MVP rename behavior:
  - [x] remove old path
  - [x] parse new path as fresh file
- [ ] later:
  - [ ] preserve stable node identity across rename if hash unchanged

---

## Phase 5 — Parser Abstraction

The upstream parser is both the most important subsystem and the most monolithic file. The right Rust design is a per-language handler model behind a common parser interface.

### 5.1 Parser interface

- [x] `supports(path) -> bool`
- [x] `parse(repo_root, abs_path, src) -> ParsedFile`
- [x] `language_name()`
- [x] `extract_nodes()`
- [x] `extract_edges()`

### 5.2 Parser registry

- [x] register handlers
- [x] resolve parser by extension
- [x] expose supported languages list
- [x] fail gracefully on unknown languages

### 5.3 Language strategy

- [ ] v1 first-class languages:
  - [x] Rust
  - [x] Go
  - [x] Python
  - [x] JavaScript
  - [ ] TypeScript
- [ ] v1.1 later:
  - [ ] Java
  - [ ] C#
  - [ ] PHP
  - [ ] JSON (tree-sitter-json)
  - [ ] TOML (tree-sitter-toml)
  - [ ] HTML (tree-sitter-html)
  - [ ] CSS (tree-sitter-css)
  - [ ] Bash (tree-sitter-bash)
- [ ] treat notebooks and framework-specific formats as later work

### 5.4 Tree-sitter integration

- [x] wire core Tree-sitter parser
- [x] load per-language grammars
- [x] standardize AST walking helpers
- [x] standardize line-span extraction
- [x] standardize text slice extraction
- [x] standardize fallback behavior on parse failure

### 5.5 Parser output shape

- [x] always emit a `File` node
- [x] emit symbol nodes
- [x] emit containment edges
- [x] emit imports edges
- [x] emit calls edges
- [x] emit tested-by/tests edges where possible
- [x] include unresolved edges if exact resolution is unavailable

---

## Phase 6 — First Language: Rust

### 6.1 Rust extension support

- [x] `.rs`

### 6.2 Rust node extraction

- [x] modules
- [x] functions
- [x] impl methods
- [x] structs
- [x] enums
- [x] traits
- [x] constants
- [x] statics
- [x] tests

### 6.3 Rust edge extraction

- [x] `Contains`
- [ ] `Calls`
- [x] `Implements` via `impl Trait for Type`
- [ ] `References` for `use`/type refs later
- [x] `Tests` / `TestedBy` for `#[cfg(test)]` and `#[test]`

### 6.4 Rust qualified-name scheme

- [x] file node: `<relative-path>`
- [x] module node: `<relative-path>::module::<name>`
- [x] function node: `<relative-path>::fn::<name>`
- [x] method node: `<relative-path>::method::<Type>.<name>`
- [x] struct node: `<relative-path>::struct::<name>`
- [x] enum node: `<relative-path>::enum::<name>`
- [x] trait node: `<relative-path>::trait::<name>`

### 6.5 Rust parser tests

- [x] free functions
- [x] nested modules
- [x] trait impls
- [ ] generic functions
- [x] methods on impl blocks
- [x] test modules
- [ ] macro-heavy files
- [x] line-span accuracy

---

## Phase 7 — Additional Language Handlers

### 7.1 Go

- [x] package node
- [x] functions
- [x] methods
- [x] structs
- [x] interfaces
- [x] imports
- [ ] call edges

### 7.2 Python

- [x] modules
- [x] functions
- [x] classes
- [x] methods
- [x] imports
- [ ] decorators
- [x] tests

### 7.3 JavaScript/TypeScript

- [x] functions
- [x] classes
- [x] methods
- [x] imports/exports
- [ ] call expressions
- [x] TS type/interface nodes
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

- [x] find repo root
- [x] open DB
- [x] run migrations
- [x] collect tracked files
- [x] filter supported files
- [x] read + hash each file
- [x] skip unchanged files
- [x] parse file
- [x] replace file graph in DB
- [x] summarize:
  - [x] scanned count
  - [x] skipped count
  - [x] parsed count
  - [x] nodes inserted
  - [x] edges inserted
  - [x] elapsed time

### 8.2 Concurrency model

- [ ] concurrent file parsing
- [ ] single writer thread for SQLite
- [ ] bounded queue between parser workers and DB writer
- [ ] memory cap for queued parsed files
- [ ] backpressure instead of unbounded buffering

### 8.3 Failure handling

- [x] continue on per-file parse failure
- [x] surface file parse errors in summary
- [ ] add `--fail-fast`
- [x] keep DB consistent on crashes

---

## Phase 9 — Incremental Update Pipeline

The upstream project’s incremental update flow is one of the highest-value behaviors to preserve. It re-parses changed files plus dependent files, then replaces only affected graph slices.

### 9.1 `atlas update`

- [x] discover changed files
- [x] if no explicit list, call git diff
- [x] find dependent files from graph
- [x] merge + dedupe targets
- [x] remove deleted files from graph
- [x] parse changed + dependent files
- [x] batch replace graph slices
- [x] print update summary

### 9.2 Dependent invalidation

- [x] implement `find_dependents(changed_files)`
- [x] start conservative:
  - [x] files importing changed file package/module
  - [x] callers/callees by edge links
- [x] tolerate over-invalidation in v1
- [x] avoid under-invalidation where possible

### 9.3 Update modes

- [x] `atlas update --base origin/main`
- [x] `atlas update --staged`
- [x] `atlas update --working-tree`
- [x] `atlas update --files path1 path2`

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
- [x] Windows path/casing behavior
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

- [x] `atlas serve`
- [x] expose only core tools in first version
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

## Post-MVP / Atlas v2 Roadmap

These phases extend v1 after core graph/build/update/query path is reliable.

## Phase 18 — Retrieval & Search Intelligence

### 18.1 Hybrid search

- [ ] keep SQLite FTS5 as baseline
- [ ] add embeddings behind optional toggle
- [ ] chunk symbol-sized nodes for retrieval
- [ ] generate embeddings
- [ ] store vectors in SQLite or external store
- [ ] implement hybrid retrieval:
  - [ ] FTS results
  - [ ] vector results
  - [ ] reciprocal-rank fusion merge

### 18.2 Ranking improvements

- [ ] exact name boost
- [ ] qualified-name boost
- [ ] fuzzy match
- [ ] camelCase/snake_case token split
- [ ] recent-file boost
- [ ] API-level boost

### 18.3 Graph-aware search

- [ ] expand results to callers
- [ ] expand results to callees
- [ ] expand results to imports
- [ ] rank by graph distance

## Phase 19 — Advanced Impact Analysis

### 19.1 Weighted traversal

- [ ] assign traversal weights:
  - [ ] calls > imports > references
- [ ] add confidence tiers

### 19.2 Impact scoring

- [ ] compute `impact_score` per node
- [ ] rank impacted nodes

### 19.3 Change classification

- [ ] detect API change
- [ ] detect signature change
- [ ] detect internal change
- [ ] assign risk level

### 19.4 Test impact

- [ ] map tests to functions
- [ ] list affected tests
- [ ] detect missing tests

### 19.5 Boundary detection

- [ ] detect cross-module changes
- [ ] highlight architecture violations

## Phase 20 — Performance & Incremental Engine

### 20.1 Incremental parsing

- [ ] partial file reparse
- [ ] optional Tree-sitter incremental parsing

### 20.2 Dependency invalidation

- [ ] improve `find_dependents`
- [ ] reduce over-invalidation

### 20.3 Parallelization

- [ ] optimize worker pool
- [ ] batch DB writes
- [ ] reduce lock contention

### 20.4 Large-repo handling

- [ ] streaming parsing
- [ ] memory caps
- [ ] chunked DB writes

## Phase 21 — Developer Workflow Features

### 21.1 Explain change

- [ ] summarize diff
- [ ] list impacted components
- [ ] explain ripple effects

### 21.2 Smart review context

- [ ] prioritize high-impact nodes
- [ ] include call chains
- [ ] remove noise

### 21.3 Natural-language queries

- [ ] support `where is X used`
- [ ] support `what calls Y`
- [ ] support `what breaks if I change Z`
- [ ] map intent to graph query

### 21.4 CLI UX

- [ ] interactive shell (`atlas shell`)
- [ ] fuzzy search
- [ ] paging
- [ ] colored output

## Phase 22 — MCP / Agent Integration

### 22.1 Core tools

- [ ] `get_review_context`
- [ ] `get_impact_radius`
- [ ] `query_graph`
- [ ] `explain_change`

### 22.2 Output design

- [ ] structured JSON
- [ ] stable schemas
- [ ] token-efficient responses

### 22.3 Context optimization

- [ ] return summaries only
- [ ] limit node count
- [ ] prioritize relevance

## Phase 23 — Observability

### 23.1 Metrics

- [ ] indexing time
- [ ] nodes/sec
- [ ] query latency
- [ ] impact latency

### 23.2 Debug tools

- [ ] `atlas doctor`
- [ ] `atlas debug graph`
- [ ] `atlas explain-query`

### 23.3 Data integrity

- [ ] orphan-node detection
- [ ] edge validation
- [ ] DB consistency checks

## Phase 24 — Optional Advanced Features

### 24.1 Code intelligence

- [ ] similar-function detection
- [ ] duplicate detection

### 24.2 Architecture insights

- [ ] detect layers
- [ ] infer modules
- [ ] label components

### 24.3 Watch mode

- [ ] auto-update on file change

### 24.4 Multi-repo

- [ ] shared graph
- [ ] cross-repo impact

## Phase 25 — Deferred Lowest Priority

### 25.1 Wiki / docs generation

- [ ] generate Markdown docs
- [ ] module pages
- [ ] function pages
- [ ] static site export

### 25.2 v2 completion criteria

- [ ] search beats grep
- [ ] impact analysis is reliable
- [ ] review context is useful
- [ ] MCP tools are usable by agents
- [ ] performance scales to large repos

### 25.3 Guiding principle

- [ ] avoid feature growth without signal quality gains
- [ ] prioritize better ranking
- [ ] prioritize better context
- [ ] prioritize better signals

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

- [x] schema
- [x] insert/replace/delete
- [x] stats
- [x] FTS
- [x] basic search

### Slice 3 — repo

- [x] repo root
- [x] tracked files
- [x] hashing
- [x] git diff parsing

### Slice 4 — parser

- [x] parser trait
- [x] Tree-sitter bootstrap
- [x] Rust language handler
- [x] Go language handler
- [x] node/edge extraction

### Slice 5 — build/update

- [x] full build pipeline
- [x] single-writer DB loop
- [x] incremental update
- [x] dependent invalidation

### Slice 6 — graph intelligence

- [x] impact-radius SQL
- [x] query helpers
- [x] review-context assembly
- [x] detect-changes summary

### Slice 7 — polish

- [x] JSON outputs
- [x] more parsers
- [x] benchmarks
- [x] Windows hardening
- [x] serve/MCP

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
