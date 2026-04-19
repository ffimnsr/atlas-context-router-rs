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

- [x] Use binary name: `atlas`
- [x] Use hidden work dir: `.atlas/`
- [x] Use DB path: `.atlas/worldview.sqlite`
- [ ] Use config path later: `.atlas/config.toml`
- [x] Use CLI commands:
  - [x] `atlas init`
  - [x] `atlas build`
  - [x] `atlas update`
  - [x] `atlas detect-changes`
  - [x] `atlas status`
  - [x] `atlas query`
  - [x] `atlas impact`
  - [x] `atlas review-context`
  - [x] `atlas serve` (later, MCP/stdin or JSON-RPC style)

---

## Phase 0 — Core Architecture Decisions

### 0.1 Freeze v1 scope

- [x] Include in v1:
  - [x] repo root detection
  - [x] tracked-file collection
  - [x] git diff change detection
  - [x] parser abstraction
  - [x] first language handlers
  - [x] SQLite graph store
  - [x] batch file graph replacement
  - [x] recursive SQL impact traversal
  - [x] review context assembly
  - [x] FTS5 keyword search
  - [x] CLI
- [x] Explicitly defer:
  - [x] embeddings
  - [x] communities
  - [x] flows
  - [x] wiki
  - [x] visualization/export
  - [x] multi-repo registry
  - [x] install hooks
  - [x] auto-watch mode
  - [x] refactor/apply-refactor
  - [x] evaluation harness
  - [x] cloud providers

### 0.2 Freeze compatibility policy

- [x] Preserve upstream behavior where it matters:
  - [x] qualified-name semantics
  - [x] incremental build/update flow
  - [x] SQLite-first persistence
  - [x] impact radius from changed-file seed nodes
  - [x] review/query usefulness
- [x] Permit deliberate redesign where it improves maintainability:
  - [x] split giant parser into per-language modules
  - [x] split graph store/query/review into separate crates/modules
  - [x] use repo-relative paths internally instead of absolute paths where possible
- [x] Document every intentional compatibility break (see COMPATIBILITY.md)

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
  - [ ] `packages/atlas-mcp`
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
  - [ ] `clap_complete`
- [x] Errors:
  - [x] `thiserror`
  - [x] `anyhow` for CLI layer only
- [x] Serialization:
  - [x] `serde`
  - [x] `serde_json`
- [x] Paths/hash/time:
  - [x] `camino`
  - [x] `sha2`
  - [x] `time`
- [x] SQLite:
  - [x] `rusqlite` with bundled SQLite + FTS5 support
- [x] Logging:
  - [x] `tracing`
  - [x] `tracing-subscriber`
- [x] Concurrency:
  - [x] `rayon` added to workspace deps (parallel file processing, v1.1)
  - [x] using std threads for v1 baseline
- [x] Tree-sitter:
  - [x] `tree-sitter`
  - [x] language crates as needed
- [x] Git integration:
  - [x] use `std::process::Command` wrapping `git` CLI (v1 decision — avoids libgit2 build dep)
  - [ ] `git2` deferred to post-v1
- [x] For performance-sensitive hashmaps use `hashbrown` crate (added to workspace deps)


### 1.3 CI and quality gates

- [x] Add:
  - [x] `cargo fmt --check`
  - [x] `cargo clippy --all-targets --all-features -- -D warnings`
  - [x] `cargo test --workspace`
- [x] Add Linux CI
- [x] Add SQLite/FTS5 smoke test in CI
- [x] Add fixture-based regression tests

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

- [x] Create DB at `.atlas/worldview.sqlite`
- [x] On open, set:
  - [x] `PRAGMA journal_mode=WAL;`
  - [x] `PRAGMA synchronous=NORMAL;`
  - [x] `PRAGMA foreign_keys=ON;`
  - [x] `PRAGMA busy_timeout=5000;`
- [x] Use one write connection policy for mutation-heavy operations
- [x] Add startup integrity check command later

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
- [x] Decide symlink policy
- [x] Add Windows casing normalization tests

### 4.3 Ignore handling

- [x] Support git-tracked files first via `git ls-files`
- [x] Add `.atlasignore` later
- [x] Ignore by default:
  - [x] `.git`
  - [x] `node_modules`
  - [x] `vendor`
  - [x] `dist`
  - [x] `build`
  - [x] `.next`
  - [x] `target`
  - [x] `.venv`
  - [x] `__pycache__`

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
- [ ] v1.1 parser expansion covered in Phase 7.5
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
- [ ] TS path alias resolution

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

## Phase 7.5 — v1.1 Language Handlers

Implement these like Rust and Go: dedicated handler, qualified-name scheme, edge extraction, parser tests, build/update integration.

### 7.5.1 Java

- [ ] package node
- [ ] classes
- [ ] interfaces
- [ ] enums
- [ ] methods
- [ ] imports
- [ ] annotations
- [ ] call edges
- [ ] qualified-name scheme
- [ ] parser tests

### 7.5.2 C#

- [ ] namespace node
- [ ] classes
- [ ] interfaces
- [ ] enums
- [ ] structs
- [ ] methods
- [ ] using imports
- [ ] attributes
- [ ] call edges
- [ ] qualified-name scheme
- [ ] parser tests

### 7.5.3 PHP

- [ ] namespace node
- [ ] classes
- [ ] interfaces
- [ ] traits
- [ ] functions
- [ ] methods
- [ ] `use` imports
- [ ] attributes/annotations where practical
- [ ] call edges
- [ ] qualified-name scheme
- [ ] parser tests

### 7.5.4 JSON and TOML

- [ ] JSON document node extraction
- [ ] JSON top-level object/key symbol strategy
- [ ] TOML document node extraction
- [ ] TOML table/key symbol strategy
- [ ] stable qualified-name scheme for config files
- [ ] parser tests for nested keys and arrays

### 7.5.5 HTML, CSS, Bash

- [ ] HTML document/component node extraction
- [ ] HTML imports/includes where practical
- [ ] CSS selector/rule extraction
- [ ] Bash functions
- [ ] Bash sourced-file/import handling where practical
- [ ] language-specific qualified-name scheme
- [ ] parser tests for representative fixtures

### 7.5.6 Shared acceptance criteria

- [ ] unsupported constructs degrade gracefully
- [ ] parser never panic on malformed source
- [ ] line-span accuracy
- [ ] file-slice replacement work same as Rust and Go
- [ ] integration coverage in build/update path

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

- [x] concurrent file parsing
- [x] single writer thread for SQLite
- [x] bounded queue between parser workers and DB writer
- [x] memory cap for queued parsed files
- [x] backpressure instead of unbounded buffering

### 8.3 Failure handling

- [x] continue on per-file parse failure
- [x] surface file parse errors in summary
- [x] add `--fail-fast`
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
- [x] tracing spans around build/update phases
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

- [x] keep SQLite FTS5 as baseline
- [ ] add embeddings behind optional toggle
- [ ] chunk symbol-sized nodes for retrieval
- [ ] generate embeddings
- [ ] store vectors in SQLite or external store
- [ ] implement hybrid retrieval:
  - [ ] FTS results
  - [ ] vector results
  - [ ] reciprocal-rank fusion merge

### 18.2 Ranking improvements

- [x] exact name boost
- [x] qualified-name boost
- [ ] fuzzy match
- [x] camelCase/snake_case token split
- [ ] recent-file boost
- [x] API-level boost

### 18.3 Graph-aware search

- [x] expand results to callers
- [x] expand results to callees
- [x] expand results to imports
- [x] rank by graph distance

## Phase 19 — Advanced Impact Analysis

### 19.1 Weighted traversal

- [x] assign traversal weights:
  - [x] calls > imports > references
- [x] add confidence tiers

### 19.2 Impact scoring

- [x] compute `impact_score` per node
- [x] rank impacted nodes

### 19.3 Change classification

- [x] detect API change
- [x] detect signature change
- [x] detect internal change
- [x] assign risk level

### 19.4 Test impact

- [x] map tests to functions
- [x] list affected tests
- [x] detect missing tests

### 19.5 Boundary detection

- [x] detect cross-module changes
- [x] highlight architecture violations

## Phase 20 — Performance & Incremental Engine

### 20.1 Incremental parsing

- [x] partial file reparse
- [ ] optional Tree-sitter incremental parsing

### 20.2 Dependency invalidation

- [x] improve `find_dependents`
- [x] reduce over-invalidation

### 20.3 Parallelization

- [x] optimize worker pool
- [x] batch DB writes
- [x] reduce lock contention

### 20.4 Large-repo handling

- [x] streaming parsing
- [x] memory caps
- [x] chunked DB writes

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

## Phase 22 — Context Engine

Build deterministic retrieval-and-selection layer over graph. No LLM dependence. Input structured request or simple text. Output bounded, explainable context for CLI, review flow, later agent flow.

### 22.1 Scope and responsibilities

- [ ] accept structured or semi-structured request
- [ ] resolve target symbol(s), file(s), or change-set
- [ ] retrieve nearby graph structure
- [ ] rank retrieved items by relevance
- [ ] trim to bounded result size
- [ ] return machine-readable context

### 22.2 Request model

- [ ] define `ContextIntent` enum:
  - [ ] `ImpactAnalysis`
  - [ ] `UsageLookup`
  - [ ] `RefactorSafety`
  - [ ] `DeadCodeCheck`
  - [ ] `RenamePreview`
  - [ ] `DependencyRemoval`
  - [ ] `ReviewContext`
  - [ ] `SymbolContext`
- [ ] define `ContextTarget` variants:
  - [ ] symbol qualified name
  - [ ] symbol name
  - [ ] file path
  - [ ] changed file list
  - [ ] changed symbol list
  - [ ] edge query seed
- [ ] define `ContextRequest` fields:
  - [ ] intent
  - [ ] target
  - [ ] max_nodes
  - [ ] max_edges
  - [ ] max_files
  - [ ] max_depth
  - [ ] include_code_spans
  - [ ] include_tests
  - [ ] include_imports
  - [ ] include_callers
  - [ ] include_callees
  - [ ] include_neighbors

### 22.3 Response model

- [ ] define `ContextResult`:
  - [ ] resolved target nodes
  - [ ] selected nodes
  - [ ] selected edges
  - [ ] selected files
  - [ ] code spans
  - [ ] relevance scores
  - [ ] truncation flags
  - [ ] retrieval metadata
- [ ] define `SelectedNode`:
  - [ ] node id
  - [ ] qualified name
  - [ ] kind
  - [ ] file path
  - [ ] line span
  - [ ] relevance score
  - [ ] selection reason
- [ ] define `SelectedEdge`:
  - [ ] source
  - [ ] target
  - [ ] edge kind
  - [ ] depth
  - [ ] relevance score
  - [ ] selection reason
- [ ] define `SelectedFile`:
  - [ ] path
  - [ ] language
  - [ ] reason included
  - [ ] node count included

### 22.4 Intent parsing and resolution

- [ ] implement exact symbol lookup path
- [ ] implement simple query classifier:
  - [ ] contains `what breaks`
  - [ ] contains `used by`
  - [ ] contains `who calls`
  - [ ] contains `safe to refactor`
  - [ ] contains `dead code`
  - [ ] contains `rename`
  - [ ] contains `remove dependency`
- [ ] add regex extraction for:
  - [ ] quoted symbol names
  - [ ] file paths
  - [ ] function-like names
  - [ ] method-like names
- [ ] fallback to symbol search + context expansion
- [ ] resolve by qualified name
- [ ] resolve by exact symbol name
- [ ] resolve by file path
- [ ] resolve by ranked search if ambiguous
- [ ] return ambiguity metadata if multiple candidates remain

### 22.5 Retrieval, ranking, trimming

- [ ] fetch direct node record
- [ ] fetch direct callers
- [ ] fetch direct callees
- [ ] fetch import edges
- [ ] fetch file containment edges
- [ ] fetch test adjacency if enabled
- [ ] fetch one-hop neighbors
- [ ] fetch multi-hop neighbors if requested
- [ ] rank highest:
  - [ ] exact target node
  - [ ] direct callers
  - [ ] direct callees
- [ ] rank medium:
  - [ ] same-file siblings
  - [ ] tests targeting target node
  - [ ] imports linked to target file
- [ ] rank lower:
  - [ ] second-hop neighbors
  - [ ] broad file-level nodes
  - [ ] weak reference edges
- [ ] add scoring factors:
  - [ ] graph distance
  - [ ] edge confidence
  - [ ] same file boost
  - [ ] same package/module boost
  - [ ] public API boost
  - [ ] test adjacency boost
- [ ] hard-limit nodes
- [ ] hard-limit edges
- [ ] hard-limit files
- [ ] prefer direct relationships over broad context
- [ ] drop low-confidence edges first
- [ ] drop distant neighbors before dropping direct callers/callees
- [ ] mark output as truncated if limits applied

### 22.6 Code spans, APIs, tests

- [ ] include target symbol span
- [ ] include caller/callee spans if enabled
- [ ] include nearest relevant lines only
- [ ] avoid whole-file dumps by default
- [ ] provide file path + line range references
- [ ] create `ContextEngine`
- [ ] implement:
  - [ ] `resolve_target`
  - [ ] `build_symbol_context`
  - [ ] `build_review_context`
  - [ ] `build_impact_context`
  - [ ] `rank_context`
  - [ ] `trim_context`
- [ ] tests:
  - [ ] exact symbol lookup
  - [ ] ambiguous symbol resolution
  - [ ] missing symbol behavior
  - [ ] bounded node trimming
  - [ ] caller/callee prioritization
  - [ ] include/exclude tests behavior
  - [ ] code span selection accuracy

## Phase 23 — Autonomous Code Reasoning

Answer structural questions from graph + parser + store facts only. No unsupported claims. Return structured findings with evidence and certainty.

### 23.1 Engine responsibilities and core types

- [ ] analyze removal impact
- [ ] detect dead code candidates
- [ ] score refactor safety
- [ ] validate dependency removal
- [ ] inspect rename blast radius
- [ ] classify change risk
- [ ] detect missing test adjacency
- [ ] explain graph facts behind result
- [ ] define `ReasoningResult`
- [ ] define `ReasoningEvidence`
- [ ] define `ReasoningWarning`
- [ ] define `ConfidenceTier`
- [ ] define `SafetyScore`
- [ ] define `ImpactClass`
- [ ] define `DeadCodeCandidate`
- [ ] define `DependencyRemovalResult`
- [ ] define `RenamePreviewResult`

### 23.2 Removal impact analysis

- [ ] accept symbol or file as seed
- [ ] find direct inbound edges
- [ ] find direct outbound edges
- [ ] traverse impact graph to configured depth
- [ ] separate:
  - [ ] definitely impacted
  - [ ] probably impacted
  - [ ] weakly related
- [ ] return:
  - [ ] impacted symbols
  - [ ] impacted files
  - [ ] impacted tests
  - [ ] relevant edges
- [ ] use high-confidence heuristics:
  - [ ] direct call edges
  - [ ] direct import edges
  - [ ] direct test links
- [ ] use medium-confidence heuristics:
  - [ ] inferred symbol links
  - [ ] unresolved selector calls within same file/package
- [ ] use low-confidence heuristics:
  - [ ] textual references only
  - [ ] weak unresolved edges
- [ ] include seed node(s)
- [ ] include per-node depth
- [ ] include edge kind per path
- [ ] include impact class

### 23.3 Dead code, safety, dependency removal

- [ ] detect dead code candidates when:
  - [ ] no inbound call edges
  - [ ] no inbound reference edges
  - [ ] not public/exported
  - [ ] not in configured entrypoint allowlist
  - [ ] not framework entrypoint
  - [ ] not test
  - [ ] not referenced by known routing/config conventions
  - [ ] not dynamically registered where detectable
- [ ] support suppression / allowlists:
  - [ ] main entrypoints
  - [ ] CLI command handlers
  - [ ] framework lifecycle hooks
  - [ ] exported plugin symbols
  - [ ] reflection-based registration points
  - [ ] manual ignore list
- [ ] dead-code output:
  - [ ] candidate symbol
  - [ ] why flagged
  - [ ] certainty tier
  - [ ] blockers preventing auto-removal
- [ ] score refactor safety from:
  - [ ] fan-in
  - [ ] fan-out
  - [ ] visibility
  - [ ] file/module scope
  - [ ] public API status
  - [ ] linked test count
  - [ ] dependency depth
  - [ ] dynamic usage risk
  - [ ] unresolved edge count
- [ ] mark safer when:
  - [ ] private/internal
  - [ ] low fan-in
  - [ ] low fan-out
  - [ ] strong test adjacency
  - [ ] self-contained in one file/module
- [ ] mark riskier when:
  - [ ] public/exported
  - [ ] many inbound callers
  - [ ] many cross-module callers
  - [ ] unresolved/dynamic references
  - [ ] no tests nearby
- [ ] refactor-safety output:
  - [ ] numeric score
  - [ ] safety band: `safe`, `caution`, `risky`
  - [ ] reasons list
  - [ ] suggested validations
- [ ] validate dependency removal:
  - [ ] detect unused imports
  - [ ] detect unreferenced dependencies
  - [ ] verify zero references in graph
  - [ ] verify zero references in same-file AST if needed
  - [ ] verify no tests depend on target
  - [ ] flag dynamic/reflective uncertainty
- [ ] dependency-removal output:
  - [ ] removable boolean
  - [ ] blocking references
  - [ ] confidence tier
  - [ ] suggested cleanup edits

### 23.4 Rename radius, test adjacency, risk, APIs

- [ ] preview rename blast radius:
  - [ ] locate definition
  - [ ] locate all references
  - [ ] classify references as same-file, same-module/package, cross-module/package, tests
  - [ ] detect unresolved references needing manual review
  - [ ] count affected files
  - [ ] count affected symbols
- [ ] rename output:
  - [ ] affected references
  - [ ] affected files
  - [ ] risk level
  - [ ] collision warnings
  - [ ] manual review flags
- [ ] estimate test adjacency:
  - [ ] map tests to symbols where possible
  - [ ] map changed symbol to direct tests
  - [ ] map changed symbol to same-file tests
  - [ ] map changed symbol to same-module tests
  - [ ] flag no linked tests
  - [ ] flag weak test adjacency only
- [ ] test-adjacency output:
  - [ ] linked tests
  - [ ] coverage strength
  - [ ] recommendation flag
- [ ] classify change risk from:
  - [ ] public API touched
  - [ ] test adjacency strength
  - [ ] cross-module impact
  - [ ] inbound caller count
  - [ ] unresolved references
  - [ ] dependency fan-out
  - [ ] impacted file count
- [ ] risk output:
  - [ ] low / medium / high
  - [ ] contributing factors
  - [ ] suggested review focus
- [ ] create `ReasoningEngine`
- [ ] implement:
  - [ ] `analyze_removal`
  - [ ] `detect_dead_code`
  - [ ] `score_refactor_safety`
  - [ ] `check_dependency_removal`
  - [ ] `preview_rename_radius`
  - [ ] `classify_change_risk`
  - [ ] `find_test_adjacency`
- [ ] tests:
  - [ ] simple call graph impact
  - [ ] cyclic graph impact
  - [ ] dead private function candidate
  - [ ] exported/public function not flagged
  - [ ] entrypoint suppression
  - [ ] rename blast radius same file
  - [ ] rename blast radius cross module
  - [ ] dependency removal blocked by reference
  - [ ] missing test signal for changed symbol
  - [ ] risk scoring sanity checks

## Phase 24 — Smart Refactoring Core

Deterministic, syntax-aware transforms backed by graph validation. Start with strongly checkable operations only: rename, dead-code removal, import cleanup. Keep extract-function as detection/planning first.

### 24.1 Responsibilities and operation model

- [ ] plan refactor
- [ ] simulate impact before apply
- [ ] apply deterministic text/AST edits
- [ ] validate no collisions
- [ ] validate references updated
- [ ] emit patch preview
- [ ] support dry-run mode
- [ ] support rollback on validation failure
- [ ] define `RefactorOperation` enum:
  - [ ] `RenameSymbol`
  - [ ] `RemoveDeadCode`
  - [ ] `CleanImports`
  - [ ] `ExtractFunctionCandidate`
- [ ] define `RefactorPlan`
- [ ] define `RefactorEdit`
- [ ] define `RefactorPatch`
- [ ] define `RefactorValidationResult`
- [ ] define `RefactorDryRunResult`

### 24.2 Rename symbol

- [ ] require unique definition resolution
- [ ] require valid new identifier
- [ ] reject local collision at definition site
- [ ] reject obvious collision in affected scopes
- [ ] resolve definition node
- [ ] gather all references
- [ ] classify references by certainty
- [ ] build edit set
- [ ] simulate rename impact
- [ ] apply edits in stable order
- [ ] validate resulting references
- [ ] rename validation:
  - [ ] renamed definition exists
  - [ ] all expected references updated
  - [ ] no duplicate definitions created
  - [ ] no blocked write targets
  - [ ] unresolved/manual-review references reported separately
- [ ] rename output:
  - [ ] files changed
  - [ ] edits count
  - [ ] manual review list
  - [ ] patch preview

### 24.3 Remove dead code and clean imports

- [ ] remove dead code only when:
  - [ ] candidate has sufficient confidence
  - [ ] no protected entrypoint status
  - [ ] no unresolved high-risk blockers
- [ ] dead-code removal steps:
  - [ ] select removable node
  - [ ] remove symbol span
  - [ ] clean surrounding whitespace/comments if safe
  - [ ] run import cleanup on touched file
  - [ ] update graph slice
- [ ] dead-code validation:
  - [ ] symbol definition removed
  - [ ] no dangling same-file references
  - [ ] import cleanup stable
  - [ ] patch preview generated
- [ ] import-cleanup steps:
  - [ ] compute actual symbol usage in file
  - [ ] compare imports vs usage
  - [ ] mark unused imports
  - [ ] remove unused imports
  - [ ] normalize spacing/order if formatter integration exists later
- [ ] import-cleanup validation:
  - [ ] no used import removed
  - [ ] file remains syntactically valid if parser re-check exists
  - [ ] no duplicate imports created

### 24.4 Extract-function detection, simulation, APIs, tests

- [ ] detect extract-function candidates from:
  - [ ] large contiguous block
  - [ ] repeated block pattern
  - [ ] clear input variables
  - [ ] clear output variables
  - [ ] limited side-effect boundaries
- [ ] score candidates with:
  - [ ] repeated logic boost
  - [ ] long block boost
  - [ ] low free-variable count boost
  - [ ] low control-flow complexity boost
- [ ] candidate output:
  - [ ] span
  - [ ] proposed inputs
  - [ ] proposed outputs
  - [ ] extraction difficulty score
  - [ ] no auto-apply in initial version
- [ ] run impact simulation before non-trivial refactor:
  - [ ] rename blast radius
  - [ ] removal impact
  - [ ] safety score
  - [ ] affected files
  - [ ] affected symbols
  - [ ] nearby tests
  - [ ] unresolved risks
- [ ] patch and dry-run support:
  - [ ] generate unified diff preview
  - [ ] support `--dry-run`
  - [ ] support per-file edit grouping
  - [ ] support machine-readable edit output
  - [ ] support cancellation before apply
- [ ] create `RefactorEngine`
- [ ] implement:
  - [ ] `plan_rename`
  - [ ] `apply_rename`
  - [ ] `plan_dead_code_removal`
  - [ ] `apply_dead_code_removal`
  - [ ] `plan_import_cleanup`
  - [ ] `apply_import_cleanup`
  - [ ] `detect_extract_function_candidates`
  - [ ] `simulate_refactor_impact`
- [ ] add safety checks:
  - [ ] file write safety
  - [ ] edit overlap detection
  - [ ] parser revalidation hook later
  - [ ] reject unsafe overlapping edits
  - [ ] reject ambiguous rename targets
  - [ ] reject low-confidence dead code removals by default
- [ ] tests:
  - [ ] rename single-file symbol
  - [ ] rename multi-file symbol
  - [ ] rename collision rejection
  - [ ] dead code removal private helper
  - [ ] protected entrypoint not removed
  - [ ] unused import removed
  - [ ] used import preserved
  - [ ] extract-function candidate detection basic case
  - [ ] dry-run output stable
  - [ ] patch output stable

## Phase 25 — Shared Analysis and Refactor Infrastructure

Shared support for explainability, config, CLI surface, JSON contracts, benchmarks. Phase 22-24 depend on this.

### 25.1 Evidence and explainability

- [ ] attach evidence edges
- [ ] attach evidence nodes
- [ ] attach scoring factors
- [ ] attach uncertainty flags

### 25.2 Config surface

- [ ] max context nodes
- [ ] max context depth
- [ ] dead code certainty threshold
- [ ] refactor safety threshold
- [ ] impact max depth
- [ ] impact max nodes
- [ ] dynamic usage allowlist
- [ ] entrypoint allowlist
- [ ] framework conventions file

### 25.3 Language support policy

- [ ] phase-2 features degrade gracefully by language
- [ ] enable rename only where symbol/reference mapping is mature
- [ ] enable dead code only where inbound usage confidence is acceptable
- [ ] enable import cleanup only where parser support is reliable

### 25.4 CLI surfaces

- [ ] `atlas context <symbol>`
- [ ] `atlas analyze remove <symbol>`
- [ ] `atlas analyze dead-code`
- [ ] `atlas analyze safety <symbol>`
- [ ] `atlas analyze dependency <symbol-or-import>`
- [ ] `atlas refactor rename <symbol> <new-name> --dry-run`
- [ ] `atlas refactor remove-dead <symbol> --dry-run`
- [ ] `atlas refactor clean-imports <file> --dry-run`

### 25.5 JSON output, benchmarks, completion criteria

- [ ] stable JSON schema for all analysis commands
- [ ] stable JSON schema for patch previews
- [ ] include evidence and certainty fields
- [ ] benchmark context retrieval latency
- [ ] benchmark impact analysis latency
- [ ] benchmark dead-code scan latency
- [ ] benchmark rename planning latency
- [ ] benchmark import-cleanup latency
- [ ] completion criteria:
  - [ ] context engine resolves and returns bounded symbol/change context
  - [ ] removal impact analysis works on representative repos
  - [ ] dead code detection produces useful candidates with suppressions
  - [ ] refactor safety scoring is implemented and explainable
  - [ ] dependency removal checks are implemented
  - [ ] rename blast radius is implemented
  - [ ] deterministic rename refactor works in dry-run and apply modes
  - [ ] deterministic dead code removal works for high-confidence candidates
  - [ ] import cleanup works reliably
  - [ ] extract-function candidate detection exists even if auto-apply stays deferred

## Phase 26 — MCP / Agent Integration

### 26.1 Core tools

- [ ] `get_review_context`
- [ ] `get_impact_radius`
- [ ] `query_graph`
- [ ] `explain_change`

### 26.2 Output design

- [ ] structured JSON
- [ ] stable schemas
- [ ] token-efficient responses

### 26.3 Context optimization

- [ ] return summaries only
- [ ] limit node count
- [ ] prioritize relevance

## Phase 27 — Observability

### 27.1 Metrics

- [ ] indexing time
- [ ] nodes/sec
- [ ] query latency
- [ ] impact latency

### 27.2 Debug tools

- [ ] `atlas doctor`
- [ ] `atlas debug graph`
- [ ] `atlas explain-query`

### 27.3 Data integrity

- [ ] orphan-node detection
- [ ] edge validation
- [ ] DB consistency checks

## Phase 28 — Real-Time & Continuous Mode

Deterministic watch flow on top of existing incremental pipeline. Goal: near-real-time graph freshness without full rebuilds for small edits.

### 28.1 Watch mode scope

- [ ] auto-update graph when files change
- [ ] stay efficient on rapid edit bursts
- [ ] avoid full rebuild path for ordinary edits
- [ ] integrate with existing incremental parse + update flow
- [ ] stay deterministic and LLM-free

### 28.2 File watcher

- [ ] choose watcher crate (for example `notify`)
- [ ] watch repo directories recursively
- [ ] ignore:
  - [ ] `.git`
  - [ ] build directories
  - [ ] ignored paths
- [ ] map watch roots to normalized repo-relative paths
- [ ] handle platform-specific watcher quirks

### 28.3 Change detection

- [ ] detect:
  - [ ] file create
  - [ ] file modify
  - [ ] file delete
  - [ ] file rename
- [ ] map events to file paths
- [ ] normalize duplicate event bursts
- [ ] keep delete/rename handling consistent with batch update mode

### 28.4 Update pipeline integration

- [ ] on change enqueue file for update
- [ ] batch changes with debounce window (`100–500ms`)
- [ ] trigger:
  - [ ] incremental parsing
  - [ ] graph update
- [ ] reuse existing update/build primitives where practical
- [ ] avoid duplicate queue entries for same file

### 28.5 Incremental update logic

- [ ] reuse existing update logic
- [ ] handle:
  - [ ] modified files
  - [ ] deleted files
  - [ ] renamed files
- [ ] preserve dependent invalidation rules
- [ ] ensure graph slice replacement semantics stay atomic

### 28.6 Queue, workers, state

- [ ] create update queue
- [ ] worker responsibilities:
  - [ ] parse file
  - [ ] update graph
- [ ] ensure:
  - [ ] single DB writer
  - [ ] no race conditions
- [ ] track:
  - [ ] pending updates
  - [ ] in-progress updates
  - [ ] last update time
- [ ] expose internal state for status/debug surfaces later

### 28.7 Performance and failure handling

- [ ] debounce rapid file changes
- [ ] coalesce duplicate updates
- [ ] limit concurrent parsing
- [ ] handle parse failures gracefully
- [ ] add retry logic only if bounded and safe
- [ ] log watch/update errors
- [ ] keep watch loop alive after recoverable failures

### 28.8 CLI and tests

- [ ] add `atlas watch`
- [ ] show:
  - [ ] files updated
  - [ ] nodes updated
  - [ ] errors
- [ ] support JSON output if command surface standardizes on it
- [ ] tests:
  - [ ] file modify triggers update
  - [ ] file delete removes graph slice
  - [ ] rename handled correctly
  - [ ] debounce works
  - [ ] no duplicate updates
- [ ] completion criteria:
  - [ ] watch mode updates graph in near real-time
  - [ ] no full rebuild required for small changes
  - [ ] queue and writer path remain race-free

## Phase 29 — Intelligence & Insights

Deterministic analytics layer on top of graph + stored metadata. Produce explainable architecture insights, metrics, risk assessments, pattern detection. No LLM dependency.

### 29.1 Architecture analysis

- [ ] build module-level graph
- [ ] detect strongly connected components (SCC)
- [ ] identify cyclic dependencies
- [ ] classify cycles (`local` vs `cross-module`)
- [ ] output cycle paths
- [ ] define configurable layer rules
- [ ] map files/modules to layers
- [ ] detect invalid edges
- [ ] output layer violations
- [ ] compute coupling score per module
- [ ] detect high-coupling modules
- [ ] detect tightly coupled clusters
- [ ] compute nodes per file
- [ ] compute edges per file
- [ ] flag large/highly connected files

### 29.2 Code health metrics

- [ ] node-level metrics:
  - [ ] fan-in
  - [ ] fan-out
  - [ ] dependency depth
  - [ ] reference count
  - [ ] test adjacency
- [ ] file-level metrics:
  - [ ] node count
  - [ ] edge count
  - [ ] average fan-in/out
  - [ ] import count
  - [ ] test coverage ratio
- [ ] module-level metrics:
  - [ ] internal vs external dependencies
  - [ ] coupling score
  - [ ] cohesion approximation
- [ ] compute percentiles
- [ ] detect outliers

### 29.3 Risk assessment engine

- [ ] score from inputs:
  - [ ] public API
  - [ ] fan-in/out
  - [ ] cross-module dependencies
  - [ ] test adjacency
  - [ ] depth
  - [ ] unresolved edges
- [ ] implement weighted formula
- [ ] normalize to `0–100`
- [ ] classify `low` / `medium` / `high`
- [ ] output:
  - [ ] factors list
  - [ ] evidence nodes/edges

### 29.4 Pattern detection

- [ ] duplicate patterns:
  - [ ] repeated call chains
  - [ ] similar subgraphs
- [ ] unused structures:
  - [ ] unused modules
  - [ ] isolated graphs
  - [ ] orphan nodes
- [ ] high centrality:
  - [ ] compute centrality
  - [ ] find hubs
  - [ ] find bottlenecks
- [ ] deep chains:
  - [ ] detect long call chains
  - [ ] flag complexity

### 29.5 APIs, outputs, CLI, config, tests

- [ ] create `InsightsEngine`
- [ ] implement:
  - [ ] `analyze_architecture()`
  - [ ] `compute_metrics()`
  - [ ] `assess_risk()`
  - [ ] `detect_patterns()`
  - [ ] `find_cycles()`
- [ ] define:
  - [ ] `ArchitectureReport`
  - [ ] `MetricsReport`
  - [ ] `RiskReport`
  - [ ] `PatternReport`
- [ ] ensure each report includes:
  - [ ] summary
  - [ ] detailed findings
  - [ ] evidence
- [ ] CLI:
  - [ ] `atlas insights architecture`
  - [ ] `atlas insights metrics`
  - [ ] `atlas insights risk <symbol>`
  - [ ] `atlas insights patterns`
  - [ ] JSON output support
- [ ] config:
  - [ ] thresholds
  - [ ] layer config
  - [ ] ignore lists
- [ ] tests:
  - [ ] cycle detection
  - [ ] coupling detection
  - [ ] unused-node detection
  - [ ] risk scoring validation
  - [ ] outlier detection
- [ ] completion criteria:
  - [ ] accurate architecture insights
  - [ ] correct metrics
  - [ ] explainable risk scoring
  - [ ] useful pattern detection
  - [ ] structured outputs

## Phase 30 — Optional Advanced Features

### 30.1 Multi-repo

- [ ] shared graph
- [ ] cross-repo impact

### 30.2 Remaining code intelligence

- [ ] similar-function detection beyond graph-shape heuristics
- [ ] duplicate detection beyond exact structural patterns
- [ ] infer modules
- [ ] label components

## Phase 31 — Deferred Lowest Priority

### 31.1 Wiki / docs generation

- [ ] generate Markdown docs
- [ ] module pages
- [ ] function pages
- [ ] static site export

### 31.2 v2 completion criteria

- [ ] search beats grep
- [ ] impact analysis is reliable
- [ ] review context is useful
- [ ] MCP tools are usable by agents
- [ ] performance scales to large repos

### 31.3 Guiding principle

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

### Slice 8 — product contract

- [x] rename DB path to `.atlas/worldview.sqlite`
- [x] finish binary/work-dir/config naming contract (paths module in atlas-cli)
- [x] freeze v1 include/defer scope
- [x] document every intentional compatibility break (see COMPATIBILITY.md)
- [x] decide remaining dependency choices that affect public shape

### Slice 9 — correctness gaps

- [x] add `NodeId` type
- [x] finish remaining SQLite table/transaction/schema test gaps
- [x] complete path normalization and ignore handling
- [x] finish deleted/renamed file behavior
- [x] complete remaining language-strategy and call-resolution work

### Slice 10 — MVP command completion

- [x] finish `atlas init`
- [x] finish `atlas status`
- [x] finish `atlas query`
- [x] finish `atlas impact`
- [x] finish `atlas review-context`
- [x] close remaining impact/search/review CLI gaps

### Slice 11 — quality gates

- [x] add `cargo fmt --check`
- [x] add `cargo clippy --all-targets --all-features -- -D warnings`
- [x] add `cargo test --workspace`
- [x] add Linux CI
- [x] add SQLite/FTS5 smoke coverage
- [x] add fixture/golden/integration regression coverage

### Slice 12 — hardening

- [x] finish build concurrency model
- [x] add failure-handling gaps in build/update path
- [x] add startup integrity check command
- [x] improve performance, query tuning, memory, diagnostics
- [x] add cross-platform hardening beyond current Windows baseline

### Slice 13 — MCP and agent surface

- [x] create `packages/atlas-mcp`
- [x] finish MCP transport and serve-command details
- [x] expose core MCP tools with agent-usable output
- [x] optimize context packaging for agents

### Slice 14 — post-MVP gate

- [x] confirm MVP complete before expanding scope
- [x] keep deferred items from blocking core path

### Slice 15 — retrieval

- [x] hybrid search
- [x] ranking improvements
- [x] graph-aware search

### Slice 16 — advanced impact

- [x] weighted traversal
- [x] impact scoring
- [x] change classification
- [x] test impact
- [x] boundary detection

### Slice 17 — incremental engine

- [x] incremental parsing
- [x] dependency invalidation follow-up
- [x] parallelization
- [x] large-repo handling

### Slice 18 — developer workflows

- [ ] explain change
- [ ] smart review context
- [ ] natural-language queries
- [ ] CLI workflow UX

### Slice 19 — context engine

- [ ] context request/response model
- [ ] deterministic intent parsing
- [ ] target resolution pipeline
- [ ] retrieval/ranking/trimming
- [ ] code-span selection
- [ ] `ContextEngine` API + tests

### Slice 20 — reasoning engine

- [ ] reasoning result/evidence types
- [ ] removal impact analysis
- [ ] dead code detection
- [ ] refactor safety scoring
- [ ] dependency removal validation
- [ ] rename radius + risk/test adjacency

### Slice 21 — refactor engine

- [ ] refactor operation/plan/patch types
- [ ] rename planning/apply
- [ ] dead-code removal
- [ ] import cleanup
- [ ] extract-function candidate detection
- [ ] dry-run/patch/validation coverage

### Slice 22 — shared analysis infra

- [ ] explainability/evidence plumbing
- [ ] config surface
- [ ] language capability gates
- [ ] CLI commands for context/analyze/refactor
- [ ] stable JSON contracts
- [ ] benchmarks and phase-completion gate

### Slice 23 — MCP and agent surface

- [ ] core MCP tools
- [ ] stable structured output
- [ ] token-efficient relevance trimming

### Slice 24 — observability

- [ ] metrics
- [ ] debug tools
- [ ] data integrity tooling

### Slice 25 — watch mode

- [ ] file watcher
- [ ] change detection mapping
- [ ] queue/debounce/worker system
- [ ] incremental update integration
- [ ] watch CLI + tests

### Slice 26 — insights

- [ ] architecture analysis
- [ ] code-health metrics
- [ ] risk assessment engine
- [ ] pattern detection
- [ ] `InsightsEngine` reports + CLI + tests

### Slice 27 — optional advanced

- [ ] multi-repo support
- [ ] remaining advanced code intelligence

### Slice 28 — deferred lowest priority

- [ ] wiki/docs generation
- [ ] v2 completion criteria
- [ ] lowest-priority guiding-principle items

### Slice 29 — deferred platform and ecosystem

- [ ] install hooks
- [ ] flows/communities schema
- [ ] evaluation harness
- [ ] cloud providers
- [ ] shell completion and minor tooling leftovers

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
