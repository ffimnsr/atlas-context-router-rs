# Atlas — Stateful Coding Agent Backend

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

## Product Name and CLI

- [x] Use binary name: `atlas`
- [x] Use hidden work dir: `.atlas/`
- [x] Use DB path: `.atlas/worldtree.db`
- [x] Use config path: `.atlas/config.toml`
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

## Roadmap Layout

- Part I. Core delivery roadmap: Phase 0 through Phase 17
- Part II. Release and interface gates: Release 1, Release 2, MCP and Agent Roadmap
- Part III. Post-MVP product expansion: Phase 18 through Phase 32
- Part IV. Context continuity and memory: Phase CM1 through Phase CM15
- Part V. Focused follow-up patches: Retrieval Follow-Up Patch, Graph Build Lifecycle Patch

## Cross-Cutting Track Map

- MCP and agent surfaces: MCP and Agent Roadmap, Phase 16, Phase 22.0 step 9
- Retrieval and search: Phase 11, Phase 18, Phase CM6, Phase CM9, Patch R
- Context and session continuity: Phase 22, Context-Mode and Continuity Roadmap
- Historical and analytics work: Phase 17, Phase 29, Phase 30, Phase 31

---

## Part I — Core Delivery Roadmap

Read this part in order. It covers initial architecture, storage, parsing, indexing, querying, UX, quality, serve foundations, and historical graph planning.

## Phase 0 — Core Architecture Decisions

### 0.1 Freeze release-1 scope

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
- [x] Move non-MVP items to post-MVP backlog:
  - [x] embeddings
  - [x] communities
  - [x] flows
  - [ ] wiki doc generation (CLI command)
  - [ ] visualization/export
  - [ ] multi-repo registry
  - [x] install hooks
  - [ ] auto-watch mode
  - [x] refactor/apply-refactor
  - [ ] evaluation harness
  - [ ] cloud providers

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
  - [x] `packages/atlas-mcp`
  - [x] `packages/atlas-engine` — shared build/update pipeline crate
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
  - [x] `clap_complete`
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
  - [ ] `git2` optional later
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

- [x] Create `NodeId` type
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
- [x] Add golden-schema tests

### 3.3 Tables

- [x] `metadata`
- [x] `files`
- [x] `nodes`
- [x] `edges`
- [x] `nodes_fts`
- [x] `flows` — catalog of named ordered scenarios or paths over graph
- [x] `flow_memberships` — join table assigning node to flow with order/role metadata
- [x] `communities` — catalog of named graph clusters; membership table can be added later if clustering work needs persistent node-to-community assignment

#### 3.3.1 Flow, Flow Membership, Communities Meaning

- `flows` should store reusable higher-level paths over graph, not duplicate raw `edges`. Example: "payment request path" or "rename blast radius walkthrough".
- `flow_memberships` should store which node is in which flow, plus `position` for order and `role` for labels like `entrypoint`, `middle`, `sink`, `changed`, `caller`, `test`.
- `communities` should store cluster metadata such as algorithm, level, parent/child hierarchy, and summary stats. Current schema does not yet persist explicit per-node community membership; if later features need that, add a separate `community_memberships` table instead of overloading `flow_memberships`.

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
- [x] Add rollback tests
- [x] Add lock-contention tests

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
- [x] Skip unsupported extensions
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
- [x] preserve stable node identity across rename if hash unchanged

### 4.8 Workspace and package-member awareness (useful for monorepo)

Track real workspace/package boundaries instead of using top-level path heuristics so monorepos like this Cargo workspace, NPM workspace, or Go workspace behave correctly during build, impact, review, and reasoning.

- [x] parse root `Cargo.toml` workspace metadata when present
- [x] resolve `workspace.members` globs into concrete member roots
- [x] parse root `package.json` workspace metadata when present
- [x] resolve NPM `workspaces` entries into concrete member roots
- [x] detect standalone package roots for repos without workspace table
- [x] map each tracked file to owning workspace member / package root
- [x] persist package identity separately from plain file path prefixes
- [x] emit first-class package/workspace nodes instead of path-only heuristics
- [x] replace top-level-directory `cross_package` heuristic with owning-package comparison
- [x] replace same-directory `same_package` heuristic with owning-package-aware resolution
- [x] ensure cross-crate and cross-package imports / impact edges in `packages/*` style repos classify correctly
- [x] add fixture repo with multiple Cargo workspace members
- [x] add fixture repo with multiple NPM workspace members
- [x] add fixture repo with multiple Go workspace modules
- [x] add regression tests for `build`, `update`, `impact`, `review-context`, and reasoning on multi-package workspace repos

#### 4.8.1 Owner Resolution Rules for Repos Without Workspace Metadata

Repos with multiple packages but no root workspace file must still resolve package ownership correctly. Workspace metadata is preferred when present, but standalone package-root detection is required and must produce same owner model.

- [x] detect standalone Cargo package roots from any tracked `Cargo.toml` containing `[package]`
- [x] detect standalone NPM package roots from any tracked `package.json` representing a real package
- [x] detect standalone Go module roots from any tracked `go.mod`
- [x] assign each tracked file to nearest ancestor package root when no workspace manifest exists
- [x] use nearest valid package root when nested package roots exist
- [x] keep files outside any package root as `unknown_owner` or repo-scope owner instead of forcing package match
- [x] use stable owner identity derived from ecosystem + manifest path, not package name alone
- [x] ensure root package does not swallow files under nested child package roots
- [x] ensure rename or move across package roots changes owner identity correctly during `update`

#### 4.8.2 Multi-Package Acceptance Cases

- [x] no-workspace Cargo repo with `crates/foo/Cargo.toml` and `libs/bar/Cargo.toml` resolves files under each root to different owning packages
- [x] no-workspace Cargo repo with root `Cargo.toml` plus nested `tools/gen/Cargo.toml` assigns `tools/gen/**` to nested package, not root package
- [x] no-workspace NPM repo with `apps/web/package.json` and `packages/ui/package.json` resolves files under each root to different owning packages
- [x] mixed repo with package roots plus repo-level scripts/docs keeps non-package files outside package ownership unless explicitly attached to a package
- [x] ambiguous symbol/query results across multiple packages include owning package identity in ranking/output metadata
- [x] `cross_package` reasoning, impact, and review signals compare owner identity, not top-level path segment
- [x] `same_package` call-resolution fallback scopes candidates by owning package first, then uses directory/import proximity only as tie-breakers or ranking signals

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

- [x] v1 first-class languages:
  - [x] Rust
  - [x] Go
  - [x] Python
  - [x] JavaScript
  - [x] TypeScript
- [x] v1.1 parser expansion covered in Phase 7.5
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
- [x] `Calls`
- [x] `Implements` via `impl Trait for Type`
- [x] `References` for `use`/type refs later
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
- [x] generic functions
- [x] methods on impl blocks
- [x] test modules
- [x] macro-heavy files
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
- [x] call edges

### 7.2 Python

- [x] modules
- [x] functions
- [x] classes
- [x] methods
- [x] imports
- [x] decorators
- [x] tests

### 7.3 JavaScript/TypeScript

- [x] functions
- [x] classes
- [x] methods
- [x] imports/exports
- [x] call expressions
- [x] TS type/interface nodes
- [x] TS path alias resolution

### 7.4 Call-target resolution tiers

- [x] Tier 1:
  - [x] capture textual callee target only
- [x] Tier 2:
  - [x] resolve same-file symbols
- [x] Tier 3:
  - [x] resolve same-package/module symbols
- [x] Tier 4:
  - [x] resolve imports where practical
- [x] Never block parse success on perfect call resolution
- [x] Next call-resolution edge cases:
  - [x] support non-relative/package-style `tsconfig` `extends` values
  - [x] resolve JS/TS barrel re-export chains for import-based call targets
  - [x] support TypeScript 6 `baseUrl` deprecation handling
  - [x] stop implicit bare-import fallback through `baseUrl`
  - [x] support explicit `"*"` catch-all `paths` migration patterns
  - [x] resolve `paths` targets relative to declaring config while keeping legacy `baseUrl`-prefixed aliases working where possible

## Phase 7.5 — v1.1 Language Handlers

Implement these like Rust and Go: dedicated handler, qualified-name scheme, edge extraction, parser tests, build/update integration.

When implementation starts, pin exact grammar source in crate/module docs so parser wiring does not drift:

- Java: `github.com/tree-sitter/tree-sitter-java`
- C: `github.com/tree-sitter/tree-sitter-c`
- Scala: `github.com/tree-sitter/tree-sitter-scala`
- C#: `github.com/tree-sitter/tree-sitter-c-sharp`
- PHP: `github.com/tree-sitter/tree-sitter-php`
- Ruby: `github.com/tree-sitter/tree-sitter-ruby`
- C++: `github.com/tree-sitter/tree-sitter-cpp`
- Bash: `github.com/tree-sitter/tree-sitter-bash`
- HTML: `github.com/tree-sitter/tree-sitter-html`
- CSS: `github.com/tree-sitter/tree-sitter-css`
- Markdown: `github.com/tree-sitter-grammars/tree-sitter-markdown`

### 7.5.1 Java (`tree-sitter/tree-sitter-java`)

- [x] package node
- [x] classes
- [x] interfaces
- [x] enums
- [x] methods
- [x] imports
- [x] annotations
- [x] call edges
- [x] qualified-name scheme
- [x] parser tests

### 7.5.2 C# (`tree-sitter/tree-sitter-c-sharp`)

- [x] namespace node
- [x] classes
- [x] interfaces
- [x] enums
- [x] structs
- [x] methods
- [x] using imports
- [x] attributes
- [x] call edges
- [x] qualified-name scheme
- [x] parser tests

### 7.5.3 PHP (`tree-sitter/tree-sitter-php`)

- [x] namespace node
- [x] classes
- [x] interfaces
- [x] traits
- [x] functions
- [x] methods
- [x] `use` imports
- [x] attributes/annotations where practical
- [x] call edges
- [x] qualified-name scheme
- [x] parser tests

### 7.5.4 C (`tree-sitter/tree-sitter-c`)

- [x] translation-unit / file node
- [x] functions
- [x] structs
- [x] enums
- [x] typedefs
- [x] macros/preprocessor references where practical
- [x] `#include` imports
- [x] call edges
- [x] qualified-name scheme
- [x] parser tests

### 7.5.5 C++ (`tree-sitter/tree-sitter-cpp`)

- [x] namespace node
- [x] classes
- [x] structs
- [x] enums
- [x] templates where practical
- [x] methods
- [x] `#include` imports
- [x] call edges
- [x] qualified-name scheme
- [x] parser tests

### 7.5.6 Scala (`tree-sitter/tree-sitter-scala`)

- [x] package node
- [x] objects
- [x] classes
- [x] traits
- [x] case classes
- [x] methods
- [x] imports
- [x] call edges
- [x] qualified-name scheme
- [x] parser tests

### 7.5.7 Ruby (`tree-sitter/tree-sitter-ruby`)

- [x] module node
- [x] classes
- [x] methods
- [x] singleton methods where practical
- [x] `require` / `require_relative` imports
- [x] mixins/include/extend where practical
- [x] call edges
- [x] qualified-name scheme
- [x] parser tests

### 7.5.8 JSON and TOML

- [x] JSON document node extraction
- [x] JSON top-level object/key symbol strategy
- [x] TOML document node extraction
- [x] TOML table/key symbol strategy
- [x] stable qualified-name scheme for config files
- [x] parser tests for nested keys and arrays

### 7.5.9 HTML, CSS, Bash (`tree-sitter/tree-sitter-html`, `tree-sitter/tree-sitter-css`, `tree-sitter/tree-sitter-bash`)

- [x] HTML document/component node extraction
- [x] HTML imports/includes where practical
- [x] CSS selector/rule extraction
- [x] Bash functions
- [x] Bash sourced-file/import handling where practical
- [x] language-specific qualified-name scheme
- [x] parser tests for representative fixtures

### 7.5.10 Markdown (`tree-sitter-grammars/tree-sitter-markdown`)

- [x] document node extraction
- [x] heading hierarchy extraction
- [x] fenced-code block extraction where practical
- [x] link/reference extraction where practical
- [x] stable qualified-name scheme by heading path
- [x] parser tests for nested heading documents

### 7.5.11 Shared acceptance criteria

- [x] unsupported constructs degrade gracefully
- [x] parser never panic on malformed source
- [x] line-span accuracy
- [x] file-slice replacement work same as Rust and Go
- [x] integration coverage in build/update path

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

- [x] map changed files to node qualified names
- [x] load seed nodes into temp table
- [x] preserve changed node set separately from impacted node set

### 10.2 Recursive traversal

- [x] forward through source -> target edges
- [x] backward through target -> source edges
- [x] depth-limited recursion
- [x] node-count cap
- [x] dedupe visited nodes

### 10.3 Impact result shape

- [x] changed nodes
- [x] impacted nodes
- [x] impacted files
- [x] relevant edges among those nodes

### 10.4 CLI

- [x] `atlas impact --base origin/main`
- [x] `atlas impact --files ...`
- [x] `atlas impact --max-depth 3`
- [x] `atlas impact --max-nodes 200`
- [x] `atlas impact --json`

### 10.5 Tests

- [x] one-hop graph
- [x] cyclic graph
- [x] disconnected graph
- [x] depth cap behavior
- [x] max node cap behavior
- [x] deleted seed files
- [x] seed file with no nodes

---

## Phase 11 — Search

The upstream search layer uses FTS5 and ranking heuristics; embeddings are explicitly optional and belong later, not in the first release.

### 11.1 Basic FTS search

- [x] search `nodes_fts`
- [x] join back to `nodes`
- [x] order by BM25
- [x] limit results
- [x] return scored nodes

### 11.2 Search filters

- [x] by kind
- [x] by language
- [x] by file path
- [x] by test status
- [x] by repo subpath

### 11.3 Ranking heuristics

- [x] exact name boost
- [x] exact qualified-name boost
- [x] function/method/class boost
- [x] same-directory boost
- [x] same-language boost
- [x] changed-file boost

### 11.4 CLI

- [x] `atlas query "ReplaceFileGraph"`
- [x] `atlas query "impact radius" --kind function`
- [x] `atlas query "parser" --language rust`
- [x] `atlas query "foo" --json`

---

## Phase 12 — Review Context Assembly

The main user benefit of the upstream project is not just building the graph, but generating minimal useful context around code changes. That review/query layer belongs in core scope.

### 12.1 Minimal context

- [x] input:
  - [x] changed files
  - [x] max depth
  - [x] max nodes
- [x] output:
  - [x] changed node summaries
  - [x] key impacted neighbors
  - [x] critical edges
  - [x] relevant file excerpts later

### 12.2 Review context

- [x] identify touched functions/methods/classes
- [x] list callers/callees/importers/tests
- [x] include impact-radius result
- [x] rank by relevance
- [x] avoid dumping entire graph
- [x] provide machine-readable JSON and concise text output

### 12.3 Risk/change summaries

- [x] changed files list
- [x] changed symbol count
- [x] public API node changes
- [x] test coverage adjacency
- [x] large function touched
- [x] cross-module/cross-package impact

### 12.4 CLI

- [x] `atlas review-context --base origin/main`
- [x] `atlas review-context --files ...`
- [x] `atlas review-context --json`
- [x] `atlas detect-changes --base origin/main`

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
- [x] stable machine schema for automation
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
- [x] changed files since base

---

## Phase 14 — Testing Strategy

The upstream report highlights parser fidelity and install/hook fragility as the real high-risk areas, not SQLite itself. For the Rust rewrite, parser and incremental-update tests should therefore be first-class.

### 14.1 Unit tests

- [x] node/edge serialization
- [x] qualified-name generation
- [x] path normalization
- [x] hash stability
- [x] CLI arg parsing

### 14.2 SQLite tests

- [x] migration creates schema
- [x] WAL mode enabled
- [x] file graph replacement works
- [x] delete file graph works
- [x] FTS search works
- [x] impact CTE works
- [ ] lock/retry behavior — **backlog**: requires concurrent SQLite connections or separate processes; cannot be covered with single-connection in-process tests

### 14.3 Repo tests

- [x] repo root detection
- [x] tracked-file collection
- [x] change detection
- [x] rename handling
- [x] deleted file handling

### 14.4 Parser golden tests

- [x] Rust fixtures
- [x] Go fixtures
- [x] Python fixtures
- [x] JS/TS fixtures
- [x] call edges
- [x] imports
- [x] tests detection
- [x] bad syntax handling
- [x] line ranges

### 14.5 Integration tests

- [x] `atlas build` on sample repo
- [x] `atlas update` after edits
- [x] `atlas impact` returns expected nodes
- [x] `atlas review-context` returns stable useful output
- [x] `atlas query` returns expected ranked matches

### 14.6 Cross-platform tests

- [x] Linux
- [x] Windows path/casing behavior
- [x] macOS path handling
- [x] git command behavior on each

---

## Phase 15 — Performance and Operational Hardening

### 15.1 Build performance

- [x] measure files/sec — covered by `store_bench` write-throughput criterion benchmark
- [x] measure nodes/sec — covered by `store_bench` write-throughput criterion benchmark
- [x] measure DB writes/sec — covered by `store_bench` write-throughput criterion benchmark
- [ ] benchmark parser workers vs writer bottleneck — **backlog**: requires full pipeline harness with real repo; out of scope for unit-level benches
- [ ] tune batch sizes — **backlog**: depends on profiling results from real workloads

### 15.2 Query performance

- [x] benchmark FTS query latency — covered by `store_bench`
- [x] benchmark impact-radius latency — covered by `store_bench`
- [ ] benchmark review-context latency — **backlog**: requires end-to-end integration bench; skipped for now

### 15.3 Memory and reliability

- [x] cap parse queue size — build pipeline uses bounded chunk-based batches; no unbounded in-memory accumulation
- [x] avoid loading giant repos into memory — chunked parallel parse; per-file size cap in collector
- [x] add partial-failure reporting — `parse_errors` counter surfaces failures in build/update summary
- [x] add crash-safe file replacement semantics — each file graph replaced in an atomic `BEGIN IMMEDIATE` transaction

### 15.4 Diagnostics

- [x] `atlas doctor` — implemented: checks repo root, git root, .atlas dir, config, DB file, integrity, graph stats, git ls-files
- [x] `atlas db check` — implemented
- [x] tracing spans around build/update phases
- [ ] optional metrics export — **backlog**: needs external metrics infra (Prometheus/OTEL); not on core path

---

## Phase 16 — MCP / Serve Layer

### 16.1 Status

Keep this phase as chronological marker for first MCP/serve work. Detailed MCP checklist now lives in Part II under MCP and Agent Roadmap, especially MCP1.

---

## Phase 17 — Historical Graphs (Atlas v3 / Phase 1.1)

Add time dimension to Atlas so system can answer historical questions, compare architectural evolution, and reason about how symbols, dependencies, and risks changed over time.

This phase should make Atlas capable of answering questions like:

- when was this symbol introduced?
- how did this function evolve?
- when did this dependency appear?
- which commits changed this module most often?
- what architectural edges were added or removed between two points in time?
- what did the graph look like at a given commit?

This phase must:

- remain deterministic
- build on existing SQLite graph model
- avoid any LLM dependency
- support commit-based historical inspection
- keep storage and indexing costs bounded

### 17.1 Scope

Historical Graphs means Atlas can persist and query graph state across multiple commits or snapshots.

This phase is not:

- a full git hosting integration
- a PR review UI
- a wiki/history summarizer
- a blame replacement
- a cloud history service

This phase is:

- graph snapshotting
- graph diffing
- commit-linked graph metadata
- symbol/file/edge history queries
- architectural evolution analysis

### 17.2 Core capabilities

- [ ] store graph snapshots per commit
- [ ] associate graph snapshots with repository + branch metadata
- [ ] diff two graph snapshots
- [ ] track node lifecycle:
  - [ ] introduced
  - [ ] modified
  - [ ] removed
- [ ] track edge lifecycle:
  - [ ] added
  - [ ] removed
  - [ ] changed confidence
- [ ] answer history queries for:
  - [ ] symbol
  - [ ] file
  - [ ] module
  - [ ] dependency
- [ ] expose historical graph queries through CLI
- [ ] keep retention and storage policies configurable

### 17.3 Historical model choice

#### Design principle

Do not duplicate entire live schema blindly for every commit if it explodes storage.

Start with hybrid design:

- current graph stays optimized for live queries
- historical layer stores snapshot metadata + compact graph state references
- initial version may store full per-commit graph slices for affected files only
- later version may deduplicate unchanged file graphs across commits

#### Recommended first implementation

- [ ] persist commit-level snapshot records
- [ ] persist file-graph state keyed by file hash
- [ ] map each commit snapshot to set of file hashes active at that commit
- [ ] reconstruct graph for commit from file-hash references
- [ ] avoid duplicating unchanged file graphs across commits

This provides historical power without storing same file graph repeatedly.

### 17.4 Git metadata ingestion

#### Commit discovery

- [ ] implement commit enumeration:
  - [ ] latest commit only
  - [ ] bounded history window
  - [ ] explicit commit list
  - [ ] commit range
- [ ] support:
  - [ ] HEAD
  - [ ] branch ref
  - [ ] commit SHA
  - [ ] tag
  - [ ] merge base ranges later

#### Commit metadata

- [ ] collect and store:
  - [ ] commit SHA
  - [ ] parent SHA(s)
  - [ ] author name
  - [ ] author email
  - [ ] author time
  - [ ] committer time
  - [ ] commit message subject
  - [ ] full message later
  - [ ] branch/ref used during indexing
- [ ] normalize timestamps
- [ ] define canonical repo-relative commit identity

#### Git commands

- [ ] implement helper wrappers for:
  - [ ] `git rev-parse`
  - [ ] `git log`
  - [ ] `git show`
  - [ ] `git diff-tree`
  - [ ] `git cat-file`
- [ ] ensure commands are deterministic and machine-parseable
- [ ] add robust error handling for:
  - [ ] shallow clones
  - [ ] detached HEAD
  - [ ] missing refs
  - [ ] rewritten history
  - [ ] submodules later

### 17.5 Snapshot data model

#### New tables

- [ ] create `repos` table if not already present
- [ ] create `commits` table
- [ ] create `graph_snapshots` table
- [ ] create `snapshot_files` table
- [ ] create `historical_nodes` table or reuse content-addressed node storage
- [ ] create `historical_edges` table or reuse content-addressed edge storage
- [ ] create `node_history` table
- [ ] create `edge_history` table

#### `commits` table

- [ ] columns:
  - [ ] `commit_sha`
  - [ ] `repo_id`
  - [ ] `parent_sha`
  - [ ] `author_name`
  - [ ] `author_email`
  - [ ] `author_time`
  - [ ] `committer_time`
  - [ ] `subject`
  - [ ] `message`
  - [ ] `indexed_at`

#### `graph_snapshots` table

- [ ] columns:
  - [ ] `snapshot_id`
  - [ ] `repo_id`
  - [ ] `commit_sha`
  - [ ] `root_tree_hash` if available
  - [ ] `node_count`
  - [ ] `edge_count`
  - [ ] `file_count`
  - [ ] `created_at`

#### `snapshot_files` table

- [ ] columns:
  - [ ] `snapshot_id`
  - [ ] `file_path`
  - [ ] `file_hash`
  - [ ] `language`
  - [ ] `size`
- [ ] enforce uniqueness on `(snapshot_id, file_path)`

#### Node/edge history model

Recommended first pass:

- [ ] keep canonical live-style nodes/edges keyed by content hash or stable synthetic identity
- [ ] record snapshot membership separately
- [ ] record history rows mapping:
  - [ ] snapshot -> node ids
  - [ ] snapshot -> edge ids

Alternative first pass if simpler:

- [ ] duplicate per-snapshot nodes/edges for correctness first
- [ ] optimize storage later

#### Lifecycle tables

- [ ] `node_history` should support:
  - [ ] first_seen_snapshot
  - [ ] last_seen_snapshot
  - [ ] first_seen_commit
  - [ ] last_seen_commit
  - [ ] introduction_commit
  - [ ] removal_commit
- [ ] `edge_history` should support same lifecycle fields

### 17.6 Identity strategy

#### Symbol identity

This is hardest design problem in historical graphs.

Need stable way to say whether symbol in commit A is same symbol in commit B.

#### First-pass identity rules

- [ ] use qualified name as primary identity key
- [ ] pair with file path and kind
- [ ] include signature hash where helpful
- [ ] treat changed qualified name as remove + add unless explicit rename tracking exists
- [ ] document this behavior clearly

#### Later improvement

- [ ] add rename-aware symbol lineage
- [ ] add content-based similarity matching for moved/renamed symbols
- [ ] add signature-aware continuity heuristics

#### Edge identity

- [ ] use:
  - [ ] edge kind
  - [ ] source qualified name
  - [ ] target qualified name
  - [ ] file path
- [ ] optionally include line number bucket or hash

### 17.7 Historical indexing pipeline

#### Initial historical build

- [ ] implement `atlas history build`
- [ ] accept:
  - [ ] `--since`
  - [ ] `--until`
  - [ ] `--max-commits`
  - [ ] `--branch`
  - [ ] `--commits`
- [ ] for each commit:
  - [ ] checkout-free file access using git object reads where possible
  - [ ] enumerate tracked files at that commit
  - [ ] compute file hash
  - [ ] reuse existing parsed file graph if identical hash already indexed
  - [ ] parse only new file hashes
  - [ ] write snapshot metadata
  - [ ] attach file hash membership
  - [ ] attach node/edge membership
- [ ] summarize:
  - [ ] commits processed
  - [ ] files reused
  - [ ] files newly parsed
  - [ ] nodes reused
  - [ ] elapsed time

#### Incremental historical update

- [ ] implement `atlas history update`
- [ ] detect commits not yet indexed
- [ ] only process missing commits
- [ ] support appending new commits on branch
- [ ] guard against rewritten history
- [ ] detect force-push divergence and require explicit repair mode

### 17.8 Commit-time file reconstruction

#### Source retrieval

- [ ] support reading file contents from commit without checkout
- [ ] use:
  - [ ] `git show <sha>:<path>`
  - [ ] or tree/blob plumbing commands for efficiency later
- [ ] ensure binary detection still applies
- [ ] handle deleted paths correctly

#### File list reconstruction

- [ ] reconstruct tracked file list for each commit
- [ ] support:
  - [ ] `git ls-tree`
  - [ ] or `git diff-tree` for incremental file-set changes
- [ ] decide whether to full-enumerate per commit or replay diffs
- [ ] first version may prefer correctness over speed

### 17.9 Graph diff engine

#### Goal

Compare two graph snapshots and describe structural differences.

#### Diff scopes

- [ ] file diff
- [ ] node diff
- [ ] edge diff
- [ ] module diff
- [ ] architecture diff

#### Node diff

- [ ] detect:
  - [ ] added nodes
  - [ ] removed nodes
  - [ ] changed nodes
- [ ] changed criteria:
  - [ ] line span changed
  - [ ] signature changed
  - [ ] modifiers changed
  - [ ] test status changed
  - [ ] extra metadata changed

#### Edge diff

- [ ] detect:
  - [ ] added edges
  - [ ] removed edges
  - [ ] changed confidence tier
  - [ ] changed metadata

#### File diff

- [ ] detect:
  - [ ] added files
  - [ ] removed files
  - [ ] modified files
  - [ ] renamed files if git reports them
- [ ] expose language and size changes

#### Architecture diff

- [ ] detect:
  - [ ] new dependency paths
  - [ ] removed dependency paths
  - [ ] new cycles
  - [ ] broken cycles
  - [ ] changed central hubs
  - [ ] changed coupling between modules

### 17.10 History query layer

#### Symbol history

- [ ] implement query:
  - [ ] show first/last appearance
  - [ ] show commits where changed
  - [ ] show signature evolution
  - [ ] show file path changes

#### File history

- [ ] implement query:
  - [ ] show all commits touching file
  - [ ] show node count over time
  - [ ] show edge count over time
  - [ ] show symbol additions/removals

#### Dependency history

- [ ] implement query:
  - [ ] when edge first appeared
  - [ ] when edge disappeared
  - [ ] which commits added/removed dependency
  - [ ] how long edge persisted

#### Module history

- [ ] implement query:
  - [ ] node growth over time
  - [ ] dependency growth over time
  - [ ] test adjacency over time later
  - [ ] coupling trend over time

### 17.11 Evolution analytics

#### Churn metrics

- [ ] compute per symbol:
  - [ ] change count
  - [ ] lifetime
  - [ ] add/remove frequency
- [ ] compute per file:
  - [ ] commits touched
  - [ ] graph delta size
- [ ] compute per module:
  - [ ] dependency churn
  - [ ] symbol churn

#### Stability indicators

- [ ] identify:
  - [ ] stable symbols
  - [ ] unstable symbols
  - [ ] frequently changing dependencies
  - [ ] architectural hotspots

#### Trend metrics

- [ ] track:
  - [ ] file count growth
  - [ ] node count growth
  - [ ] edge count growth
  - [ ] module coupling trend
  - [ ] cycle count trend

### 17.12 Snapshot storage efficiency

#### Deduplication

- [ ] reuse parsed file graph when file hash repeats across commits
- [ ] avoid duplicate node/edge rows when content-identical
- [ ] deduplicate snapshot membership rows where possible

#### Retention controls

- [ ] support pruning policies:
  - [ ] keep all commits
  - [ ] keep latest N
  - [ ] keep tagged releases only
  - [ ] keep weekly snapshots
- [ ] implement `atlas history prune`

#### Storage diagnostics

- [ ] report:
  - [ ] commits stored
  - [ ] unique file hashes
  - [ ] deduplication ratio
  - [ ] DB size
  - [ ] snapshot density

### 17.13 CLI surfaces

#### New commands

- [ ] `atlas history build`
- [ ] `atlas history update`
- [ ] `atlas history status`
- [ ] `atlas history diff <commit-a> <commit-b>`
- [ ] `atlas history symbol <qualified-name>`
- [ ] `atlas history file <path>`
- [ ] `atlas history dependency <source> <target>`
- [ ] `atlas history prune`

#### Flags

- [ ] `--repo`
- [ ] `--db`
- [ ] `--since`
- [ ] `--until`
- [ ] `--branch`
- [ ] `--max-commits`
- [ ] `--json`
- [ ] `--stat-only`
- [ ] `--full`
- [ ] `--follow-renames` later

### 17.14 Output structures

- [ ] define `HistoricalSnapshot`
- [ ] define `GraphDiffReport`
- [ ] define `NodeHistoryReport`
- [ ] define `EdgeHistoryReport`
- [ ] define `FileHistoryReport`
- [ ] define `ModuleHistoryReport`
- [ ] define `ChurnReport`

Each should include:

- [ ] summary fields
- [ ] detailed findings
- [ ] evidence:
  - [ ] snapshot ids
  - [ ] commit SHAs
  - [ ] node/edge identifiers
  - [ ] file paths

### 17.15 Compatibility and correctness rules

- [ ] if symbol identity cannot be confidently linked across commits, prefer add/remove over false continuity
- [ ] preserve exact commit SHA references
- [ ] never rely on branch name as identity
- [ ] make rewritten-history behavior explicit
- [ ] keep historical indexing reproducible for same commit range

### 17.16 Failure modes and safeguards

- [ ] handle missing commits in shallow clones
- [ ] handle corrupted snapshot membership rows
- [ ] handle parser failures at historical commits without aborting full run
- [ ] track partial snapshot completeness
- [ ] mark snapshots with parse errors
- [ ] allow reindex/rebuild of individual snapshots

### 17.17 Tests

#### Git history fixtures

- [ ] repo with:
  - [ ] symbol introduced
  - [ ] symbol removed
  - [ ] symbol modified
  - [ ] dependency introduced
  - [ ] dependency removed
  - [ ] file renamed
  - [ ] module split/merge later

#### Snapshot tests

- [ ] commit metadata stored correctly
- [ ] snapshot membership stored correctly
- [ ] unchanged file graph reused across commits
- [ ] modified file graph creates new membership state

#### Diff tests

- [ ] node add/remove/change diff
- [ ] edge add/remove diff
- [ ] architecture diff detects new cycle
- [ ] architecture diff detects broken cycle

#### Query tests

- [ ] symbol history query
- [ ] file history query
- [ ] dependency history query
- [ ] module history trend query

#### Retention tests

- [ ] prune latest N
- [ ] prune by age
- [ ] prune by release/tag policy later

### 17.18 Performance and scaling

- [ ] benchmark commits/sec
- [ ] benchmark snapshot reconstruction speed
- [ ] benchmark graph diff speed
- [ ] benchmark symbol history query latency
- [ ] measure storage growth with and without deduplication

#### Optimization backlog

- [ ] commit-to-commit diff replay instead of full file enumeration
- [ ] blob-level cache
- [ ] parser result cache keyed by blob hash
- [ ] compressed membership encoding
- [ ] partial snapshot materialization

### 17.19 Recommended implementation order

#### Slice 1 — metadata foundation

- [ ] commits table
- [ ] graph_snapshots table
- [ ] snapshot_files table
- [ ] git metadata ingestion
- [ ] `atlas history status`

#### Slice 2 — reusable file-hash historical storage

- [ ] file hash reuse model
- [ ] snapshot membership mapping
- [ ] historical build for bounded commit range

#### Slice 3 — snapshot reconstruction and diff

- [ ] reconstruct graph for commit
- [ ] node diff
- [ ] edge diff
- [ ] file diff
- [ ] `atlas history diff`

#### Slice 4 — history queries

- [ ] symbol history
- [ ] file history
- [ ] dependency history
- [ ] module history

#### Slice 5 — analytics and retention

- [ ] churn metrics
- [ ] stability metrics
- [ ] prune policies
- [ ] storage diagnostics

### 17.20 Completion criteria

Phase 17 is complete when all of these are true:

- [ ] Atlas can persist commit-linked graph snapshots
- [ ] unchanged file graphs are reused across commits
- [ ] Atlas can diff two snapshots structurally
- [ ] Atlas can answer symbol/file/dependency history queries
- [ ] Atlas can report churn/stability metrics
- [ ] storage growth is measurable and bounded by policy
- [ ] all historical outputs are deterministic and evidence-backed

### 17.21 Guiding rules

- [ ] correctness before optimization
- [ ] reuse unchanged file graphs across history
- [ ] prefer explicit evidence over inferred continuity
- [ ] keep history queries deterministic
- [ ] do not add LLM dependence anywhere in this phase

---

## Part II — Release and Interface Gates

Use this part for release definitions and all MCP / agent-facing rollout work.

## Release Gates

Use these as outcome checkpoints between core roadmap completion and broader post-MVP expansion.

### Release 1 Definition (MVP)

Release 1 is done when this works end-to-end:

- [x] `atlas init`
- [x] `atlas build`
- [x] `atlas status`
- [x] `atlas query "some symbol"`
- [x] `atlas update --base origin/main`
- [x] `atlas impact --base origin/main`
- [x] `atlas review-context --base origin/main`

And the system has:

- [x] multi-language parsing for a small v1 language set
- [x] SQLite graph persistence
- [x] file-slice replacement
- [x] recursive impact-radius SQL traversal
- [x] review-context assembly
- [x] FTS5 search
- [x] CI on Linux

---

### Release 2 Definition

Release 2 is done when this works end-to-end:

- [x] `atlas install`
- [x] `atlas update --base origin/main`
- [x] `atlas query "some symbol" --expand`
- [x] `atlas review-context --base origin/main`
- [x] `atlas explain-change --base origin/main`
- [x] `atlas context "what should I read before editing X?"`
- [x] `atlas analyze dead-code --subpath <path>`
- [x] `atlas refactor rename --symbol <qualified-name> --to <new-name> --dry-run`

And system has:

- [x] graph-aware search proven against grep baseline for symbol lookup:
  - [x] exact symbol and qualified-name lookup returns intended definition in top 3 on fixture queries
  - [x] ambiguous short-name lookup returns ranked candidates with kind and file metadata
  - [x] caller/callee/import expansion surfaces relevant graph neighbors plain grep cannot infer
  - [x] fixture evaluation shows better top-1 or top-3 symbol lookup accuracy than plain grep baseline
- [x] reliable impact scoring with test and boundary signals
- [x] workspace-aware package/crate boundaries on multi-package repos:
  - [x] Cargo workspace members resolve into explicit package identities
  - [x] NPM workspace members resolve into explicit package identities
  - [x] Go workspace modules resolve into explicit package identities
  - [x] `cross_package` and related reasoning signals use owning package, not top-level directory name
  - [x] fixture acceptance passes on repo layouts like `packages/<crate>`, `packages/<app>`, or `go.work` multi-module roots
- [x] smart review context with better ranking and trimming
- [x] review-context usefulness acceptance gate passes on fixture repos and changed-file flows
- [x] deterministic context engine with target resolution and code-span selection
- [x] reasoning engine for removal impact, dead code, and refactor risk
- [x] refactor planning with dry-run patch validation
- [x] stable MCP tools usable by agents with token-efficient output
- [x] MCP tool usability acceptance gate passes for agent-facing review, impact, query, and context flows
- [x] watch mode for incremental local updates
- [x] observability/debug tooling for graph integrity and pipeline behavior
- [x] performance that scales to large repos
  - [x] use the current repo on a new disconnected worktree for testing?
- [x] large-repo performance acceptance gate passes on representative repos without memory or latency regressions

---

## MCP and Agent Roadmap

Use this section for MCP-specific rollout, payload design, continuity, and agent-facing tool work. Other phases should point here instead of repeating MCP checklists.

### MCP1 — Core serve foundation

- [x] `build_or_update_graph`
- [x] `get_minimal_context`
- [x] `get_impact_radius`
- [x] `get_review_context`
- [x] `query_graph`
- [x] `traverse_graph`
- [x] `list_graph_stats`
- [x] `detect_changes`
- [x] keep service layer transport-independent
- [x] add stdio server later
- [x] avoid long-running tool deadlocks
- [x] wrap blocking work in dedicated worker threads if needed
- [x] `atlas serve`
- [x] expose only core tools in first version
- [x] add prompts later, not first (MCP prompt templates for external LLMs to use as guidance)

### MCP2 — Public context surface and schema

- [x] expose MCP tool only after JSON shape stabilizes
- [x] decide whether MCP public context surface stays review-focused (`get_review_context`) or adds generic `get_context`
- [x] if generic MCP context tool added, keep it thin over `ContextEngine` with no duplicated ranking/trimming logic
- [x] document MCP tool schemas and response contracts for public/agent use
- [x] add `packages/atlas-mcp` tests for `tools/list`, `tools/call`, argument validation, ambiguity, not-found, and truncation cases
- [x] freeze compact MCP payload contract (`PackagedContextResult` or successor) before broad external use
- [x] confirm public MCP tools stay token-efficient without hiding critical ambiguity/truncation metadata
- [x] MCP adapter thin, no duplicated retrieval logic

### MCP3 — Agent-facing tools and response shaping

- [x] `get_review_context`
- [x] `get_impact_radius`
- [x] `query_graph`
- [x] `explain_change`
- [x] add MCP `resolve_symbol` tool:
  - [x] inputs: `name`, optional `kind`, optional `file`, optional `language`
  - [x] returns exact `qualified_name`, best match, ambiguity list, and suggestions
  - [x] accepts public kind aliases like `function` -> `fn` where qualified-name parsing uses compact tokens
  - [x] example: `resolve_symbol({ "name": "LoadIdentityMessages", "kind": "function", "file": "internal/requestctx/context.go" })` returns `internal/requestctx/context.go::fn::LoadIdentityMessages`
  - [x] resolver removes need for agent workflow: `query_graph` -> copy exact `qualified_name` -> call `symbol_neighbors` / `traverse_graph`
- [x] structured JSON
- [x] stable schemas
- [x] token-efficient responses
- [x] return summaries only
- [x] limit node count
- [x] prioritize relevance

### MCP4 — Alternate payload modes

- [x] add MCP response mode for TOON text output
- [x] use TOON first for context-heavy agent-facing tools
- [x] keep MCP tool contracts stable while swapping payload body format
- [x] add opt-in selection per tool or global config flag

### MCP5 — Continuity, adapters, and saved-context tools

- [x] MCP tool handler execution boundaries
- [x] MCP adapter
- [x] `get_session_status`
- [x] `resume_session`
- [x] `search_saved_context`
- [x] `save_context_artifact`
- [x] `get_context_stats`
- [x] `purge_saved_context`
- [x] add saved-context retrieval tools
- [x] `get_review_context` must emit session events
- [x] `get_impact_radius` must emit session events
- [x] `query_graph` must emit session events
- [x] `detect_changes` must emit session events
- [x] return previews instead of large blobs
- [x] return `source_id` for stored artifacts
- [x] return retrieval hints for follow-up access
- [x] expose compact stats for avoided bytes / stored artifact counts when requested
- [x] include build status in `build_or_update_graph` MCP tool response
- [x] MCP `build_or_update_graph` returns persisted build state

### MCP6 — Content and file discovery

- [x] add MCP tool for file-name/path discovery outside graph-symbol lookup
  - [x] search_files(pattern, globs)
- [x] add MCP tool for content search outside graph-symbol lookup
  - [x] search_content(query, globs, exclude_generated=true)
- [x] support glob include filters
- [x] support ignore rules from `.gitignore` and Atlas config
- [x] exclude or down-rank generated/vendor noise by default
- [x] exclude or down-rank:
  - [x] `node_modules`
  - [x] `package-lock.json`
  - [x] generated bundles
  - [x] vendored static assets
  - [x] minified JS
- [x] keep graph-first workflow for symbol and relationship questions
- [x] use content/file search as fallback when graph lookup is wrong tool
- [x] document when agents should choose `query_graph` vs content/file search
- [x] add tests for Markdown, prompt, SQL, config, and template file discovery
- [x] add tests for ignore-rule behavior and generated-file suppression
- [x] keep response payloads compact and agent-friendly
- [x] use crates globset, ignore, grep (all crates by ripgrep)

### MCP7 — Response provenance and trust metadata

- [x] include compact repo/index provenance metadata in every MCP tool response
- [x] include:
  - [x] `repo_root`
  - [x] `db_path`
  - [x] `indexed_file_count`
  - [x] `last_indexed_at`
  - [x] `result_count`
- [x] include tool-local truncation and paging metadata where relevant
- [x] keep metadata envelope stable across all MCP tools
- [x] ensure metadata does not bloat heavy context payloads
- [x] expose same metadata in JSON and TOON output modes
- [x] add tests that every exported MCP tool returns provenance metadata
- [ ] document metadata contract in MCP reference and agent instructions
- [ ] make mismatched repo/db/index state obvious in agent sessions

### MCP8 — Health and debug command parity

Expose CLI health/debug commands through MCP so agents can verify graph health before trusting graph-backed context.

- [x] add MCP `status` tool
- [x] add MCP `doctor` tool
- [x] add MCP `db_check` tool
- [x] add MCP `debug_graph` tool
- [x] add MCP `explain_query` tool
- [x] keep MCP implementations thin over existing CLI/service-layer diagnostics
- [x] fix `debug-graph --json` edge-schema mismatch before exposing MCP `debug_graph`
  - [x] regression: `atlas db-check --json` OK and `atlas debug-graph --json` must not fail with missing `e.source_qn`
- [x] return compact health summaries by default
- [x] expose machine-readable failure categories for:
  - [x] missing graph DB
  - [ ] schema mismatch
  - [x] interrupted build
  - [x] failed build
  - [ ] stale index
  - [ ] retrieval/content index unavailable
  - [x] corrupt or inconsistent graph rows
- [x] standardize machine-readable error contract across CLI JSON and MCP:
  - [x] `ok`
  - [x] `error_code`
  - [x] `message`
  - [x] `suggestions`
  - [x] unresolved seed and node-not-found cases exit/report consistently
- [x] include repo/index provenance metadata in every health/debug response
- [x] document when agents should call health tools before `query_graph`, `get_context`, and review tools
- [x] add MCP handler tests for healthy repo, missing DB, stale graph, failed build, and schema mismatch

### MCP9 — File and content search expansion

Graph search is symbol-oriented. Add first-class MCP search for prompts, Markdown, SQL templates, config snippets, and embedded text so agents do not need to fall back to `rg`.

- [x] add or verify MCP `search_files`
- [x] add or verify MCP `search_content`
- [x] add MCP `search_templates`
- [x] add MCP `search_text_assets`
- [x] support include/exclude globs consistently across all file/content search tools
- [x] support repo subpath scoping for monorepos
- [x] apply `.gitignore`, Atlas ignore config, generated/vendor suppression, and binary-file skipping by default
- [x] return compact path, line, snippet, score, and match-kind fields
- [ ] expose opt-in richer snippets without bloating default responses
- [ ] document selection rules for `query_graph` vs file/content/template/text-asset search
- [x] add tests for Markdown, prompt files, SQL, config, templates, embedded strings, ignored paths, and generated-file suppression

### MCP10 — Query graph option parity

Expose CLI query options in MCP `query_graph` so agents can use the same ranking and scope controls as CLI users.

- [x] add `subpath` argument
- [x] add `fuzzy` argument
- [x] add `hybrid` argument
- [ ] improve fuzzy symbol typo recovery:
  - [ ] prefer close symbol-name edit distance over weaker Markdown/docs/content token matches
  - [ ] regression: `LoadIdentityMesages` should suggest/rank `LoadIdentityMessages` above Markdown nodes
- [x] expose query explanation in MCP `explain_query`:
  - [x] include ranking factors, filters, FTS terms, fuzzy corrections, regex mode, and active query mode
- [x] clarify `regex` mode behavior in schema docs:
  - [x] regex-only structural scan
  - [x] text + regex post-filter over FTS candidates
  - [x] invalid regex error shape
- [ ] evaluate and add `include_files` argument if file nodes improve agent workflows
- [x] make `subpath` filtering happen before ranking where possible
- [x] include active query mode in response metadata
- [x] add tests for monorepo subpath filtering, fuzzy ranking, hybrid mode, regex-only lookup, text+regex filtering, and invalid regex

### MCP10.1 — Analysis tool MCP wrappers

Expose CLI analysis commands directly through MCP with compact, agent-oriented defaults.

- [x] add MCP `analyze_safety`
- [x] add MCP `analyze_remove`
- [x] add MCP `analyze_dead_code`
- [x] add MCP `analyze_dependency`
- [x] keep wrappers thin over `ReasoningEngine`
- [x] default MCP analysis output to summaries plus bounded evidence
- [x] include applied limits, omitted counts, and truncation metadata
- [x] reuse standardized error contract for unresolved seeds and ambiguous symbols

### MCP11 — Review and impact change-source parity

Make `get_impact_radius` and `get_review_context` accept the same change-source controls as CLI and `detect_changes`.

- [x] add `base` argument to `get_impact_radius`
- [x] add `staged` argument to `get_impact_radius`
- [x] add `working_tree` argument to `get_impact_radius`
- [x] add `base` argument to `get_review_context`
- [x] add `staged` argument to `get_review_context`
- [x] add `working_tree` argument to `get_review_context`
- [x] keep explicit `files` input supported for direct callers
- [x] reject ambiguous combinations with clear validation errors
- [x] reuse `detect_changes` change detection logic instead of duplicating git behavior
- [x] include resolved changed-file set and source mode in response metadata
- [x] add tests for explicit files, base diff, staged diff, working-tree diff, empty diff, and invalid argument combinations

### MCP11.1 — Graph freshness warnings

Warn agents when graph-backed answers may be stale for changed code files.

- [x] compare queried files/symbol files against changed files from `detect_changes`
- [x] emit freshness warnings in:
  - [x] `query_graph`
  - [x] `get_context`
  - [x] `get_review_context`
  - [x] `get_impact_radius`
- [x] warn only for code/path changes that can affect graph facts; do not warn for zero-node files like `.gitignore` unless relevant
- [x] include suggested recovery, for example `build_or_update_graph`
- [x] add tests for clean repo, changed code file, changed non-code zero-node file, and stale changed symbol

### MCP12 — Context detail controls

Expose `atlas context` detail toggles through MCP `get_context` so agents can tune token use without changing service internals.

- [x] add `max_files` argument
- [x] add `code_spans` argument
- [x] add `tests` argument
- [x] add `imports` argument
- [x] add `neighbors` argument
- [x] add `semantic` argument
- [x] keep defaults token-efficient and compatible with current MCP behavior
- [x] route all toggles through `ContextEngine` request options
- [x] include applied limits, omitted sections, and truncation metadata in responses
- [x] document token-use tradeoffs for each toggle in MCP reference
- [x] add tests for each toggle, combined toggles, limit enforcement, and JSON/TOON output parity

### MCP13 — Saved context read-by-id

Add direct full-artifact retrieval by `source_id`; search previews are not enough after agents save large context.

- [ ] add MCP `read_saved_context` tool
- [ ] accept `source_id`
- [ ] support optional paging or byte/token caps for large artifacts
- [ ] return full content when within configured limits
- [ ] return truncation metadata and continuation hints when content exceeds limits
- [ ] preserve existing preview behavior in `search_saved_context`
- [ ] enforce session/repo scoping so one session cannot read unrelated saved artifacts accidentally
- [ ] include artifact metadata:
  - [ ] `source_id`
  - [ ] artifact kind
  - [ ] created time
  - [ ] session id
  - [ ] byte count
  - [ ] chunk count
- [ ] add tests for found artifact, missing artifact, oversized artifact, paged artifact, and cross-session/repo isolation

---

## Part III — Post-MVP Product Expansion

Use this part for advanced retrieval, analysis, refactoring, observability, real-time updates, insights, optional features, and MCP-facing payload optimizations.

These phases extend v1 after core graph/build/update/query path is reliable.

## Phase 18 — Retrieval & Search Intelligence

### 18.1 Hybrid search

- [x] keep SQLite FTS5 as baseline
- [x] add embeddings behind optional toggle
- [x] chunk symbol-sized nodes for retrieval
- [x] generate embeddings
- [x] store vectors in SQLite or external store
- [x] implement hybrid retrieval:
  - [x] FTS results
  - [x] vector results
  - [x] reciprocal-rank fusion merge

### 18.2 Ranking improvements

- [x] exact name boost
- [x] qualified-name boost
- [x] fuzzy match
  - [ ] NOTE: current fuzzy behavior still needs symbol-typo recovery hardening; typoed symbols must not lose to docs/config nodes
- [x] camelCase/snake_case token split
- [x] recent-file boost
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
- [x] optional Tree-sitter incremental parsing

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

- [x] summarize diff
- [x] list impacted components
- [x] explain ripple effects

### 21.2 Smart review context

- [x] prioritize high-impact nodes
- [x] include call chains
- [x] remove noise

### 21.3 Natural-language queries

- [x] support `where is X used`
- [x] support `what calls Y`
- [x] support `what breaks if I change Z`
- [x] map intent to graph query

### 21.4 CLI UX

- [x] interactive shell (`atlas shell`)
- [x] fuzzy search
- [x] paging
- [x] colored output

## Phase 22 — Context Engine

Build deterministic retrieval-and-selection layer over graph. No LLM dependence. Input structured request or simple text. Output bounded, explainable context for CLI, review flow, later agent flow.

### 22.0 Recommended implementation order

Implement Phase 22 in this order so each slice reuses existing store/search/review pieces and leaves Phase 23-25 with stable contracts instead of churn.

#### 22.0.1 Core types and crate boundary

- [x] decide crate home for context engine (`packages/atlas-review` if scope stays retrieval-only, new crate only if responsibilities outgrow review assembly)
- [x] add `ContextIntent`, `ContextTarget`, `ContextRequest`, `ContextResult`, `SelectedNode`, `SelectedEdge`, `SelectedFile`
- [x] add serde/json contracts now so CLI, MCP, later reasoning reuse same payloads
- [x] keep v1 API small: symbol/file/review/impact requests only
- [x] define truncation + ambiguity metadata up front to avoid later breaking changes

Why first:
- existing Phase 22, 23, 25 all depend on stable request/response shapes
- cheapest place to lock naming, limits, and explainability fields

Exit criteria:
- [x] model types compile
- [x] json snapshot tests cover serialize/deserialize round-trip

#### 22.0.2 Store/query support needed by engine

- [x] audit and expose exact helper queries from SQLite store before engine logic grows
- [x] add focused store helpers for direct callers, direct callees, import neighbors, containment neighbors, node lookup by qname/name/path
- [x] keep helpers deterministic and bounded; avoid embedding ranking policy in SQL
- [x] reuse existing `search`, `impact_radius`, `find_dependents_for_qnames`, review assembly inputs where possible

Why second:
- engine should compose verified primitives, not hide SQL inside ranking code
- reduces duplicate traversal logic across context, review, later reasoning

Exit criteria:
- [x] unit tests for each helper query on small graph fixtures
- [x] helper outputs stable for missing nodes, ambiguous names, deleted paths

#### 22.0.3 Exact target resolution path

- [x] implement `resolve_target` for qualified name, exact symbol name, exact file path
  - [x] expose this as first-class public resolver surface instead of forcing agents to copy `qualified_name` from `query_graph`
- [x] return single resolved node/file when exact match exists
- [x] return ambiguity metadata with ranked candidates when multiple matches remain
- [x] fallback to existing FTS/hybrid search only after exact paths fail
- [x] accept qualified-name kind aliases during resolution:
  - [x] `::function::` -> `::fn::`
  - [x] `::method::` -> canonical method token
  - [x] documented aliases for language-specific compact tokens
  - [x] normalize user-entered QNs before lookup, for example `internal/requestctx/context.go::function::LoadIdentityMessages` should resolve to `internal/requestctx/context.go::fn::LoadIdentityMessages`
  - [x] keep canonical examples explicit: `internal/requestctx/context.go::fn::LoadIdentityMessages`, `src/lib.rs::fn::foo`
  - [x] if alias-normalized QN still misses, return node-not-found with close symbol/file suggestions instead of a bare failed lookup
- [x] document canonical qualified-name tokens and aliases in CLI/MCP reference

Why third:
- every higher-level context builder depends on trustworthy seed selection
- ambiguity handling must exist before natural-language classifier starts routing requests

Exit criteria:
- [x] tests for exact qname hit
- [x] tests for exact file path hit
- [x] tests for ambiguous short symbol names
- [x] tests for missing target with suggestions

#### 22.0.4 Deterministic symbol-context retrieval

- [x] implement `build_symbol_context` from resolved seed
- [x] retrieve direct node, callers, callees, imports, containment siblings, optional tests
- [x] support one-hop first; gate multi-hop behind explicit request depth
- [x] preserve provenance per selected node/edge (`selection_reason`)

Why fourth:
- smallest useful end-to-end feature
- validates request model, store helpers, scoring inputs, truncation behavior without classifier noise

Exit criteria:
- [x] symbol context returns bounded nodes/edges/files
- [x] direct callers/callees always survive trimming over broad file neighbors
- [x] include/exclude flags work for tests/imports/neighbors

#### 22.0.5 Ranking and trimming policy

- [x] implement `rank_context`
- [x] score by exact-target boost, graph distance, edge confidence, same-file, same-package, public API, test adjacency
- [x] implement `trim_context` with hard node/edge/file caps
- [x] drop low-confidence and distant neighbors before direct relationships
- [x] set truncation flags plus dropped-count metadata

Why fifth:
- retrieval without stable ranking will make CLI/MCP output noisy and hard to trust
- Phase 23 evidence layer wants deterministic scoring factors, not ad hoc ordering

Exit criteria:
- [x] tests prove caller/callee prioritization over sibling/file nodes
- [x] tests prove caps deterministic under tie conditions
- [x] truncated output explains what got cut

#### 22.0.6 Review and impact context builders

- [x] implement `build_review_context` by adapting existing changed-file and impact flow into `ContextResult`
- [x] implement `build_impact_context` from file seeds and changed-symbol seeds
- [x] reuse current `atlas_review::assemble_review_context` until new result shape fully covers it, then consolidate

Why sixth:
- existing review flow already gives working behavior and should become first consumer of shared engine
- proves engine handles both symbol-seeded and change-set-seeded requests

Exit criteria:
- [x] current review-context command can be mapped onto context engine without behavior regression
- [x] impact context returns machine-readable bounded graph slice

#### 22.0.7 Semi-structured query parsing

- [x] add simple classifier for `what breaks`, `used by`, `who calls`, `safe to refactor`, `dead code`, `rename`, `remove dependency`
- [x] add regex extraction for quoted symbols, file paths, function-like names, method-like names
- [x] route parsed text into same `ContextRequest` pipeline used by structured callers
- [x] keep classifier intentionally shallow; no fuzzy LLM-style inference

Why seventh:
- parser should sit on top of stable engine, not drive core architecture
- avoids debugging resolution/ranking bugs through natural-language ambiguity

Exit criteria:
- [x] text requests resolve to same result as equivalent structured requests
- [x] ambiguity metadata survives classifier path

#### 22.0.8 Code spans and source packaging

- [x] include target span first
- [x] include caller/callee spans only when enabled
- [x] extract nearest relevant lines, never whole-file by default
- [x] return file path + line ranges ready for CLI/MCP rendering

Why eighth:
- line packaging depends on already-final selected nodes/files
- keeps early engine work focused on graph correctness before token-shaping

Exit criteria:
- [x] code span tests verify exact lines for target and adjacent symbols
- [x] large file requests stay bounded

#### 22.0.9 Public surfaces

- [x] add internal engine entrypoint `ContextEngine`
- [x] wire CLI prototype behind future `atlas context` surface or hidden/dev command first
- [x] track MCP public-surface rollout in dedicated MCP and Agent Roadmap section
- [x] keep old `review-context` command during transition; switch implementation under hood first

##### 22.0.9.1 Public rollout checklist for `atlas context` and MCP context tools

- [x] unhide `atlas context` once command shape is frozen
- [x] replace dev-style `--qname` / `--name` / `--file` targeting UX with stable public CLI contract
- [x] decide whether `atlas context` accepts free text, explicit subcommands, or both; document one public path
- [x] keep `atlas review-context` during transition; define whether it stays as alias, focused shortcut, or deprecated surface
- [x] document `atlas context` examples and JSON contract in `README.md`
- [x] add CLI parser tests for public `atlas context` syntax
- [x] add fixture/integration tests for `atlas context` symbol, file, review, impact, ambiguity, and not-found flows
- [x] add golden/snapshot coverage for public `atlas context --json` output
- [x] freeze `ContextResult` compatibility expectations for public CLI consumers
- [x] document default limits and truncation behavior for public context output
- [x] MCP-specific public-context checklist consolidated in MCP and Agent Roadmap

Why ninth:
- shipping surface too early freezes unstable payloads
- hidden/dev entrypoint lets fixture tests harden engine before user-facing commit

Exit criteria:
- [x] CLI json output stable enough for golden tests

#### 22.0.10 Finish gates for “context engine complete”

- [x] exact symbol lookup
- [x] ambiguous symbol resolution
- [x] missing symbol behavior
- [x] bounded node trimming
- [x] caller/callee prioritization
- [x] include/exclude tests behavior
- [x] code span selection accuracy
- [x] fixture integration covering review-context parity and context-engine json output
- [x] `cargo test --workspace`
- [x] `cargo clippy --workspace --all-targets --all-features -- -D warnings`

#### 22.0.11 Validate completion rule
- [x] Phase 22 done only when review flow, symbol flow, and impact flow all share same engine contracts and no duplicate ranking/trimming logic remains in CLI or MCP layers

### 22.1 Scope and responsibilities

- [x] accept structured or semi-structured request
- [x] resolve target symbol(s), file(s), or change-set
- [x] retrieve nearby graph structure
- [x] rank retrieved items by relevance
- [x] trim to bounded result size
- [x] return machine-readable context

### 22.2 Request model

- [x] define `ContextIntent` enum:
  - [x] `ImpactAnalysis`
  - [x] `UsageLookup`
  - [x] `RefactorSafety`
  - [x] `DeadCodeCheck`
  - [x] `RenamePreview`
  - [x] `DependencyRemoval`
  - [x] `ReviewContext`
  - [x] `SymbolContext`
- [x] define `ContextTarget` variants:
  - [x] symbol qualified name
  - [x] symbol name
  - [x] file path
  - [x] changed file list
  - [x] changed symbol list
  - [x] edge query seed
- [x] define `ContextRequest` fields:
  - [x] intent
  - [x] target
  - [x] max_nodes
  - [x] max_edges
  - [x] max_files
  - [x] max_depth
  - [x] include_code_spans
  - [x] include_tests
  - [x] include_imports
  - [x] include_callers
  - [x] include_callees
  - [x] include_neighbors

### 22.3 Response model

- [x] define `ContextResult`:
  - [x] resolved target nodes
  - [x] selected nodes
  - [x] selected edges
  - [x] selected files
  - [x] code spans
  - [x] relevance scores
  - [x] truncation flags
  - [x] retrieval metadata
- [x] define `SelectedNode`:
  - [x] node id
  - [x] qualified name
  - [x] kind
  - [x] file path
  - [x] line span
  - [x] relevance score
  - [x] selection reason
- [x] define `SelectedEdge`:
  - [x] source
  - [x] target
  - [x] edge kind
  - [x] depth
  - [x] relevance score
  - [x] selection reason
- [x] define `SelectedFile`:
  - [x] path
  - [x] language
  - [x] reason included
  - [x] node count included

### 22.4 Intent parsing and resolution

- [x] implement exact symbol lookup path
- [x] implement simple query classifier:
  - [x] contains `what breaks`
  - [x] contains `used by`
  - [x] contains `who calls`
  - [x] contains `safe to refactor`
  - [x] contains `dead code`
  - [x] contains `rename`
  - [x] contains `remove dependency`
- [x] add regex extraction for:
  - [x] quoted symbol names
  - [x] file paths
  - [x] function-like names
  - [x] method-like names
- [x] fallback to symbol search + context expansion
- [x] resolve by qualified name
  - [x] NOTE: qualified-name resolution exists, but alias normalization must be implemented on existing CLI/MCP tools so `::function::` and other public names work wherever exact QNs are accepted
  - [x] NOTE: public resolver tooling is still needed so agents do not memorize internal encoding or retry failed calls manually
- [x] resolve by exact symbol name
- [x] resolve by file path
- [x] resolve by ranked search if ambiguous
- [x] return ambiguity metadata if multiple candidates remain, including import/call-resolution ties

### 22.5 Retrieval, ranking, trimming

- [x] fetch direct node record
- [x] fetch direct callers
- [x] fetch direct callees
- [x] fetch import edges
- [x] fetch file containment edges
- [x] fetch test adjacency if enabled
- [x] fetch one-hop neighbors
- [x] fetch multi-hop neighbors if requested
- [x] rank highest:
  - [x] exact target node
  - [x] direct callers
  - [x] direct callees
- [x] rank medium:
  - [x] same-file siblings
  - [x] tests targeting target node
  - [x] imports linked to target file
- [x] rank lower:
  - [x] second-hop neighbors
  - [x] broad file-level nodes
  - [x] weak reference edges
- [x] add scoring factors:
  - [x] graph distance
  - [x] edge confidence
  - [x] same file boost
  - [x] same package/module boost
  - [x] public API boost
  - [x] test adjacency boost
- [x] hard-limit nodes
- [x] hard-limit edges
- [x] hard-limit files
- [x] prefer direct relationships over broad context
- [x] drop low-confidence edges first
- [x] drop distant neighbors before dropping direct callers/callees
- [x] mark output as truncated if limits applied

### 22.6 Code spans, APIs, tests

- [x] include target symbol span
- [x] include caller/callee spans if enabled
- [x] include nearest relevant lines only
- [x] avoid whole-file dumps by default
- [x] provide file path + line range references
- [x] create `ContextEngine`
- [x] implement:
  - [x] `resolve_target`
  - [x] `build_symbol_context`
  - [x] `build_review_context`
  - [x] `build_impact_context`
  - [x] `rank_context`
  - [x] `trim_context`
- [x] tests:
  - [x] exact symbol lookup
  - [x] ambiguous symbol resolution
  - [x] missing symbol behavior
  - [x] bounded node trimming
  - [x] caller/callee prioritization
  - [x] include/exclude tests behavior
  - [x] code span selection accuracy

## Phase 23 — Autonomous Code Reasoning

Answer structural questions from graph + parser + store facts only. No unsupported claims. Return structured findings with evidence and certainty.

### 23.1 Engine responsibilities and core types

- [x] analyze removal impact
- [x] detect dead code candidates
- [x] score refactor safety
- [x] validate dependency removal
- [x] inspect rename blast radius
- [x] classify change risk
- [x] detect missing test adjacency
- [x] explain graph facts behind result
- [x] define `ReasoningResult`
- [x] define `ReasoningEvidence`
- [x] define `ReasoningWarning`
- [x] define `ConfidenceTier`
- [x] define `SafetyScore`
- [x] define `ImpactClass`
- [x] define `DeadCodeCandidate`
- [x] define `DependencyRemovalResult`
- [x] define `RenamePreviewResult`

### 23.2 Removal impact analysis

- [x] accept symbol or file as seed
- [x] find direct inbound edges
- [x] find direct outbound edges
- [x] traverse impact graph to configured depth
- [x] separate:
  - [x] definitely impacted
  - [x] probably impacted
  - [x] weakly related
- [x] return:
  - [x] impacted symbols
  - [x] impacted files
  - [x] impacted tests
  - [x] relevant edges
- [x] use high-confidence heuristics:
  - [x] direct call edges
  - [x] direct import edges
  - [x] direct test links
- [x] use medium-confidence heuristics:
  - [x] inferred symbol links
  - [x] unresolved selector calls within same file/package
- [x] use low-confidence heuristics:
  - [x] textual references only
  - [x] weak unresolved edges
- [x] include seed node(s)
- [x] include per-node depth
- [x] include edge kind per path
- [x] include impact class
- [x] demote containment-only relationships in removal/refactor analysis:
  - [x] keep containment siblings as secondary context
  - [x] do not count `contains` edges as probable impact unless explicitly requested
  - [x] add regression for `analyze remove <symbol>` inflated by file/package containment

### 23.3 Dead code, safety, dependency removal

- [x] detect dead code candidates when:
  - [x] no inbound call edges
  - [x] no inbound reference edges
  - [x] not public/exported
  - [x] not in configured entrypoint allowlist
  - [x] not framework entrypoint
  - [x] not test
  - [x] not referenced by known routing/config conventions
  - [x] not dynamically registered where detectable
- [x] support suppression / allowlists:
  - [x] main entrypoints
  - [x] CLI command handlers
  - [x] framework lifecycle hooks
  - [x] exported plugin symbols
  - [x] reflection-based registration points
  - [x] manual ignore list
- [x] dead-code output:
  - [x] candidate symbol
  - [x] why flagged
  - [x] certainty tier
  - [x] blockers preventing auto-removal
- [x] score refactor safety from:
  - [x] fan-in
  - [x] fan-out
  - [x] visibility
  - [x] file/module scope
  - [x] public API status
  - [x] linked test count
  - [x] dependency depth
  - [x] dynamic usage risk
  - [x] unresolved edge count
- [x] mark safer when:
  - [x] private/internal
  - [x] low fan-in
  - [x] low fan-out
  - [x] strong test adjacency
  - [x] self-contained in one file/module
- [x] mark riskier when:
  - [x] public/exported
  - [x] many inbound callers
  - [x] many cross-module callers
  - [x] unresolved/dynamic references
  - [x] no tests nearby
- [x] refactor-safety output:
  - [x] numeric score
  - [x] safety band: `safe`, `caution`, `risky`
  - [x] reasons list
  - [x] suggested validations
- [x] validate dependency removal:
  - [x] detect unused imports
  - [x] detect unreferenced dependencies
  - [x] verify zero references in graph
  - [x] verify zero references in same-file AST if needed
  - [x] verify no tests depend on target
  - [x] flag dynamic/reflective uncertainty
- [x] dependency-removal output:
  - [x] removable boolean
  - [x] blocking references
  - [x] confidence tier
  - [x] suggested cleanup edits
- [x] default `analyze dead-code` to code symbols only:
  - [x] functions
  - [x] methods
  - [x] structs/types/classes/interfaces/traits/enums
  - [x] exported constants/vars where language supports them
  - [x] exclude Markdown, TOML keys, docs/config nodes unless `--include-non-code`
- [x] add analysis output controls:
  - [x] `--limit`
  - [x] `--max-edges`
  - [x] `--max-files`
  - [x] `--summary`
  - [x] `--exclude-kind`
  - [x] `--code-only`
- [x] enforce compact defaults for MCP analysis wrappers

### 23.4 Rename radius, test adjacency, risk, APIs

- [x] preview rename blast radius:
  - [x] locate definition
  - [x] locate all references
  - [x] classify references as same-file, same-module/package, cross-module/package, tests
  - [x] detect unresolved references needing manual review
  - [x] count affected files
  - [x] count affected symbols
- [x] rename output:
  - [x] affected references
  - [x] affected files
  - [x] risk level
  - [x] collision warnings
  - [x] manual review flags
- [x] estimate test adjacency:
  - [x] map tests to symbols where possible
  - [x] map changed symbol to direct tests
  - [x] map changed symbol to same-file tests
  - [x] map changed symbol to same-module tests
  - [x] flag no linked tests
  - [x] flag weak test adjacency only
  - [x] NOTE: current safety output can understate coverage; distinguish direct, indirect-through-callers, package-level, and no-known coverage
- [x] test-adjacency output:
  - [x] linked tests
  - [x] coverage strength
  - [x] recommendation flag
- [x] classify change risk from:
  - [x] public API touched
  - [x] test adjacency strength
  - [x] cross-module impact
  - [x] inbound caller count
  - [x] unresolved references
  - [x] dependency fan-out
  - [x] impacted file count
- [x] risk output:
  - [x] low / medium / high
  - [x] contributing factors
  - [x] suggested review focus
- [x] create `ReasoningEngine`
- [x] implement:
  - [x] `analyze_removal`
  - [x] `detect_dead_code`
  - [x] `score_refactor_safety`
  - [x] `check_dependency_removal`
  - [x] `preview_rename_radius`
  - [x] `classify_change_risk`
  - [x] `find_test_adjacency`
- [x] tests:
  - [x] simple call graph impact
  - [x] cyclic graph impact
  - [x] dead private function candidate
  - [x] exported/public function not flagged
  - [x] entrypoint suppression
  - [x] rename blast radius same file
  - [x] rename blast radius cross module
  - [x] dependency removal blocked by reference
  - [x] missing test signal for changed symbol
  - [x] risk scoring sanity checks

## Phase 24 — Smart Refactoring Core

Deterministic, syntax-aware transforms backed by graph validation. Start with strongly checkable operations only: rename, dead-code removal, import cleanup. Keep extract-function as detection/planning first.

### 24.1 Responsibilities and operation model

- [x] plan refactor
- [x] simulate impact before apply
- [x] apply deterministic text/AST edits
- [x] validate no collisions
- [x] validate references updated
- [x] emit patch preview
- [x] support dry-run mode
- [x] support rollback on validation failure
- [x] define `RefactorOperation` enum:
  - [x] `RenameSymbol`
  - [x] `RemoveDeadCode`
  - [x] `CleanImports`
  - [x] `ExtractFunctionCandidate`
- [x] define `RefactorPlan`
- [x] define `RefactorEdit`
- [x] define `RefactorPatch`
- [x] define `RefactorValidationResult`
- [x] define `RefactorDryRunResult`

### 24.2 Rename symbol

- [x] require unique definition resolution
- [x] require valid new identifier
- [x] reject local collision at definition site
- [x] reject obvious collision in affected scopes
- [x] resolve definition node
- [x] gather all references
- [x] classify references by certainty
- [x] build edit set
- [x] simulate rename impact
- [x] apply edits in stable order
- [x] validate resulting references
- [x] rename validation:
  - [x] renamed definition exists
  - [x] all expected references updated
  - [x] no duplicate definitions created
  - [x] no blocked write targets
  - [x] unresolved/manual-review references reported separately
- [x] rename output:
  - [x] files changed
  - [x] edits count
  - [x] manual review list
  - [x] patch preview

### 24.3 Remove dead code and clean imports

- [x] remove dead code only when:
  - [x] candidate has sufficient confidence
  - [x] no protected entrypoint status
  - [x] no unresolved high-risk blockers
- [x] dead-code removal steps:
  - [x] select removable node
  - [x] remove symbol span
  - [x] clean surrounding whitespace/comments if safe
  - [x] run import cleanup on touched file
  - [x] update graph slice
- [x] dead-code validation:
  - [x] symbol definition removed
  - [x] no dangling same-file references
  - [x] import cleanup stable
  - [x] patch preview generated
- [x] import-cleanup steps:
  - [x] compute actual symbol usage in file
  - [x] compare imports vs usage
  - [x] mark unused imports
  - [x] remove unused imports
  - [x] normalize spacing/order if formatter integration exists later
- [x] import-cleanup validation:
  - [x] no used import removed
  - [x] file remains syntactically valid if parser re-check exists
  - [x] no duplicate imports created

### 24.4 Extract-function detection, simulation, APIs, tests

- [x] detect extract-function candidates from:
  - [x] large contiguous block
  - [x] repeated block pattern
  - [x] clear input variables
  - [x] clear output variables
  - [x] limited side-effect boundaries
- [x] score candidates with:
  - [x] repeated logic boost
  - [x] long block boost
  - [x] low free-variable count boost
  - [x] low control-flow complexity boost
- [x] candidate output:
  - [x] span
  - [x] proposed inputs
  - [x] proposed outputs
  - [x] extraction difficulty score
  - [x] no auto-apply in initial version
- [x] run impact simulation before non-trivial refactor:
  - [x] rename blast radius
  - [x] removal impact
  - [x] safety score
  - [x] affected files
  - [x] affected symbols
  - [x] nearby tests
  - [x] unresolved risks
- [x] patch and dry-run support:
  - [x] generate unified diff preview
  - [x] support `--dry-run`
  - [x] support per-file edit grouping
  - [x] support machine-readable edit output
  - [x] support cancellation before apply
- [x] create `RefactorEngine`
- [x] implement:
  - [x] `plan_rename`
  - [x] `apply_rename`
  - [x] `plan_dead_code_removal`
  - [x] `apply_dead_code_removal`
  - [x] `plan_import_cleanup`
  - [x] `apply_import_cleanup`
  - [x] `detect_extract_function_candidates`
  - [x] `simulate_refactor_impact`
- [x] add safety checks:
  - [x] file write safety
  - [x] edit overlap detection
  - [x] parser revalidation hook later
  - [x] reject unsafe overlapping edits
  - [x] reject ambiguous rename targets
  - [x] reject low-confidence dead code removals by default
- [x] tests:
  - [x] rename single-file symbol
  - [x] rename multi-file symbol
  - [x] rename collision rejection
  - [x] dead code removal private helper
  - [x] protected entrypoint not removed
  - [x] unused import removed
  - [x] used import preserved
  - [x] extract-function candidate detection basic case
  - [x] dry-run output stable
  - [x] patch output stable

## Phase 25 — Shared Analysis and Refactor Infrastructure

Shared support for explainability, config, CLI surface, JSON contracts, benchmarks. Phase 22-24 depend on this.

### 25.1 Evidence and explainability

- [x] attach evidence edges
- [x] attach evidence nodes
- [x] attach scoring factors
- [x] attach uncertainty flags

### 25.2 Config surface

- [x] max context nodes
- [x] max context depth
- [x] dead code certainty threshold
- [x] refactor safety threshold
- [x] impact max depth
- [x] impact max nodes
- [x] dynamic usage allowlist
- [x] entrypoint allowlist
- [x] framework conventions file

### 25.3 Language support policy

- [x] phase-2 features degrade gracefully by language
- [x] enable rename only where symbol/reference mapping is mature
- [x] enable dead code only where inbound usage confidence is acceptable
- [x] enable import cleanup only where parser support is reliable

### 25.4 CLI surfaces

- [x] `atlas context <symbol>`
- [x] `atlas analyze remove <symbol>`
  - [x] add compact output controls and containment-noise demotion from Phase 23 follow-up
- [x] `atlas analyze dead-code`
  - [x] add code-only default and output limits from Phase 23 follow-up
- [x] `atlas analyze safety <symbol>`
  - [x] distinguish direct, indirect-through-callers, package-level, and missing test coverage
- [x] `atlas analyze dependency <symbol-or-import>`
- [x] `atlas refactor rename <symbol> <new-name> --dry-run`
- [x] `atlas refactor remove-dead <symbol> --dry-run`
- [x] `atlas refactor clean-imports <file> --dry-run`

### 25.5 JSON output, benchmarks, completion criteria

- [x] stable JSON schema for all analysis commands
  - [x] NOTE: stable schema exists, but unresolved seeds and warnings need standardized `ok/error_code/message/suggestions` contract
- [x] stable JSON schema for patch previews
- [x] include evidence and certainty fields
- [x] benchmark context retrieval latency
- [x] benchmark impact analysis latency
- [x] benchmark dead-code scan latency
- [x] benchmark rename planning latency
- [x] benchmark import-cleanup latency
- [x] completion criteria:
  - [x] context engine resolves and returns bounded symbol/change context
  - [x] removal impact analysis works on representative repos
  - [x] dead code detection produces useful candidates with suppressions
  - [x] refactor safety scoring is implemented and explainable
  - [x] dependency removal checks are implemented
  - [x] rename blast radius is implemented
  - [x] deterministic rename refactor works in dry-run and apply modes
  - [x] deterministic dead code removal works for high-confidence candidates
  - [x] import cleanup works reliably
  - [x] extract-function candidate detection exists even if auto-apply stays out of scope

## Phase 26 — MCP / Agent Integration

### 26.1 Status

Detailed MCP tool rollout, schema work, and response shaping now live in Part II under MCP and Agent Roadmap.

## Phase 27 — Observability

### 27.1 Metrics

- [x] indexing time
- [x] nodes/sec
- [x] query latency
- [x] impact latency

### 27.2 Debug tools

- [x] `atlas doctor`
- [x] `atlas debug graph`
  - [x] NOTE: `debug-graph --json` currently needs schema-mismatch fix for edge columns (`e.source_qn` failure)
- [x] `atlas explain-query`
  - [x] expose same query-explanation details through MCP `explain_query`

### 27.3 Data integrity

- [x] orphan-node detection
  - [x] add regression that orphan-node query uses current edge schema column names
- [x] edge validation
- [x] DB consistency checks

## Phase 28 — Real-Time & Continuous Mode

Deterministic watch flow on top of existing incremental pipeline. Goal: near-real-time graph freshness without full rebuilds for small edits.

### 28.1 Watch mode scope

- [x] auto-update graph when files change
- [x] stay efficient on rapid edit bursts
- [x] avoid full rebuild path for ordinary edits
- [x] integrate with existing incremental parse + update flow

### 28.2 File watcher

- [x] choose watcher crate (for example `notify`)
- [x] watch repo directories recursively
- [x] ignore:
  - [x] `.git`
  - [x] build directories
  - [x] ignored paths
- [x] map watch roots to normalized repo-relative paths
- [x] handle platform-specific watcher quirks

### 28.3 Change detection

- [x] detect:
  - [x] file create
  - [x] file modify
  - [x] file delete
  - [x] file rename
- [x] map events to file paths
- [x] normalize duplicate event bursts
- [x] keep delete/rename handling consistent with batch update mode

### 28.4 Update pipeline integration

- [x] on change enqueue file for update
- [x] batch changes with debounce window (`100–500ms`)
- [x] trigger:
  - [x] incremental parsing
  - [x] graph update
- [x] reuse existing update/build primitives where practical
- [x] avoid duplicate queue entries for same file

### 28.5 Incremental update logic

- [x] reuse existing update logic
- [x] handle:
  - [x] modified files
  - [x] deleted files
  - [x] renamed files
- [x] preserve dependent invalidation rules
- [x] ensure graph slice replacement semantics stay atomic

### 28.6 Queue, workers, state

- [x] create update queue
- [x] worker responsibilities:
  - [x] parse file
  - [x] update graph
- [x] ensure:
  - [x] single DB writer
  - [x] no race conditions
- [x] track:
  - [x] pending updates
  - [x] in-progress updates
  - [x] last update time
- [x] expose internal state for status/debug surfaces later

### 28.7 Performance and failure handling

- [x] debounce rapid file changes
- [x] coalesce duplicate updates
- [x] limit concurrent parsing
- [x] handle parse failures gracefully
- [x] add retry logic only if bounded and safe
- [x] log watch/update errors
- [x] keep watch loop alive after recoverable failures

### 28.8 CLI and tests

- [x] add `atlas watch`
- [x] show:
  - [x] files updated
  - [x] nodes updated
  - [x] errors
- [x] support JSON output if command surface standardizes on it
- [x] tests:
  - [x] file modify triggers update
  - [x] file delete removes graph slice
  - [x] rename handled correctly
  - [x] debounce works
  - [x] no duplicate updates
- [x] completion criteria:
  - [x] watch mode updates graph in near real-time
  - [x] no full rebuild required for small changes
  - [x] queue and writer path remain race-free

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

## Phase 31 — Lowest Priority

### 31.1 Wiki / docs generation (CLI command)

- [ ] generate Markdown docs
- [ ] module pages
- [ ] function pages
- [ ] static site export

## Phase 32 — TOON Output

TOON for LLM-facing MCP output only. Goal: reduce token usage for review and context payloads without changing Atlas core storage, parser, or JSON-RPC transport. Prefer official Rust TOON library (official only) (`toon-format/toon-rust`) over a custom Atlas encoder. Build Atlas-specific adapter code only where library integration is insufficient.

### 32.1 Scope and boundaries

- [x] evaluate official Rust TOON library for Atlas use
- [x] add TOON dependency only if maintenance, API shape, and spec coverage are acceptable
- [x] create thin Atlas adapter layer only if needed
- [x] keep TOON limited to LLM-facing MCP output
- [x] keep JSON as baseline and fallback output
- [x] do not use TOON for SQLite persistence, internal domain models, or MCP transport framing
- [x] avoid custom TOON implementation unless official library is blocked or insufficient

### 32.2 Encoding MVP

- [x] encode `serde_json::Value` to TOON through library API
- [x] confirm support for objects, arrays, strings, numbers, booleans, and null
- [x] confirm deterministic field ordering behavior or add wrapper normalization
- [x] confirm canonical number formatting behavior or add wrapper normalization
- [x] confirm delimiter-aware quoting rules
- [x] confirm inline primitive arrays
- [x] confirm tabular encoding for uniform arrays of primitive-only objects
- [x] confirm expanded encoding for mixed or nested arrays
- [x] add Atlas-side fallback/error path when payload shape exceeds supported library behavior

### 32.3 MCP integration

- [x] tracked in MCP4 under MCP and Agent Roadmap

### 32.4 Validation and quality gates

- [x] add fixture tests from TOON spec examples for supported library subset
- [x] add round-trip tests for Atlas-produced payloads where feasible
- [x] reject unsupported cases instead of emitting ambiguous output
- [x] benchmark token count and response size vs JSON on representative MCP payloads
- [x] document exact supported TOON subset, library choice, pinned version, and deliberate deviations from full spec

---

## Part IV — Context Continuity and Memory

Use this part for session persistence, saved artifacts, retrieval-backed resume, and long-lived memory work.

## Context-Mode and Continuity Roadmap

These phases cover continuity storage, session lifecycle, retrieval-backed restoration, memory quality, and longer-term cross-session intelligence.

### Overview

Extend Atlas with context-mode persistence and session continuity without mixing those concerns into graph database.

This backlog covers pieces needed for:

- artifact persistence
- session continuity
- resume snapshots
- retrieval-backed restoration

#### Core Design Rules

- DO NOT store saved context in graph database
- DO NOT replay raw command history into future sessions
- ALWAYS restore context through retrieval
- ALWAYS store large outputs outside model context
- KEEP graph storage, content storage, and session storage as separate systems
- KEEP continuity best-effort; never block primary CLI/MCP flow on session persistence failure
- KEEP retrieval lexical and local first; embeddings are optional later, not required for v1 context-mode completion

### Phase CM1 — Foundation and crate boundaries

Create storage and adapter boundaries first so later work does not leak session or artifact concerns into graph code.

#### New crates

- [x] `packages/atlas-contentstore`
- [x] `packages/atlas-session`
- [x] `packages/atlas-contextsave`
- [x] `packages/atlas-adapters`

#### Session identity model

- [x] define `session_id = hash(repo_root + worktree + frontend)`
- [x] normalize paths before hashing
- [x] keep worktree isolation

Why first:
- later content/session/MCP work all depend on stable boundaries
- prevents graph DB and transport layers from becoming persistence dumping grounds

Exit criteria:
- [x] crates compile with narrow responsibilities
- [x] session identity rules are fixed before persistence APIs spread

### Phase CM2 — Content store for saved artifacts

Build durable artifact storage before eventing so large outputs already have somewhere safe to go.

#### Database

- [x] create SQLite database at `.atlas/context.db`
- [x] enable `PRAGMA journal_mode=WAL;`
- [x] enable `PRAGMA synchronous=NORMAL;`
- [x] enable `PRAGMA foreign_keys=ON;`
- [x] enable `PRAGMA busy_timeout=5000;`
- [x] enable FTS5 support
- [x] keep this database separate from `.atlas/worldtree.db`

#### Required tables

`sources`

- [x] `id TEXT PRIMARY KEY`
- [x] `session_id TEXT`
- [x] `source_type TEXT NOT NULL`
- [x] `label TEXT NOT NULL`
- [x] `repo_root TEXT`
- [x] `created_at TEXT NOT NULL`

`chunks`

- [x] `id INTEGER PRIMARY KEY`
- [x] `source_id TEXT NOT NULL`
- [x] `content TEXT NOT NULL`
- [x] `content_type TEXT NOT NULL`
- [x] `chunk_index INTEGER NOT NULL`
- [x] `title TEXT`
- [x] `metadata_json TEXT NOT NULL`
- [x] `created_at TEXT NOT NULL`

`chunks_fts`

- [x] FTS5 virtual table indexing `title`
- [x] FTS5 virtual table indexing `content`
- [x] FTS5 virtual table indexing `source_id`
- [x] FTS5 virtual table indexing `content_type`

`chunks_trigram`

- [x] trigram FTS5 table for typo-tolerant fallback retrieval

`vocabulary`

- [x] vocabulary table for bounded fuzzy correction and term suggestions

#### Content store API

- [x] `open(path)`
- [x] `migrate()`
- [x] `index_artifact(source_meta, raw_text, content_type)`
- [x] `search(query, filters)`
- [x] `get_source(source_id)`
- [x] `get_chunks(source_id)`
- [x] `delete_source(source_id)`
- [x] `cleanup(retention_policy)`

#### Chunking rules

- [x] markdown must split by headings first
- [x] plain text must split by paragraph blocks or line windows
- [x] JSON must split by path and array batches
- [x] oversized chunks must be subdivided
- [x] each chunk must preserve stable `chunk_index`
- [x] each chunk should preserve human-readable `title` when possible

#### Compression routing

- [x] if output is below small-output threshold, return raw output directly
- [x] if output is above preview threshold, index it and return compact preview
- [x] if output is above large-output threshold, index it and return pointer only
- [x] never put raw large output into future prompts

#### Retrieval quality stack

- [x] keep byte-threshold routing configurable and documented
- [x] add `search_with_fallback(query, filters)`
- [x] add source/content-type aware ranking
- [x] add BM25 title weighting
- [x] add trigram fallback retrieval
- [x] add reciprocal-rank fusion between lexical and trigram results
- [x] add vocabulary-based fuzzy correction
- [x] add proximity reranking for multi-term queries
- [x] add title boosts for high-signal matches

Why second-and-a-half:
- stored artifacts are only useful for continuity if retrieval quality is good enough to recover exact prior tool results and topics
- parity target is retrieval-driven compression, not blob storage alone

Why second:
- session events need artifact references from day one
- retrieval-backed restore is impossible without persisted chunks and source ids

Exit criteria:
- [x] large artifacts can be stored and retrieved by `source_id`
- [x] chunking is deterministic enough for tests and follow-up retrieval
- [x] saved artifact search is strong enough to recover relevant prior results without replaying raw history

### Phase CM3 — Session store and event ledger

Persist session facts and bounded events next so every later surface can write into one service.

#### Database

- [x] create SQLite database at `.atlas/session.db`

#### Required tables

`session_meta`

- [x] `session_id TEXT PRIMARY KEY`
- [x] `repo_root TEXT NOT NULL`
- [x] `frontend TEXT NOT NULL`
- [x] `worktree_id TEXT`
- [x] `created_at TEXT NOT NULL`
- [x] `updated_at TEXT NOT NULL`
- [x] `last_resume_at TEXT`
- [x] `last_compaction_at TEXT`

`session_events`

- [x] `id INTEGER PRIMARY KEY`
- [x] `session_id TEXT NOT NULL`
- [x] `event_type TEXT NOT NULL`
- [x] `priority INTEGER NOT NULL`
- [x] `payload_json TEXT NOT NULL`
- [x] `event_hash TEXT NOT NULL`
- [x] `created_at TEXT NOT NULL`

`session_resume`

- [x] `session_id TEXT PRIMARY KEY`
- [x] `snapshot TEXT NOT NULL`
- [x] `event_count INTEGER NOT NULL`
- [x] `consumed INTEGER NOT NULL DEFAULT 0`
- [x] `created_at TEXT NOT NULL`
- [x] `updated_at TEXT NOT NULL`

#### Event rules

- [x] deduplicate events using `event_hash`
- [x] keep maximum number of events per session
- [x] evict events by lower priority first
- [x] evict events by older records first
- [x] never store large raw output in `session_events`
- [x] large raw output must be stored in content store and referenced from session event payload

#### Fixed event types

- [x] `FILE_READ`
- [x] `FILE_WRITE`
- [x] `COMMAND_RUN`
- [x] `COMMAND_FAIL`
- [x] `GRAPH_BUILD`
- [x] `GRAPH_UPDATE`
- [x] `REVIEW_CONTEXT`
- [x] `IMPACT_ANALYSIS`
- [x] `CONTEXT_REQUEST`
- [x] `REASONING_RESULT`
- [x] `USER_INTENT`
- [x] `ERROR`
- [x] `SESSION_START`
- [x] `SESSION_RESUME`

Why third:
- snapshot building and CLI/MCP continuity need durable session records
- event limits must exist before broad hook coverage creates noisy or oversized history

Exit criteria:
- [x] session records persist across runs
- [x] event retention and dedup rules are enforced centrally

### Phase CM4 — Event extraction and adapter pipeline

Instrument existing commands and engines only after the session/content services exist.

#### Internal session event capture points

- [x] CLI command start
- [x] CLI command finish
- [x] session start / adapter startup
- [x] before compaction
- [x] `atlas build`
- [x] `atlas update`
- [x] `atlas review-context`
- [x] `atlas impact`
- [x] context engine request handling
- [x] reasoning engine request handling
- [x] reasoning engine response must emit session events
- [x] MCP tool handler execution boundaries

#### Extraction API

- [x] `extract_cli_event`
- [x] `extract_graph_event`
- [x] `extract_context_event`
- [x] `extract_reasoning_event`
- [x] `extract_user_event`
- [x] `extract_tool_event`
- [x] `normalize_event`
- [x] `hash_event`

#### Event payload rules

- [x] payloads must be structured JSON
- [x] payloads must be bounded in size
- [x] payloads must include identifiers for retrieval when large artifacts exist
- [x] reasoning results must reference `source_id` for saved artifacts
- [x] payloads must never embed large stdout blobs
- [x] continuity write failures must degrade to log-and-continue behavior

#### Session bridge artifacts

- [x] write transient session event markdown bridge file when direct hook payload transport is unavailable
- [x] auto-index session bridge markdown into content store
- [x] clean up consumed or stale bridge files

#### External hooks adapter interfaces

- [x] `BeforeCommand`
- [x] `AfterCommand`
- [x] `OnError`
- [x] `OnUserIntent`
- [x] `OnSessionStart`
- [x] `BeforeCompact`
- [x] `BeforeExit`

#### Initial adapters

- [x] CLI adapter
- [x] MCP adapter

#### Adapter rules

- [x] adapters must emit normalized events
- [x] adapters must not write SQLite directly
- [x] adapters must use session service layer
- [x] adapters may degrade gracefully when host lacks a native session-start hook
- [x] host-specific hook gaps must reduce continuity features, not break command execution

Why fourth:
- hooks before storage would force rewrites or duplicated logic
- adapters keep CLI and MCP instrumentation transport-specific but persistence-agnostic

Exit criteria:
- [x] core command flows emit normalized bounded events
- [x] no direct SQLite writes occur from CLI or MCP adapters
- [x] continuity remains non-blocking even when hook capture or persistence partially fails

### Phase CM5 — Resume snapshots and CLI session workflow

Once events exist, build bounded resume material and user-facing session commands.

#### Snapshot API

- [x] `build_resume(session_id) -> ResumeSnapshot`

#### Snapshot content

- [x] repo root
- [x] worktree identifier
- [x] last user intent
- [x] most recent important commands
- [x] changed files
- [x] impacted symbols
- [x] unresolved errors
- [x] recent reasoning outputs
- [x] saved artifact references
- [x] current task state
- [x] recent decisions
- [x] active rules/instructions
- [x] retrieval-ready source labels or queries for important prior artifacts

#### Snapshot constraints

- [x] snapshot size must be bounded
- [x] snapshot must contain retrieval hints
- [x] snapshot must prefer identifiers and summaries over raw content
- [x] snapshot must be stable enough for tests
- [x] snapshot must group events by category
- [x] snapshot must include exact follow-up search commands / retrieval directives
- [x] snapshot rendering must be deterministic and easy to snapshot-test

#### Lifecycle

- [x] build snapshot before compaction or reset
- [x] persist snapshot into `session_resume`
- [x] inject snapshot at next session start or explicit resume
- [x] mark snapshot consumed after successful injection

#### CLI commands

- [x] `atlas session start`
- [x] `atlas session status`
- [x] `atlas session resume`
- [x] `atlas session clear`
- [x] `atlas session list`

#### CLI behavior

- [x] auto-create session on interactive run
- [x] auto-load resume snapshot when available
- [x] show compact resume summary
- [x] add session lifecycle support
- [x] never replay raw historic output
- [x] degrade gracefully on hosts or shells without full lifecycle hooks

Why fifth:
- snapshots are only useful once event history and artifact references exist
- CLI session commands should operate on the same bounded data model that resume uses

Exit criteria:
- [x] session resume works from stored snapshot, not raw history replay
- [x] CLI surfaces expose session lifecycle without leaking internal storage details
- [x] resume snapshot gives enough retrieval instructions to recover prior tool results, topics, and decisions on demand

### Phase CM6 — Retrieval-backed restoration in Context Engine

Extend context retrieval only after saved artifacts and session identity are stable.

#### Context Engine request additions

- [x] add `include_saved_context: bool`
- [x] add `session_id: Option<String>`

#### Retrieval flow

- [x] query content store by symbol name after graph retrieval
- [x] query content store by file path after graph retrieval
- [x] query content store by session ID after graph retrieval
- [x] add retrieval from content store after graph retrieval
- [x] merge saved-context results into `ContextResult`

#### Ranking additions

- [x] add saved-context relevance
- [x] add recency boost
- [x] add same-session boost
- [x] add session-aware ranking
- [x] preserve lexical retrieval as primary ranking path
- [x] avoid vector/embedding dependency in v1 continuity path

#### Result additions

- [x] include `saved_context_sources`
- [x] include `source_ids`
- [x] include `retrieval_hints`
- [x] include saved-context previews without dumping raw blobs
- [x] include enough metadata to reopen prior tool result, topic, message, or query by retrieval

Why sixth:
- context integration depends on both content search and session identity
- ranking should only absorb saved context once the retrieval inputs are trustworthy

Exit criteria:
- [x] context restoration works through retrieval, not transcript replay
- [x] `ContextResult` exposes enough source ids and hints for follow-up fetches

### Phase CM7 — MCP continuity and saved-context tools

Expose session continuity to agents only after storage, events, resume, and retrieval paths are working locally. Detailed tool, event, and payload checklist is consolidated in MCP5 under MCP and Agent Roadmap.

Why seventh:
- MCP should stay thin over already-proven services
- agent-facing session tools are risky to expose before local lifecycle behavior is stable

Exit criteria:
- [x] MCP returns pointers and previews instead of raw large payloads
- [x] session-aware MCP tools work without duplicating business logic from CLI/services

### Phase CM8 — Safety limits, tests, and completion gate

Close with the operational guards and tests that keep context-mode safe and maintainable.

#### Tests

- [x] session creation
- [x] event deduplication
- [x] event eviction
- [x] resume snapshot correctness
- [x] snapshot consume flow
- [x] artifact indexing and retrieval
- [x] compression routing
- [x] search relevance: BM25, trigram fallback, fuzzy correction, RRF ordering, proximity/title rerank
- [x] session extraction from representative hook payloads
- [x] CLI continuity
- [x] MCP continuity
- [x] bridge markdown ingest and cleanup
- [x] corrupt DB recovery / quarantine
- [x] best-effort continuity failure path
- [x] race/concurrency coverage for session writes and snapshot updates

#### Redaction

- [x] strip environment variables
- [x] strip secrets from command arguments
- [x] strip tokens from logs and payloads
- [x] avoid indexing sensitive bridge payloads or raw secrets into content store

#### Limits

- [x] max events per session
- [x] max content DB size
- [x] retention TTL
- [x] snapshot size cap
- [x] stale source cleanup
- [x] stale session cleanup
- [x] dedup time window for repeated near-identical events

#### Operational visibility

- [x] add session stats
- [x] add content-store stats
- [x] add avoided-context byte counters
- [x] add indexed artifact / preview / pointer routing counters
- [x] add purge visibility for session DB, content DB, and bridge artifacts

#### Completion criteria

- [x] sessions persist across runs
- [x] large outputs are stored instead of passed directly
- [x] context is restored through retrieval
- [x] resume snapshot works correctly
- [x] MCP returns pointers instead of blobs
- [x] graph DB, content DB, and session DB remain separate systems
- [x] retrieval quality is good enough to recover prior topics, tool results, messages, and queries without transcript replay
- [x] continuity failures stay best-effort and do not block primary Atlas commands

Why last:
- limits and redaction must validate the final integrated system, not just one crate in isolation
- completion gate should assert end-to-end continuity behavior across CLI, retrieval, and MCP

---

### Phase CM9 — Semantic Retrieval

#### Goal

Move beyond lexical search (BM25, trigram) into meaning-aware retrieval.

#### Tasks

- [x] add symbol-aware retrieval using graph relationships
- [x] expand queries using related symbols from graph
- [x] cluster related artifacts by concept
- [x] implement cross-file semantic linking
- [x] add query expansion based on prior context

#### Output

- retrieval can find conceptually related data, not just keyword matches

#### CLI and MCP rollout follow-up

- [x] add `atlas query --semantic` routing to semantic retrieval
- [x] route `atlas context` through prior-context semantic expansion when session context is available
- [x] add semantic mode or flag to `query_graph` MCP tool
- [x] expose symbol neighborhood, cross-file links, and concept clustering through CLI or MCP surface

---

### Phase CM10 — Memory Curation

#### Goal

Reduce noise and improve signal quality in stored memory.

#### Tasks

- [ ] implement event compaction
- [ ] merge duplicate or similar events
- [ ] detect repeated actions and summarize
- [ ] decay low-value events over time
- [ ] promote high-value events to persistent memory
- [ ] deduplicate reasoning outputs

#### Output

- cleaner, more meaningful session memory
- reduced redundancy

#### CLI and MCP rollout follow-up

- [ ] surface curation and compaction stats in `atlas session status` and `get_session_status`
- [ ] apply curation before resume snapshot build and before saved-context retrieval results are returned
- [ ] add manual compaction/curation trigger through CLI or MCP if automatic lifecycle hooks are insufficient

---

### Phase CM11 — Cross-Session Intelligence

#### Goal

Enable memory across multiple sessions.

#### Tasks

- [ ] implement cross-session search
- [ ] create global memory layer
- [ ] track frequently accessed symbols/files
- [ ] detect recurring workflows
- [ ] surface relevant past sessions

#### Output

- system recalls past work across sessions

#### CLI and MCP rollout follow-up

- [ ] add cross-session mode or flag to saved-context search surfaces
- [ ] route `atlas context` and MCP context/query tools through global memory lookup when cross-session recall is enabled
- [ ] expose frequently accessed symbols/files and recurring workflows in session or context status surfaces

---

### Phase CM12 — Predictive Context

#### Goal

Make context proactive instead of reactive.

#### Tasks

- [ ] predict next likely user action
- [ ] prefetch relevant artifacts
- [ ] preload context based on recent activity
- [ ] cache frequently accessed context

#### Output

- faster, smarter responses
- reduced latency for common workflows

#### CLI and MCP rollout follow-up

- [ ] wire predictive prefetch into `atlas context`, `query_graph`, and resume flows rather than leaving it as background-only logic
- [ ] expose debug or metadata fields showing what was prefetched and why in CLI JSON and MCP responses
- [ ] ensure predictive caches respect existing session and saved-context boundaries

---

### Phase CM13 — Context Budget Optimization

#### Goal

Select the best possible context within limits.

#### Tasks

- [ ] implement dynamic token budgeting
- [ ] rank sources:
  - graph context
  - saved artifacts
  - resume snapshot
- [ ] select optimal mix of context sources
- [ ] enforce strict token limits

#### Output

- optimal context selection instead of naive inclusion

#### CLI and MCP rollout follow-up

- [ ] apply token budgeting to `atlas context`, `get_review_context`, `get_context`, and related MCP responses
- [ ] expose budget decisions, dropped-source counts, and selected-source mix in structured output
- [ ] allow CLI and MCP callers to override or inspect budget caps without bypassing the optimizer

---

### Phase CM14 — Decision Memory

#### Goal

Persist and reuse decisions.

#### Tasks

- [ ] create decision event types
- [ ] link decisions to artifacts
- [ ] store reasoning behind decisions
- [ ] retrieve decisions for future tasks
- [ ] avoid recomputing prior conclusions

#### Output

- system remembers why decisions were made

#### CLI and MCP rollout follow-up

- [ ] emit decision events from CLI, context, reasoning, and MCP adapter flows
- [ ] route `atlas context` and saved-context retrieval through decision lookup when relevant prior conclusions exist
- [ ] expose decision retrieval through CLI or MCP surface with linked evidence and artifact references

---

### Phase CM15 — Agent-Aware Context (Optional)

#### Goal

Support multi-agent workflows.

#### Tasks

- [ ] implement per-agent memory partitions
- [ ] track delegated tasks
- [ ] merge outputs across agents
- [ ] track agent responsibilities

#### Output

- scalable multi-agent memory system

#### CLI and MCP rollout follow-up

- [ ] add agent partition identifiers to session, context, and saved-context APIs
- [ ] extend MCP tools to read/write per-agent memory partitions and merged views intentionally
- [ ] expose delegated-task and responsibility summaries through CLI or MCP status/context surfaces

---

#### Completion Criteria

- [ ] memory is curated, not just stored
- [ ] retrieval is semantic-aware
- [ ] system can recall past sessions
- [ ] context selection is optimized
- [ ] decisions persist and are reused
- [ ] system improves over time

---

## Part V — Follow-Up Patches

Use these patch sections for focused improvements that cut across existing roadmap phases without rewriting phase scope.

## Retrieval Follow-Up Patch

These are the high-value retrieval/indexing improvements still missing or only partially specified after the current v3 plan.

They are meant to strengthen Atlas’s retrieval/content sidecar without changing the graph-first core.

### Patch R1 — Retrieval index lifecycle state

Atlas already has strong graph build/update state and separate content/session stores, but retrieval/content indexing should also have an explicit lifecycle model so “built”, “indexed”, “searchable”, and “failed” do not drift.

- [x] add explicit retrieval index state model
- [x] create retrieval/content index status table or snapshot
- [x] track per repo / per source states:
  - [x] `indexing`
  - [x] `indexed`
  - [x] `index_failed`
- [x] persist:
  - [x] total files discovered
  - [x] files indexed
  - [x] total chunks written
  - [x] chunks reused
  - [x] last successful index time
  - [x] last error
- [x] expose retrieval index status through CLI
- [x] expose retrieval index status through MCP
- [x] ensure one source of truth for “searchable now”
- [x] ensure interrupted indexing can recover cleanly without manual cleanup

Why:
- prevents state drift between stored content, searchable content, and agent-visible status
- improves crash recovery and diagnostics

### Patch R2 — Retrieval batching and chunk explosion guardrails

Current plan has chunking and retrieval, but operational safety limits should be explicit.

- [ ] add configurable `retrieval_batch_size`
- [ ] add configurable `embedding_batch_size`
- [ ] add hard `max_chunks_per_index_run`
- [ ] add hard `max_chunks_per_file`
- [ ] add policy for oversized indexing runs:
  - [ ] fail fast
  - [ ] partial index with warning
  - [ ] skip pathological file with error entry
- [ ] measure and log:
  - [ ] buffered chunk count
  - [ ] buffered bytes
  - [ ] staged vector bytes
  - [ ] batch flush count
- [ ] add tests for:
  - [ ] chunk explosion from large file
  - [ ] recursive fallback chunk explosion
  - [ ] partial indexing recovery after hard cap hit

Why:
- protects retrieval layer from pathological files and runaway indexing cost
- makes retrieval/index behavior predictable under load

### Patch R3 — Embedding dimension registry and freeze rules

Atlas already has optional embeddings and hybrid retrieval roadmap, but dimension handling should be explicit and deterministic.

- [ ] create embedding provider registry metadata
- [ ] persist:
  - [ ] provider name
  - [ ] model name
  - [ ] embedding dimension
  - [ ] discovered_at
  - [ ] index schema version
- [ ] require dimension to be frozen at index creation time
- [ ] reject insert/search if dimension does not match active retrieval index
- [ ] cache discovered dimensions per provider/model
- [ ] add CLI / diagnostics surface for current embedding config
- [ ] add tests for:
  - [ ] dimension mismatch on insert
  - [ ] dimension mismatch on query
  - [ ] provider switch with incompatible existing index
  - [ ] explicit rebuild requirement after dimension change

Why:
- avoids one of the most common hybrid/vector indexing failure modes
- keeps retrieval layer deterministic and debuggable

### Patch R4 — Retrieval backend capability flags

Atlas should make backend capability checks explicit instead of assuming all retrieval backends support all modes.

- [ ] define retrieval backend capability model
- [ ] support capability flags for:
  - [ ] lexical FTS
  - [ ] dense vector search
  - [ ] hybrid lexical + vector fusion
  - [ ] sparse / BM25-native retrieval
  - [ ] metadata filtering
- [ ] validate requested retrieval mode against backend capabilities before query/index
- [ ] disable unsupported hybrid mode automatically with explicit warning
- [ ] ensure MCP/CLI surfaces report active retrieval mode clearly
- [ ] add tests for:
  - [ ] lexical-only backend
  - [ ] dense-only backend
  - [ ] hybrid-capable backend
  - [ ] unsupported mode request fails cleanly

Why:
- makes future retrieval backends or storage variants safe to introduce
- avoids silent degradation and confusing behavior

### Patch R5 — Stable content-derived chunk identity

Current chunk storage should have a true stable identity separate from display order.

- [x] add stable `chunk_id`
- [x] define `chunk_id` from content-derived hash over:
  - [x] source/file path
  - [x] line span or chunk boundary
  - [x] normalized content
- [x] keep `chunk_index` or display order separately
- [x] use `chunk_id` for:
  - [x] dedupe
  - [x] chunk reuse
  - [ ] retrieval cache keys
  - [ ] saved-context references
- [x] add tests for:
  - [x] same content same `chunk_id`
  - [x] moved chunk with changed path policy documented
  - [x] changed line span/content produces new `chunk_id`

Why:
- improves deduplication and retrieval consistency across rebuilds
- helps saved-context and future historical retrieval features

### Patch R6 — Retrieval/token-efficiency evaluation

Atlas already measures correctness and performance in many places, but retrieval should also be evaluated as a context-efficiency system.

- [ ] add retrieval benchmark metrics:
  - [ ] `recall_at_k`
  - [ ] `mrr`
  - [ ] exact target hit rate
  - [ ] retrieved tokens per query
  - [ ] emitted tokens per query
  - [ ] tool calls per task
- [ ] benchmark:
  - [ ] graph-only context
  - [ ] lexical retrieval only
  - [ ] hybrid retrieval
  - [ ] hybrid retrieval + graph expansion
- [ ] add fixed-budget evaluation:
  - [ ] quality under small context budget
  - [ ] quality under medium context budget
- [ ] track whether retrieval actually reduces:
  - [ ] payload size
  - [ ] repeated search calls
  - [ ] context noise
- [ ] add acceptance thresholds before enabling hybrid retrieval by default

Why:
- keeps retrieval improvements aligned with actual user value
- validates that the retrieval layer improves token efficiency, not just ranking complexity

### Patch R7 — Later experimental post-retrieval compaction

This is not core and should stay late, but it is a useful optional experiment once retrieval and context engine behavior are stable.

- [ ] add backlog item for post-retrieval compaction experiment
- [ ] only evaluate after:
  - [ ] hybrid retrieval is stable
  - [ ] context engine output quality is stable
  - [ ] token-efficiency metrics exist
- [ ] keep initial experiment strictly optional
- [ ] require evidence that compaction reduces tokens without harming answer quality
- [ ] do not let this replace retrieval filtering or graph-based selection

Why:
- useful possible optimization later
- should not destabilize current graph-first + retrieval-assisted architecture

### Patch completion criteria

This patch is complete when:

- [ ] retrieval/content index has explicit searchable state
- [ ] retrieval indexing has batch and chunk guardrails
- [ ] embedding dimension rules are explicit and enforced
- [ ] retrieval backend capabilities are validated, not assumed
- [ ] stable `chunk_id` exists and is used for dedupe/reuse
- [ ] retrieval/token-efficiency benchmarks are in place
- [ ] optional post-retrieval compaction is tracked as a late experiment only

---

## Graph Build Lifecycle Patch

Atlas has retrieval index lifecycle state in the content store (Patch R1), but the graph store (`worldtree.db`) has no equivalent. Schema version alone is not enough — a `building` or `build_failed` state cannot be inferred from `metadata.schema_version`.

### Patch G1 — Graph build lifecycle state

- [x] add explicit graph build state model to `atlas-store-sqlite`
- [x] create migration `006_graph_build_state.sql` in `packages/atlas-store-sqlite/src/migrations/`
- [x] create `graph_build_state` table with columns:
  - [x] `repo_root TEXT PRIMARY KEY`
  - [x] `state TEXT NOT NULL` — `building`, `built`, `build_failed`
  - [x] `files_discovered INTEGER NOT NULL DEFAULT 0`
  - [x] `files_processed INTEGER NOT NULL DEFAULT 0`
  - [x] `files_failed INTEGER NOT NULL DEFAULT 0`
  - [x] `nodes_written INTEGER NOT NULL DEFAULT 0`
  - [x] `edges_written INTEGER NOT NULL DEFAULT 0`
  - [x] `last_built_at TEXT`
  - [x] `last_error TEXT`
  - [x] `updated_at TEXT NOT NULL`
- [x] define `GraphBuildState` enum: `Building`, `Built`, `BuildFailed`
- [x] define `GraphBuildStatus` struct with all counter and timestamp fields
- [x] add `Store` methods:
  - [x] `begin_build(repo_root)`
  - [x] `finish_build(repo_root, stats)`
  - [x] `fail_build(repo_root, error)`
  - [x] `get_build_status(repo_root) -> Option<GraphBuildStatus>`
  - [x] `list_build_statuses() -> Vec<GraphBuildStatus>`
- [x] wire `begin_build` and `finish_build` / `fail_build` into `atlas build` command path
- [x] wire `begin_build` and `finish_build` / `fail_build` into `atlas update` command path
- [x] expose build status in `atlas status` output
- [x] expose build status in `atlas doctor` check (flag `state=building` as interrupted, `build_failed` as error)
- [x] include build status in `build_or_update_graph` MCP tool response
- [x] export `GraphBuildState`, `GraphBuildStatus` from `atlas-store-sqlite` crate
- [x] add tests:
  - [x] `begin_build` sets state to `building`
  - [x] `finish_build` after `begin_build` sets state to `built` with counters
  - [x] `fail_build` after `begin_build` sets state to `build_failed` with error
  - [x] `get_build_status` returns `None` when no row exists
  - [x] `list_build_statuses` returns all repos
  - [x] interrupted build (state stays `building`) detected by doctor
  - [x] counters accumulate correctly across update runs

Why:
- graph store has no way to distinguish "never built", "currently building", "build done", "build crashed"
- `atlas status` and `atlas doctor` cannot correctly report graph freshness without explicit state
- mirrors Patch R1 pattern for consistency across all three stores

### Patch G completion criteria

- [x] graph store has explicit build state separate from schema version
- [x] `atlas build` and `atlas update` record lifecycle transitions
- [x] `atlas doctor` flags interrupted or failed builds
- [x] `atlas status` reports graph build state alongside file/node counts
- [x] MCP `build_or_update_graph` returns persisted build state
- [x] tests cover all state transitions

---
