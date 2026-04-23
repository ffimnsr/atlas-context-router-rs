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
- Part V. Focused follow-up patches: Retrieval Follow-Up Patch, Retrieval Ranking Evidence Patch, Graph/Content Companion Patch, Parity Surface Patch, Runtime Event Enrichment and Graph Linking Patch, Ranking and Trimming Primitives Patch, Graph Build Lifecycle Patch, Canonical Path Identity Patch, Graph Readiness Source-of-Truth Patch, Operational Budget Policy Patch, Context Escalation Contract Patch, Graph Store Corruption Recovery Patch, Repo-Scoped MCP Singleton Patch

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
  - [x] auto-watch mode
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
- [x] Optional recursive submodule handling later
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
- [x] lock/retry behavior — covered by `write_succeeds_while_second_connection_holds_wal_write_lock`: blocker thread holds `BEGIN IMMEDIATE` for 100 ms; store write succeeds within `busy_timeout=5000`

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
- [x] benchmark parser workers vs writer bottleneck — covered by `pipeline_bench` (parse_only / write_only / full_pipeline groups)
- [x] tune batch sizes — `pipeline_bench` sweeps batch sizes 16/32/64/128/256; `DEFAULT_PARSE_BATCH_SIZE=64` confirmed reasonable for this repo

### 15.2 Query performance

- [x] benchmark FTS query latency — covered by `store_bench`
- [x] benchmark impact-radius latency — covered by `store_bench`
- [x] benchmark review-context latency — covered by `context_bench` in `atlas-review` (64 and 256 module variants)

### 15.3 Memory and reliability

- [x] cap parse queue size — build pipeline uses bounded chunk-based batches; no unbounded in-memory accumulation
- [x] avoid loading giant repos into memory — chunked parallel parse; per-file size cap in collector
- [x] add partial-failure reporting — `parse_errors` counter surfaces failures in build/update summary
- [x] add crash-safe file replacement semantics — each file graph replaced in an atomic `BEGIN IMMEDIATE` transaction

### 15.4 Diagnostics

- [x] `atlas doctor` — implemented: checks repo root, git root, .atlas dir, config, DB file, integrity, graph stats, git ls-files
- [x] `atlas db check` — implemented
- [x] tracing spans around build/update phases

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
- [x] document metadata contract in MCP reference and agent instructions
- [x] make mismatched repo/db/index state obvious in agent sessions

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
  - [x] schema mismatch
  - [x] interrupted build
  - [x] failed build
  - [x] stale index
  - [x] retrieval/content index unavailable
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
- [x] expose opt-in richer snippets without bloating default responses
- [x] document selection rules for `query_graph` vs file/content/template/text-asset search
- [x] add tests for Markdown, prompt files, SQL, config, templates, embedded strings, ignored paths, and generated-file suppression

### MCP10 — Query graph option parity

Expose CLI query options in MCP `query_graph` so agents can use the same ranking and scope controls as CLI users.

- [x] add `subpath` argument
- [x] add `fuzzy` argument
- [x] add `hybrid` argument
- [x] improve fuzzy symbol typo recovery:
  - [x] prefer close symbol-name edit distance over weaker Markdown/docs/content token matches
  - [x] regression: `LoadIdentityMesages` should suggest/rank `LoadIdentityMessages` above Markdown nodes
- [x] expose query explanation in MCP `explain_query`:
  - [x] include ranking factors, filters, FTS terms, fuzzy corrections, regex mode, and active query mode
- [x] clarify `regex` mode behavior in schema docs:
  - [x] regex-only structural scan
  - [x] text + regex post-filter over FTS candidates
  - [x] invalid regex error shape
- [x] evaluate and add `include_files` argument if file nodes improve agent workflows
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

- [x] add MCP `read_saved_context` tool
- [x] accept `source_id`
- [x] support optional paging or byte/token caps for large artifacts
- [x] return full content when within configured limits
- [x] return truncation metadata and continuation hints when content exceeds limits
- [x] preserve existing preview behavior in `search_saved_context`
- [x] enforce session/repo scoping so one session cannot read unrelated saved artifacts accidentally
- [x] include artifact metadata:
  - [x] `source_id`
  - [x] artifact kind
  - [x] created time
  - [x] session id
  - [x] byte count
  - [x] chunk count
- [x] add tests for found artifact, missing artifact, oversized artifact, paged artifact, and cross-session/repo isolation

### MCP14 — Agent hook integrations

Add first-class hook templates and adapter docs for Copilot, Claude, and Codex so Atlas session continuity, graph freshness, review context, and command audit signals work across agent hosts.

#### Shared Atlas hook behavior

- [x] add repo-local hook scripts under `.atlas/hooks/` or generated host-specific locations that call Atlas CLI commands, never write SQLite directly
- [x] extend existing `atlas install --platform <platform>` flow to install platform hooks in addition to MCP config
- [x] keep supported platform values exactly:
  - [x] `copilot`
  - [x] `claude`
  - [x] `codex`
  - [x] `all`
- [x] use existing `atlas install --platform <platform> --dry-run` to print files and hook events without writing
- [x] add hook validation to `atlas install --platform <platform>` output, or add a narrow `atlas install --platform <platform> --validate-only` flag if validation needs no writes
- [x] keep hook failures non-blocking unless hook purpose is explicit policy enforcement
- [x] emit normalized session events through existing session service:
  - [x] user prompt / intent
  - [x] session start / resume
  - [x] tool preflight
  - [x] tool result
  - [x] permission decision
  - [x] compaction boundary
  - [x] session stop / end
  - [x] error / failure
- [x] save compaction snapshots before host context compaction when host exposes compaction hooks
- [x] log denied or risky shell/file operations without storing secret-bearing arguments

#### Hook storage and context routing

- [x] all hooks write a small normalized event through session service first
- [x] session event stores `source_id` when large payload is saved to content store
- [x] session-only hooks:
  - [x] `SessionStart` / `sessionStart`
  - [x] `PreToolUse` / `preToolUse`
  - [x] `PermissionRequest`
  - [x] `PermissionDenied`
  - [x] `PostCompact`
  - [x] `ConfigChange`
  - [x] `CwdChanged`
  - [x] `FileChanged`
  - [x] `WorktreeCreate`
  - [x] `WorktreeRemove`
  - [x] `Notification`
  - [x] `SubagentStart`
  - [x] `SubagentStop`
  - [x] `TaskCreated`
  - [x] `TaskCompleted`
- [x] session plus content-store hooks when payload exceeds event size cap or should be retrievable later:
  - [x] `UserPromptSubmit` / `userPromptSubmitted`
  - [x] `PostToolUse` / `postToolUse`
  - [x] `PostToolUseFailure`
  - [x] `Stop`
  - [x] `StopFailure`
  - [x] `SessionEnd` / `sessionEnd`
  - [x] `errorOccurred` / `error`
  - [x] `Elicitation`
  - [x] `ElicitationResult`
  - [x] `InstructionsLoaded`
- [x] context-engine hooks:
  - [x] `SessionStart` loads resume and context hints
  - [x] `UserPromptSubmit` classifies intent and may retrieve saved context
  - [x] `PreCompact` builds resume snapshot from session events and content-store artifacts
  - [x] `PostCompact` verifies restore state
  - [x] `Stop` / `SessionEnd` persists handoff and resume hints
- [x] graph/context refresh hooks:
  - [x] `PostToolUse` runs graph update after file edits
  - [x] `PostToolUse` refreshes review/impact context after successful tests or builds when bounded
  - [x] `FileChanged` marks graph/content freshness stale without storing full file content

#### Hook install files and directories

- [x] install one shared Atlas hook runner in repo-local directory:
  - [x] `.atlas/hooks/atlas-hook`
  - [x] `.atlas/hooks/lib/`
- [x] platform hook configs must contain all supported events in one platform config file where host schema allows it
- [x] platform hook configs must call shared runner with concrete event argument:
  - [x] `.atlas/hooks/atlas-hook session-start`
  - [x] `.atlas/hooks/atlas-hook user-prompt`
  - [x] `.atlas/hooks/atlas-hook pre-tool-use`
  - [x] `.atlas/hooks/atlas-hook permission-request`
  - [x] `.atlas/hooks/atlas-hook post-tool-use`
  - [x] `.atlas/hooks/atlas-hook tool-failure`
  - [x] `.atlas/hooks/atlas-hook pre-compact`
  - [x] `.atlas/hooks/atlas-hook post-compact`
  - [x] `.atlas/hooks/atlas-hook stop`
  - [x] `.atlas/hooks/atlas-hook session-end`
  - [x] `.atlas/hooks/atlas-hook error`
- [x] install hook output, bridge, and transient state under `.atlas/sessions/` or `.atlas/tmp/`, not host config directories
- [x] install Copilot workspace hooks under:
  - [x] `.github/hooks/atlas-copilot.json`
  - [x] optional custom location from `.vscode/settings.json` via `chat.hookFilesLocations`
  - [x] do not write user-level `~/.copilot/hooks` unless `--scope user` is explicit
- [x] use same `.github/hooks/atlas-copilot.json` file for VS Code Copilot, GitHub Copilot cloud agent, and Copilot CLI where hook schema allows it
- [x] require `.github/hooks/atlas-copilot.json` on default branch for GitHub Copilot cloud agent use
- [x] use same file from current working directory for Copilot CLI
- [x] install Claude hooks under:
  - [x] `.claude/settings.json` for repo-shared hooks
  - [x] `.claude/settings.local.json` for machine-local overrides
  - [x] Claude hook entries call `.atlas/hooks/atlas-hook <event>`
  - [x] do not write `~/.claude/settings.json` unless `--scope user` is explicit
- [x] install Codex hooks under:
  - [x] `.codex/hooks.json` for repo-local hook config if supported by active Codex config
  - [x] Codex hook entries call `.atlas/hooks/atlas-hook <event>`
  - [x] update `.codex/config.toml` only when needed to point Codex at repo-local hook config
  - [x] do not write user-level Codex config unless `--scope user` is explicit
- [x] install wiki/reference files under:
  - [x] `wiki/hooks-copilot.md`
  - [x] `wiki/hooks-claude.md`
  - [x] `wiki/hooks-codex.md`
  - [x] update `wiki/_Sidebar.md`
- [x] install test fixtures under:
  - [x] `packages/atlas-cli/tests/fixtures/hooks/copilot/`
  - [x] `packages/atlas-cli/tests/fixtures/hooks/claude/`
  - [x] `packages/atlas-cli/tests/fixtures/hooks/codex/`

#### Required hooks by platform

- [x] Copilot must use these VS Code hook names where running in VS Code:
  - [x] `SessionStart` for Atlas session start/resume and graph health check
  - [x] `UserPromptSubmit` for user-intent capture and optional bounded context injection
  - [x] `PreToolUse` for command/file policy checks before tool execution
  - [x] `PostToolUse` for graph update, command-result capture, and review/impact refresh
  - [x] `PreCompact` for resume snapshot creation before context truncation
  - [x] `SubagentStart` and `SubagentStop` for nested-agent boundaries
  - [x] `Stop` for final turn state and resume hints
- [x] Copilot must use these GitHub cloud agent / Copilot CLI hook names where running in GitHub or CLI:
  - [x] `sessionStart` for Atlas session start/resume and graph health check
  - [x] `userPromptSubmitted` for user-intent capture
  - [x] `preToolUse` for `permissionDecision` allow/deny policy
  - [x] `postToolUse` for graph update, command-result capture, and review/impact refresh
  - [x] `sessionEnd` for final resume snapshot and transient cleanup
  - [x] `errorOccurred` for bounded error capture
- [x] Claude must use these hooks:
  - [x] `SessionStart` for Atlas session start/resume
  - [x] `UserPromptSubmit` for user-intent capture
  - [x] `UserPromptExpansion` for expanded prompt validation
  - [x] `PreToolUse` with `Bash|Edit|Write|MultiEdit` matcher for preflight policy
  - [x] `PermissionRequest` for narrow Atlas maintenance auto-allow decisions
  - [x] `PermissionDenied` for denial audit and optional retry guidance
  - [x] `PostToolUse` with `Edit|Write|MultiEdit|Bash` matcher for graph update and result capture
  - [x] `PostToolUseFailure` for failed tool summaries
  - [x] `Notification` for stale-graph or pending-resume notices when enabled
  - [x] `SubagentStart` and `SubagentStop` for delegated-work boundaries
  - [x] `TaskCreated` and `TaskCompleted` for task lifecycle events
  - [x] `Stop` and `StopFailure` for final turn or API-error state
  - [x] `InstructionsLoaded` for loaded instruction/rule metadata
  - [x] `ConfigChange`, `CwdChanged`, and `FileChanged` for config/root/freshness refresh
  - [x] `WorktreeCreate` and `WorktreeRemove` for temporary worktree identity
  - [x] `PreCompact` and `PostCompact` for resume snapshot before/after compaction
  - [x] `Elicitation` and `ElicitationResult` for MCP user-input flow capture
  - [x] `SessionEnd` for final snapshot and cleanup
- [x] Codex must use these hooks:
  - [x] `SessionStart` with `startup|resume` matcher for Atlas session start/resume and health check
  - [x] `UserPromptSubmit` for user-intent capture
  - [x] `PreToolUse` with `Bash` matcher for command policy before execution
  - [x] `PermissionRequest` with `Bash` matcher for narrow Atlas maintenance auto-allow decisions
  - [x] `PostToolUse` with `Bash` matcher for command-result capture and graph refresh
  - [x] `Stop` for final turn state and resume hints

#### Copilot hooks

Use `.github/hooks/atlas-copilot.json` for Copilot hooks. VS Code uses PascalCase event names; GitHub Copilot cloud agent and Copilot CLI use camelCase event names.

- [x] generate VS Code-compatible hook config for `SessionStart`
  - [x] call `atlas session start` or resume logic
  - [x] call `atlas status --json`
  - [x] record repo root, cwd, model, and session id when present
- [x] generate `UserPromptSubmit`
  - [x] call Atlas user-intent capture
  - [x] optionally inject compact repo/session guidance when host accepts hook output context
- [x] generate `PreToolUse`
  - [x] block or ask for dangerous shell/file operations only when configured
  - [x] detect operations that need graph freshness after completion
- [x] generate `PostToolUse`
  - [x] run `atlas update` after edit/write tools
  - [x] run targeted context refresh after successful tests, builds, or file changes
  - [x] record bounded stdout/stderr summaries, not raw large output
- [x] generate `PreCompact`
  - [x] call `atlas session snapshot` / resume material writer before context truncation
- [x] generate `SubagentStart` and `SubagentStop`
  - [x] track nested agent boundaries
  - [x] merge subagent summaries into parent session events
- [x] generate `Stop`
  - [x] record final turn state
  - [x] persist resume hints and unresolved errors
- [x] document VS Code settings touched:
  - [x] `chat.hookFilesLocations`
  - [x] `chat.useCustomAgentHooks` when agent-scoped hooks are emitted

#### Copilot cloud agent / CLI hooks

Use `.github/hooks/atlas-copilot.json` with `version: 1`; remember cloud agent requires hook config on default branch, while CLI loads hooks from current working directory.

- [x] generate `sessionStart`
  - [x] run Atlas startup health check
  - [x] capture `source` values such as `new`, `resume`, or `startup`
  - [x] write concise session-start event
- [x] generate `userPromptSubmitted`
  - [x] capture prompt metadata for continuity
  - [x] never persist raw prompt when configured redaction policy rejects it
- [x] generate `preToolUse`
  - [x] enforce deny/allow policy through `permissionDecision` and `permissionDecisionReason`
  - [x] guard dangerous bash, destructive file writes, secret reads, and broad generated-file edits
- [x] generate `postToolUse`
  - [x] capture `toolName`, parsed `toolArgs`, and `toolResult.resultType`
  - [x] trigger `atlas update` after edit/write-like tools
  - [x] trigger review/impact refresh after successful build/test commands
- [x] generate `sessionEnd`
  - [x] persist final resume snapshot and cleanup transient bridge files
  - [x] record end reason such as `complete`, `error`, `abort`, `timeout`, or `user_exit`
- [x] generate `errorOccurred`
  - [x] capture bounded error message/name/stack metadata
  - [x] save error event for resume and review triage
- [x] support Bash and PowerShell command fields where host allows both
- [x] keep default timeout at host default unless Atlas command needs explicit `timeoutSec`

#### Claude hooks

Use `.claude/settings.json` and `.claude/settings.local.json`; keep matchers narrow and prefer command hooks.

- [x] generate `SessionStart`
  - [x] initialize or resume Atlas session
  - [x] reload saved context hints
- [x] generate `UserPromptSubmit`
  - [x] capture user intent
  - [x] add bounded additional context only when needed
- [x] generate `UserPromptExpansion`
  - [x] validate expanded commands before model receives them
  - [x] block unsafe expansion when policy requires it
- [x] generate `PreToolUse`
  - [x] matcher: `Bash|Edit|Write|MultiEdit`
  - [x] deny protected paths and dangerous commands with clear reason
- [x] generate `PermissionRequest`
  - [x] auto-allow only narrow known-safe Atlas maintenance commands
  - [x] never broad-match all permission prompts
- [x] generate `PermissionDenied`
  - [x] record denial
  - [x] optionally return retry guidance for safe alternate commands
- [x] generate `PostToolUse`
  - [x] matcher: `Edit|Write|MultiEdit|Bash`
  - [x] run `atlas update` after edits
  - [x] capture command/test/build result summaries
- [x] generate `PostToolUseFailure`
  - [x] capture failure summaries and unresolved errors
- [x] generate `Notification`
  - [x] optionally notify when Atlas detects stale graph or pending resume state
- [x] generate `SubagentStart` and `SubagentStop`
  - [x] track delegated work and merge subagent artifacts
- [x] generate `TaskCreated` and `TaskCompleted`
  - [x] map task lifecycle to Atlas session events
- [x] generate `Stop` and `StopFailure`
  - [x] persist final turn state or API-error state
- [x] generate `InstructionsLoaded`
  - [x] record loaded instruction/rule files as session context metadata
- [x] generate `ConfigChange`, `CwdChanged`, and `FileChanged`
  - [x] refresh repo root, config, and graph freshness signals
- [x] generate `WorktreeCreate` and `WorktreeRemove`
  - [x] bind Atlas session/worktree identity to temporary worktrees
- [x] generate `PreCompact` and `PostCompact`
  - [x] save resume snapshot before compaction
  - [x] verify snapshot availability after compaction
- [x] generate `Elicitation` and `ElicitationResult`
  - [x] record MCP user-input requests and responses as bounded events
- [x] generate `SessionEnd`
  - [x] cleanup transient bridge files and persist final snapshot

#### Codex hooks

Use Codex hook config with event -> matcher group -> command handlers. Current runtime is Bash-focused for tool hooks, and Windows hook execution is not supported.

- [x] generate `SessionStart`
  - [x] matcher: `startup|resume`
- [x] generate `UserPromptSubmit`
  - [x] capture user intent
  - [x] ignore matcher because Codex does not support matching for this event
- [x] generate `PreToolUse`
  - [x] matcher: `Bash`
  - [x] inspect `tool_input.command`
  - [x] deny risky shell commands before execution when configured
- [x] generate `PermissionRequest`
  - [x] matcher: `Bash`
  - [x] allow only narrow Atlas maintenance commands
  - [x] deny or defer destructive commands to normal approval flow
- [x] generate `PostToolUse`
  - [x] matcher: `Bash`
  - [x] summarize `tool_response`
  - [x] trigger `atlas update` after commands known to modify files
  - [x] add additional context when command output changes generated files or graph freshness
- [x] generate `Stop`
  - [x] persist final turn state and resume hints
  - [x] ignore matcher because Codex does not support matching for this event
- [x] support `statusMessage`, `timeout`, and `timeoutSec`
- [x] document unsupported current Codex hook gaps:
  - [x] non-Bash tool interception incomplete
  - [x] MCP, web, write, and non-shell tools may not trigger `PreToolUse` / `PostToolUse`
  - [x] `PostToolUse` cannot undo side effects from completed commands

#### MCP14 Patch

- [x] fail closed for unknown hook names instead of silently mapping them to generic `CommandRun`
- [x] expand `HookPolicy` so policy table owns lifecycle, prompt-routing, freshness, and review-refresh triggers instead of ad hoc event matching
- [x] persist restore metadata, retrieval hints, and saved artifact refs into stored hook event payloads, not only hook action output
- [x] validate that resume snapshots carry `saved_artifact_refs` from hook-saved artifacts, not only generic resume state
- [x] attach bounded retrieval hints and `source_id` summaries to persisted `user-prompt` session events, not only prompt-routing action output
- [x] add large-payload coverage for `user-prompt` and `stop`
- [x] route oversized session-only hook payloads through bounded event-plus-`source_id` storage, or trim them before session insertion, so large host events cannot violate session payload caps
- [x] add dedicated lifecycle coverage for `post-compact`, `session-end`, and `file-changed` freshness-stale behavior
- [x] implement bounded `review_context` / `explain_change` refresh artifacts after successful build/test `post-tool-use` flows and cover them with tests
- [x] either add bounded impact-refresh artifacts for successful build/test `post-tool-use` flows, or narrow MCP14 wording that currently says review/impact refresh to match shipped `review_context` plus `explain_change` behavior
- [x] add dedicated verification metadata coverage for `post-compact`, not only existence of lifecycle branch
- [x] add dedicated handoff metadata coverage for `session-end`, not only `stop`
- [x] implement explicit freshness-stale metadata for `file-changed`, and prove no inline file-content persistence in tests
- [x] expand end-to-end hook contract coverage for large prompt payloads, pre/post compact round-trip, stop plus session-end, and `file-changed` no-inline-content behavior
- [x] shrink `.atlas/hooks/atlas-hook` to thin launcher only; move stdin read/cap, event normalization, repo resolution, and dispatch policy into Rust `atlas hook` for deterministic behavior and better tests

Expected impact for thin-runner change:

- platform hook config files and installed command paths can stay unchanged if `.atlas/hooks/atlas-hook <frontend> <event>` remains stable
- installer runner-content tests in `packages/atlas-cli/src/install.rs` will need updates because they currently assert shell-script behavior directly
- installed-runner end-to-end tests in `packages/atlas-cli/tests/cli_quality_gates/core.rs` should keep passing if launcher argv contract stays stable, but they should gain regression coverage for thin-launcher behavior
- hook docs in `wiki/hooks-copilot.md`, `wiki/hooks-claude.md`, and `wiki/hooks-codex.md` should describe runner as launcher shim, not primary logic owner

#### Tests and docs

- [x] add fixture hook configs for all supported hosts
- [x] add schema validation tests for generated JSON
- [x] add stdin payload tests for each hook script
- [x] add redaction tests for command args, prompts, env values, and error output
- [x] add idempotent install tests that do not duplicate hooks
- [x] add uninstall or disable path for generated hooks
- [x] document source references:
  - [x] Copilot VS Code hooks: `https://code.visualstudio.com/docs/copilot/customization/hooks`
  - [x] GitHub Copilot cloud agent hooks: `https://docs.github.com/en/copilot/how-tos/copilot-on-github/customize-copilot/customize-cloud-agent/use-hooks`
  - [x] GitHub Copilot hooks config: `https://docs.github.com/en/copilot/reference/hooks-configuration`
  - [x] Claude hooks: `https://code.claude.com/docs/en/hooks-guide`
  - [x] Codex hooks: `https://developers.openai.com/codex/hooks`

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
  - [x] NOTE: current fuzzy behavior still needs symbol-typo recovery hardening; typoed symbols must not lose to docs/config nodes
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
- [x] consolidate old `atlas_review::assemble_review_context` review assembly into context-engine-backed review surfaces

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
- [ ] visualization/export

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

### Phase CM13 — Context Budget Optimization (depends on Operational Budget Policy Patch)

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

## Retrieval Ranking Evidence Patch

Atlas already exposes query scores, active query mode, global `explain_query` ranking factors, provenance, and truncation metadata. What is still missing is a first-class retrieval contract that explains why each returned result ranked where it did. A result-level score alone is not enough for agents to distinguish exact matches, fuzzy repairs, package/path boosts, changed-file boosts, graph expansion, and hybrid/vector fusion.

### Patch Q1 — Result-level ranking evidence model

- [ ] add compact `RankingEvidence` / `ScoreEvidence` model for ranked retrieval results
- [ ] attach evidence to graph/search result structs without replacing numeric score
- [ ] include fields for:
  - [ ] base retrieval mode (`fts5`, `regex_structural_scan`, `vector`, `hybrid`, `graph_expand`)
  - [ ] raw score before boosts when available
  - [ ] final score
  - [ ] matched fields (`name`, `qualified_name`, `file_path`, `content`, `embedding`)
  - [ ] exact name match
  - [ ] exact qualified-name match
  - [ ] prefix match
  - [ ] fuzzy correction and edit distance
  - [ ] kind boost
  - [ ] public/exported boost
  - [ ] same-directory boost
  - [ ] same-language boost
  - [ ] recent-file boost
  - [ ] changed-file boost
  - [ ] graph expansion hop distance
  - [ ] hybrid/RRF contributing sources and ranks
- [ ] keep evidence compact and stable for MCP JSON output
- [ ] add serde round-trip tests for evidence schema

Why:
- agents need to know why a result won, not only that it scored higher
- global `ranking_factors` explain query mode, but not individual result ranking

### Patch Q2 — Capture evidence during ranking

- [ ] update `apply_ranking_boosts` to record which boosts fired per result
- [ ] update fuzzy relaxed-candidate path to record:
  - [ ] corrected/matched term
  - [ ] edit distance
  - [ ] fuzzy threshold
- [ ] update exact-hit merge path to preserve exact-match evidence
- [ ] update graph expansion to record hop distance and seed source
- [ ] update hybrid/RRF merge to record:
  - [ ] FTS rank contribution
  - [ ] vector rank contribution
  - [ ] RRF score contribution
- [ ] ensure evidence survives result merging and deduplication
- [ ] add tests for each evidence source and merge precedence

Why:
- evidence must be produced at scoring time while the ranking decision is known
- reconstructing explanation after sorting is lossy and easy to get wrong

### Patch Q3 — Surface evidence in CLI and MCP retrieval outputs

- [ ] include ranking evidence in MCP `query_graph` results
- [ ] include ranking evidence in MCP `batch_query_graph` per-query results
- [ ] include ranking evidence in `explain_query` matches
- [ ] include ranking evidence in CLI `atlas query --json`
- [ ] keep human CLI output compact:
  - [ ] show score as today
  - [ ] optionally show top evidence labels when verbose/debug mode is enabled
- [ ] document stable evidence labels and meanings
- [ ] add snapshot tests for MCP output shape

Why:
- query-mode observability should be part of normal retrieval output, not only debug output
- downstream tools can make better escalation and trust decisions from structured evidence

### Patch Q4 — Evidence contract for context and review ranking

- [ ] decide whether review/context `relevance_score` also gets evidence
- [ ] if yes, add context-ranking evidence for:
  - [ ] direct target
  - [ ] changed symbol
  - [ ] caller/callee neighbor
  - [ ] test adjacency
  - [ ] impact-score contribution
  - [ ] saved-context/session boost
- [ ] surface context-ranking evidence only where payload budget allows
- [ ] document whether graph search evidence and context relevance evidence are separate contracts
- [ ] add tests for direct target and changed-file evidence in context results

Why:
- search ranking and context ranking are related but not identical
- review flows need evidence for why context was included, not only why a symbol matched search

### Patch Q completion criteria

- [ ] every ranked graph/search result can include compact structured ranking evidence
- [ ] query boosts, fuzzy correction, graph expansion, and hybrid/RRF all record evidence
- [ ] MCP `query_graph`, `batch_query_graph`, and `explain_query` expose evidence
- [ ] CLI JSON exposes evidence without bloating human output
- [ ] evidence labels are documented and covered by tests
- [ ] context/review relevance evidence is explicitly included or deferred with documented rationale

---

## Graph/Content Companion Patch

Atlas already has graph search for symbols and relationships plus file/content/template/text-asset search for prompts, docs, config, SQL, and templates. The missing design rule is that these are coordinated retrieval surfaces, not separate universes or a simple fallback chain. Graph answers code structure questions; content lookup answers non-code and context-adjacent questions; the context engine should merge both under one bounded selection, ranking, evidence, and truncation policy.

### Patch N1 — Declare graph/content lookup contract

- [ ] document canonical responsibility split:
  - [ ] graph search answers symbols, ownership, callers, callees, tests, imports, and structural relationships
  - [ ] content lookup answers prompts, docs, config, SQL, templates, logs, and embedded text assets
  - [ ] saved-context lookup answers prior Atlas outputs and session artifacts
  - [ ] context engine decides how these surfaces combine for a task
- [ ] define graph/content lookup as companion systems, not fallback-only systems
- [ ] define when both should be queried for one request:
  - [ ] review changes touching config or templates
  - [ ] symbols whose behavior depends on prompts or SQL
  - [ ] docs/spec questions tied to implementation files
  - [ ] agent/task questions needing saved context plus graph facts
- [ ] document anti-patterns:
  - [ ] broad file search before graph resolution for symbol questions
  - [ ] graph-only review when changed files include config/templates/prompts
  - [ ] content-only answers for structural dependency questions
  - [ ] separate unbounded result lists from graph and content tools

Why:
- non-code artifacts are first-class context when they affect behavior
- graph-first should not mean content-blind

### Patch N2 — Unified bounded selection policy

- [ ] define one context selection policy for mixed graph/content results:
  - [ ] direct graph targets first
  - [ ] changed files and changed symbols next
  - [ ] adjacent config/templates/prompts/SQL tied to changed files next
  - [ ] caller/callee/test evidence next
  - [ ] saved-session artifacts only when relevant to current task
- [ ] apply shared budgets across mixed results:
  - [ ] max graph nodes
  - [ ] max graph edges
  - [ ] max content assets
  - [ ] max saved artifacts
  - [ ] max total payload bytes/tokens
- [ ] ensure truncation reports mixed omissions:
  - [ ] omitted graph nodes
  - [ ] omitted graph edges
  - [ ] omitted content assets
  - [ ] omitted saved artifacts
  - [ ] omitted bytes/tokens
- [ ] add deterministic tie-breakers when graph and content scores compete
- [ ] add tests for mixed graph/content truncation order

Why:
- separate bounded lists can still create an unbounded combined context
- agents need one budget story for the final answer context

### Patch N3 — Coordinated ranking and evidence

- [ ] define a mixed-result ranking envelope with source kind:
  - [ ] `graph_node`
  - [ ] `graph_edge`
  - [ ] `file_asset`
  - [ ] `content_match`
  - [ ] `template`
  - [ ] `text_asset`
  - [ ] `saved_context`
- [ ] normalize ranking signals across surfaces:
  - [ ] exact symbol match
  - [ ] graph distance
  - [ ] changed-file boost
  - [ ] same package/directory boost
  - [ ] BM25/content match score
  - [ ] trigram/fuzzy correction
  - [ ] proximity/title/path rerank
  - [ ] session recency/relevance
- [ ] expose why each mixed item was selected through ranking evidence
- [ ] include `selection_reason` for both graph and content assets
- [ ] add tests proving config/template/prompt matches can be selected with graph evidence when relevant

Why:
- mixed context should be explainable, not an opaque concatenation of tool outputs
- ranking evidence must work for content assets as well as graph nodes

### Patch N4 — MCP and prompt workflow integration

- [ ] update MCP tool descriptions to describe graph/content companion rules
- [ ] update `review_change` prompt to query content assets when changed files include docs/config/templates/prompts/SQL
- [ ] update `inspect_symbol` prompt to look for context-adjacent assets only when graph evidence suggests dependency
- [ ] update installed AGENTS instructions:
  - [ ] graph tools first for structure
  - [ ] content tools as companion lookup for non-code assets
  - [ ] context engine should merge both under bounded policy
- [ ] add prompt/registry snapshot tests for companion-contract wording

Why:
- agents follow surface contracts more reliably than implicit architecture
- prompt and install docs should not describe content lookup as mere fallback

### Patch N completion criteria

- [ ] graph/content companion contract is documented as a design rule
- [ ] mixed graph/content context has one bounded selection policy
- [ ] mixed results expose source kind, selection reason, ranking evidence, and truncation metadata
- [ ] MCP prompts, tool descriptions, README, and installed AGENTS instructions agree
- [ ] tests cover mixed code + config/template/prompt/doc context assembly

---

## Parity Surface Patch

Atlas already has pieces of the upstream parity surface: Markdown heading graph nodes, content search over docs, large-function risk flags in review summaries, and explicit build/update plus flows/communities commands. Missing work is to turn those pieces into first-class CLI/MCP surfaces with shared service logic, compact output, and parity tests.

### Patch PS1 — Docs section lookup parity

- [ ] add docs-section lookup service over indexed project docs:
  - [ ] resolve doc by canonical repo path
  - [ ] resolve section by Markdown heading path / slug
  - [ ] return section body with bounded child-heading context
  - [ ] include heading level, line range, file hash, and truncation metadata
  - [ ] reuse existing Markdown parser heading nodes and content-store/file reads where possible
- [ ] add CLI surface:
  - [ ] `atlas docs-section <path> --heading <heading-path-or-slug>`
  - [ ] `atlas docs-section <path> --line <line>`
  - [ ] `--json`, `--max-bytes`, and stable not-found errors
- [ ] add MCP `get_docs_section`:
  - [ ] same inputs and defaults as CLI JSON
  - [ ] TOON/JSON output parity
  - [ ] provenance and freshness metadata
- [ ] add CLI/MCP parity tests:
  - [ ] nested headings
  - [ ] duplicate heading slugs
  - [ ] missing file / missing heading
  - [ ] max-byte truncation
  - [ ] stale graph warning when doc file changed

Why:
- current docs support can find files and headings, but cannot fetch one section as a stable agent-facing unit
- review/query workflows need precise docs excerpts without broad file scans

### Patch PS2 — Large-function finder parity

- [ ] add large-function analysis service:
  - [ ] scan function/method graph nodes by line span
  - [ ] configurable threshold with default matching review risk summary
  - [ ] rank by line count, fan-in/fan-out, changed-file relevance, and package/module boundary when available
  - [ ] return file path, qualified name, kind, line range, LOC, and ranking reason
  - [ ] support repo-wide and file-scoped modes
- [ ] add CLI surface:
  - [ ] `atlas find-large-functions`
  - [ ] `atlas find-large-functions --files ...`
  - [ ] `--threshold`, `--limit`, `--include-tests`, and `--json`
- [ ] add MCP `find_large_functions`:
  - [ ] same inputs and defaults as CLI JSON
  - [ ] compact defaults suitable for agent review
  - [ ] provenance and freshness metadata
- [ ] add CLI/MCP parity tests:
  - [ ] default threshold matches review large-function flag
  - [ ] file-scoped filtering
  - [ ] threshold and limit behavior
  - [ ] test-node include/exclude behavior
  - [ ] stable sort ties

Why:
- current review code only flags large changed functions; agents need direct repo/file discovery and ranked evidence
- one service prevents review, CLI, and MCP thresholds from drifting

### Patch PS3 — Explicit postprocess command parity

- [ ] define postprocess orchestration service for derived graph analytics:
  - [ ] run after build/update without reparsing source files
  - [ ] refresh derived analytics such as flows, communities, architecture metrics, query hints, and large-function summaries
  - [ ] support full and changed-only modes where data dependencies allow
  - [ ] record started/finished/failed state and per-stage counts/durations
  - [ ] keep failures bounded and machine-readable
- [ ] add CLI surface:
  - [ ] `atlas postprocess`
  - [ ] `atlas postprocess --changed-only`
  - [ ] `atlas postprocess --stage <name>`
  - [ ] `--json`, `--dry-run`, and stable error contract
- [ ] add MCP `postprocess_graph`:
  - [ ] same stage/mode controls as CLI JSON
  - [ ] compact stage summary by default
  - [ ] provenance, readiness, and freshness metadata
- [ ] add CLI/MCP parity tests:
  - [ ] no-op repo with no graph
  - [ ] full postprocess after build
  - [ ] changed-only postprocess after update
  - [ ] single-stage execution
  - [ ] stage failure surfaces same error code in CLI JSON and MCP

Why:
- build/update should stay focused on scan, parse, and persistence
- derived analytics need explicit orchestration instead of hidden side effects or ad hoc commands

### Patch PS completion criteria

- [ ] `get_docs_section`, `find_large_functions`, and `postprocess_graph` exist as MCP tools with matching CLI surfaces
- [ ] all three surfaces share service-layer implementations with no duplicated ranking, truncation, or error rules
- [ ] CLI JSON and MCP JSON are parity-tested for representative fixtures
- [ ] README, MCP reference, installed AGENTS instructions, and prompt workflows document the new surfaces consistently
- [ ] graph freshness/readiness metadata appears on every new graph-backed MCP response

---

## Runtime Event Enrichment and Graph Linking Patch

Atlas already has session events, adapter extraction helpers, content-store artifact routing, resume snapshots, saved-context retrieval, and context-engine saved-context merge. Do not replace that foundation with a parallel extractor system. Extend it with deterministic enrichment that turns runtime activity into bounded, graph-aware memory while preserving the existing storage boundaries: graph facts stay in `worldtree.db`, large/runtime artifacts stay in `context.db`, and session timelines stay in `session.db`.

### Patch X1 — Scope and crate boundary

- [ ] define this as enrichment over existing `atlas-session`, `atlas-contentstore`, and `atlas-adapters`
- [ ] avoid creating `packages/atlas-extractor` unless extraction logic grows large enough to justify a separate crate
- [ ] if a new crate is created later, require it to depend on service APIs, not write SQLite directly
- [ ] keep extractor pipeline deterministic, local, and non-LLM
- [ ] keep extractor best-effort; extraction failure must not block primary CLI/MCP tool output
- [ ] keep raw runtime output out of graph DB
- [ ] document storage ownership:
  - [ ] `worldtree.db` stores static code graph facts only
  - [ ] `session.db` stores bounded event metadata and references
  - [ ] `context.db` stores large artifacts, chunks, previews, and searchable runtime text

Why:
- existing continuity architecture already solved session/content boundaries
- a parallel extractor crate or DB path would duplicate behavior and increase drift

### Patch X2 — Raw input envelope and deterministic event enrichment

- [ ] define a `RuntimeInput` / `RawActivityInput` envelope for enrichment:
  - [ ] `frontend` (`cli`, `mcp`, adapter host)
  - [ ] `session_id`
  - [ ] `repo_root`
  - [ ] `input_kind`
  - [ ] `tool_or_command`
  - [ ] `status`
  - [ ] `stdout_preview`
  - [ ] `stderr_preview`
  - [ ] `artifact_source_id`
  - [ ] `files`
  - [ ] `metadata`
  - [ ] `created_at`
- [ ] define enriched output that maps onto existing `NewSessionEvent` payloads:
  - [ ] `event_type`
  - [ ] `summary`
  - [ ] `symbols`
  - [ ] `file_paths`
  - [ ] `source_ids`
  - [ ] `classification`
  - [ ] `confidence`
  - [ ] `metadata`
- [ ] enrich existing event constructors rather than bypassing them:
  - [ ] `extract_cli_event`
  - [ ] `extract_graph_event`
  - [ ] `extract_context_event`
  - [ ] `extract_reasoning_event`
  - [ ] `extract_user_event`
  - [ ] `extract_tool_event`
  - [ ] `normalize_event`
- [ ] keep outputs canonical JSON so existing event hashing and dedupe remain stable
- [ ] add tests proving same input produces same enriched event and hash

Why:
- enrichment should preserve existing event persistence and dedupe semantics
- deterministic input/output keeps resume snapshots stable

### Patch X3 — Rule-based classification

- [ ] add bounded rule-based classifiers for runtime activity:
  - [ ] panic
  - [ ] exception
  - [ ] stacktrace
  - [ ] compiler error
  - [ ] test failure
  - [ ] test success
  - [ ] build success
  - [ ] deprecation warning
  - [ ] unused/dead-code warning
  - [ ] permission denied
  - [ ] command timeout
  - [ ] graph stale/readiness warning
  - [ ] retrieval/content-store failure
- [ ] map classifications to existing `SessionEventType` values where possible:
  - [ ] `ERROR`
  - [ ] `COMMAND_RUN`
  - [ ] `COMMAND_FAIL`
  - [ ] `CONTEXT_REQUEST`
  - [ ] `REASONING_RESULT`
  - [ ] `FILE_READ`
  - [ ] `FILE_WRITE`
  - [ ] `GRAPH_BUILD`
  - [ ] `GRAPH_UPDATE`
- [ ] add new event types only when existing types cannot represent the event safely
- [ ] include classification metadata instead of exploding event-type count:
  - [ ] `classification.kind`
  - [ ] `classification.severity`
  - [ ] `classification.rule_id`
  - [ ] `classification.matched_fields`
- [ ] add tests for error parsing, warning parsing, test summary parsing, and no-match behavior

Why:
- event type should stay stable; detailed meaning belongs in structured metadata
- deterministic classifiers provide useful memory without LLM inference

### Patch X4 — Artifact routing before session insertion

- [ ] run all raw stdout/stderr/tool-result blobs through existing content-store routing before session insertion
- [ ] define routing thresholds through the central budget policy:
  - [ ] `small_output_bytes`
  - [ ] `preview_output_bytes`
  - [ ] `large_output_bytes`
  - [ ] `max_runtime_artifact_bytes`
- [ ] keep session event payloads bounded:
  - [ ] small output may be stored inline only when safe and redacted
  - [ ] medium output stores preview plus `source_id`
  - [ ] large output stores pointer only
- [ ] use `ContentStore::route_output` / saved-context artifact routing instead of a new artifact path
- [ ] index routed artifacts with metadata:
  - [ ] `session_id`
  - [ ] `source_type`
  - [ ] `tool_or_command`
  - [ ] `repo_root`
  - [ ] `file_paths`
  - [ ] `symbols`
  - [ ] `classification`
- [ ] ensure secrets are redacted before persistence and previews
- [ ] add tests for small, medium, large, oversized, and secret-bearing outputs

Why:
- `SessionStore::append_event` already rejects oversized inline payloads
- content store is the correct place for searchable runtime text

### Patch X5 — Graph linking without storing runtime data in graph DB

- [ ] link enriched events to graph facts by stable identifiers, not raw node IDs alone
- [ ] store links in session/content side tables, not `worldtree.db`
- [ ] define link records:
  - [ ] `event_id`
  - [ ] `session_id`
  - [ ] `repo_root`
  - [ ] `qualified_name`
  - [ ] `canonical_file_path`
  - [ ] optional `node_id`
  - [ ] optional `file_id`
  - [ ] `link_kind`
  - [ ] `confidence`
  - [ ] `graph_last_indexed_at`
- [ ] prefer canonical identifiers:
  - [ ] canonical repo path
  - [ ] qualified name
  - [ ] kind
  - [ ] line span when available
- [ ] treat `node_id` and `file_id` as cache hints only because graph rebuilds can change row IDs
- [ ] make graph linking best-effort:
  - [ ] events with no graph target remain valid runtime memory
  - [ ] ambiguous symbols store candidate list and ambiguity metadata
  - [ ] stale graph state records `safe_to_answer=false` for link-derived claims when needed
- [ ] add tests for exact symbol, file path, ambiguous symbol, stale graph, and graph-missing cases

Why:
- runtime memory should be graph-aware without mutating graph facts
- stable identifiers survive rebuilds better than SQLite row IDs

### Patch X6 — Readiness, identity, and budget integration

- [ ] run graph linking only through canonical graph readiness state
- [ ] define behavior by execution state:
  - [ ] `fresh` -> resolve and link normally
  - [ ] `stale` -> link with freshness warning and stale metadata
  - [ ] `partial` -> link only when completeness requirements are met
  - [ ] `corrupt` -> skip graph linking and store runtime event without graph links
- [ ] require canonical path identity before any event/file/artifact key hashing
- [ ] apply central budget policy to:
  - [ ] classifier input bytes
  - [ ] number of symbols extracted
  - [ ] number of file paths extracted
  - [ ] number of graph lookup candidates
  - [ ] number of links stored
  - [ ] artifact preview bytes
- [ ] emit enrichment budget metadata:
  - [ ] `budget_hit`
  - [ ] `partial`
  - [ ] `safe_to_answer`
  - [ ] omitted symbol/file/link counts
- [ ] add tests for stale/partial/corrupt graph behavior and budget truncation

Why:
- runtime enrichment must follow the same safety rules as graph-backed tools
- extraction can otherwise become another unbounded path

### Patch X7 — Context-engine integration

- [ ] extend context engine to include enriched runtime events only when requested or relevant
- [ ] add request controls:
  - [ ] `include_runtime_events`
  - [ ] `runtime_event_limit`
  - [ ] `runtime_artifact_limit`
  - [ ] `runtime_since`
  - [ ] `runtime_session_id`
- [ ] retrieve runtime memory by:
  - [ ] linked symbol
  - [ ] canonical file path
  - [ ] session id
  - [ ] classification kind
  - [ ] artifact source id
- [ ] merge runtime memory under graph/content companion policy
- [ ] expose source kind:
  - [ ] `runtime_event`
  - [ ] `runtime_artifact`
  - [ ] `saved_context`
- [ ] include selection reason and ranking evidence:
  - [ ] same symbol
  - [ ] same file
  - [ ] recent error
  - [ ] same session
  - [ ] direct artifact reference
- [ ] keep runtime context bounded and preview-only by default
- [ ] add tests for context with graph-only, saved-context-only, runtime-event-only, and mixed graph/runtime inputs

Why:
- runtime memory is useful only when it participates in context selection
- it must not bypass existing context budgets or ranking rules

### Patch X8 — CLI, MCP, and hook integration

- [ ] integrate enrichment with existing CLI adapter event flow
- [ ] integrate enrichment with MCP tool handler boundaries
- [ ] keep MCP session event persistence best-effort and non-blocking
- [ ] avoid duplicating `save_context_artifact`; reuse existing tool and content routing
- [ ] update hook integration roadmap so host hooks emit enriched inputs through service APIs
- [ ] ensure generated hooks never write SQLite directly
- [ ] add command/tool metadata for:
  - [ ] command start
  - [ ] command finish
  - [ ] tool result
  - [ ] permission decision
  - [ ] compaction boundary
  - [ ] session end
  - [ ] error/failure
- [ ] add integration tests for CLI, MCP, and bridge-file fallback event enrichment

Why:
- runtime memory should come from existing adapters and hooks
- host-specific capture gaps must reduce enrichment quality, not break commands

### Patch X9 — Resume snapshot enrichment

- [ ] include enriched runtime signals in resume snapshots:
  - [ ] recent errors
  - [ ] recent failed commands
  - [ ] recent successful build/test summaries
  - [ ] linked symbols
  - [ ] linked files
  - [ ] artifact references
  - [ ] active unresolved runtime issues
- [ ] group by category and severity
- [ ] include retrieval hints instead of raw artifact content
- [ ] cap snapshot contribution by budget policy
- [ ] make snapshot rendering deterministic
- [ ] add snapshot tests for enriched errors, artifact references, and linked symbols

Why:
- resume should recover useful runtime state without replaying history
- enriched events make snapshots more useful while staying compact

### Patch X completion criteria

- [ ] runtime enrichment extends existing session/content/adapters architecture without replacing it
- [ ] no runtime data is stored in graph DB
- [ ] large runtime outputs route through content store before session insertion
- [ ] enriched events are deterministic, bounded, redacted, and deduplicated
- [ ] event-to-graph links use stable identifiers and treat row IDs as optional cache hints
- [ ] graph linking obeys readiness state and budget policy
- [ ] context engine can merge runtime events/artifacts with graph and saved context under one bounded ranking policy
- [ ] CLI, MCP, and hook flows feed enrichment best-effort
- [ ] resume snapshots include compact enriched runtime signals
- [ ] tests cover classification, artifact routing, graph linking, context integration, and resume enrichment

---

## Ranking and Trimming Primitives Patch

Atlas already requires MCP/context surfaces to stay thin over the context engine, and Phase 22 checks review/symbol/impact parity. Widen that rule to the whole graph/query core: no CLI, MCP, review, explain-change, impact, analyze, retrieval, or context path should carry its own ad hoc ranking or trimming rules when a shared primitive can own the decision.

### Patch D1 — Inventory and classify all ranking/trimming paths

- [x] inventory every ranking, scoring, sorting, truncation, and trimming path:
  - [x] CLI `query`
  - [x] MCP `query_graph`
  - [x] MCP `batch_query_graph`
  - [x] `explain_query`
  - [x] `get_context`
  - [x] `get_minimal_context`
  - [x] `get_review_context`
  - [x] `explain_change`
  - [x] `get_impact_radius`
  - [x] `analyze_safety`
  - [x] `analyze_remove`
  - [x] `analyze_dead_code`
  - [x] `analyze_dependency`
  - [x] saved-context retrieval
  - [x] graph expansion
  - [x] hybrid/RRF retrieval
  - [x] content/file/template/text-asset lookup
- [x] classify each path as:
  - [x] shared primitive
  - [x] domain adapter around shared primitive
  - [x] presentation-only sorting
  - [x] duplicate logic to remove
- [x] document allowed reasons for a separate domain adapter
- [x] add a checklist table mapping each public command/tool to its ranking/trimming primitive

Why:
- duplicated ranking logic hides in small `sort_by` and `truncate` blocks
- inventory makes drift visible before refactors start

### Patch D2 — Define shared ranking primitives

- [x] define shared primitives for graph/search ranking:
  - [x] exact and qualified-name match boosts
  - [x] fuzzy correction boosts
  - [x] package/directory/language boosts
  - [x] changed-file/recent-file boosts
  - [x] graph distance scoring
  - [x] hybrid/RRF merging
- [x] define shared primitives for context/review ranking:
  - [x] direct target priority
  - [x] caller/callee/test adjacency
  - [x] impact score contribution
  - [x] changed-symbol contribution
  - [x] saved-context/session contribution
- [x] define shared trimming primitives:
  - [x] max nodes
  - [x] max edges
  - [x] max files
  - [x] max content assets
  - [x] max payload bytes/tokens
  - [x] deterministic omission metadata
- [x] keep presentation formatting separate from ranking/trimming decisions
- [x] add unit tests for each primitive and tie-breaker

Why:
- query, context, review, and analysis need consistent ordering semantics
- shared primitives make ranking evidence and budget metadata easier to trust

### Patch D3 — Route public tools through shared primitives

- [x] update CLI query to use shared graph/search ranking primitive
- [x] update MCP `query_graph` and `batch_query_graph` to use same primitive as CLI query
- [x] update `explain_query` to explain the same primitive used by actual query execution
- [x] update context engine ranking/trimming to use shared context primitives
- [x] update review-context assembly to use shared context/review primitives
- [x] update explain-change and impact analysis to use shared impact/context primitives
- [x] update analyze-* commands to use shared analysis ranking/trimming primitives
- [x] update saved-context ranking to use a documented adapter around shared context ranking
- [x] remove or quarantine duplicate `sort_by` / `truncate` logic outside approved primitives

Why:
- public tools should disagree only because inputs differ, not because ranking rules forked
- `explain_query` must never describe different ranking than `query_graph` uses

### Patch D4 — Guard against future drift

- [x] add parity tests:
  - [x] CLI query versus MCP query for same inputs
  - [x] MCP query versus `explain_query` ranking explanation
  - [x] review-context versus get-context for shared seed inputs
  - [x] impact versus explain-change for shared changed files
  - [x] analyze-* output ordering versus underlying primitive ordering
- [x] add targeted tests for stable tie-breakers
- [x] add snapshot tests for truncation/omission metadata
- [x] document review rule: new public graph/query/context tool must name its ranking/trimming primitive
- [x] optionally add lint-like test searching for new ad hoc `sort_by`/`truncate` in public tool layers

Why:
- ranking drift usually returns as small local convenience code
- parity tests make drift fail loudly

### Patch D completion criteria

- [x] every public graph/query/context/review/analysis path maps to a shared ranking/trimming primitive or documented adapter
- [x] CLI and MCP query paths share ranking semantics
- [x] review, context, impact, explain-change, and analyze-* paths share trimming semantics where applicable
- [x] duplicate public-layer ranking/trimming logic is removed or documented as presentation-only
- [x] parity tests cover query, context, review, impact, explain-change, and analyze-* paths
- [x] future tool contract requires naming the ranking/trimming primitive used

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


## Canonical Path Identity Patch

Atlas already normalizes many repo paths during scan, diff handling, and some call-resolution flows, but path identity must be stronger than local normalization. Every store key, snapshot key, cache key, `source_id` seed, `chunk_id` seed, graph node/file key, and future sidecar index key must use one canonical repo-relative path identity before hashing or persistence.

### Patch P1 — Canonical repo path type and rules

- [x] state invariant explicitly: ALL path-derived keys MUST derive from canonical repo-relative path identity before hashing, persistence, dedupe, or cross-store ID generation
- [x] define a shared `CanonicalRepoPath` / `RepoPathIdentity` type in `atlas-repo` or shared core
- [x] enforce canonical form:
  - [x] repo-relative only
  - [x] forward-slash separators only
  - [x] no leading `./`
  - [x] no empty path
  - [x] no `.` components
  - [x] no unresolved `..` components
  - [x] no trailing slash for files
  - [x] platform-aware case policy applied once
- [x] provide constructors for:
  - [x] absolute path + repo root
  - [x] repo-relative path string
  - [x] git diff path
  - [x] watch event path
  - [x] explicit CLI/MCP file argument
  - [x] synthetic graph path
- [x] make invalid paths return typed errors instead of silently falling back to raw input
- [x] add property/unit tests for separators, dot segments, Windows casing, absolute paths, synthetic paths, and invalid escape paths

Why:
- prevents path-casing and separator drift before hashing
- makes path identity a contract, not scattered string cleanup

### Patch P2 — Use canonical identity for graph store keys

- [x] require `CanonicalRepoPath` before writing `files.path`
- [x] require canonical path before `replace_file_graph`
- [x] require canonical path before `replace_files_transactional`
- [x] require canonical path before `nodes.file_path`
- [x] require canonical path before qualified-name file prefixes
- [x] require canonical path before graph edge file metadata
- [x] update build/update/watch/diff paths to normalize once at boundary
- [x] add tests proving equivalent raw paths map to one graph file identity
- [x] add tests proving case policy is stable on Windows

Why:
- graph identity depends on file path strings embedded in nodes, edges, QNs, and lookup keys
- equivalent paths must not create duplicate graph facts

### Patch P3 — Use canonical identity for content, session, and adapter IDs

- [x] require canonical path before content-store `source_id` when artifact represents a repo file
- [x] require canonical path before `chunk_id` source seed when source is file-backed
- [x] require canonical path before retrieval cache keys
- [x] require canonical path before saved-context references that point to repo files
- [x] require canonical path before session resume snapshot file references
- [x] require canonical path before adapter bridge `source_id` path hashing
- [x] require canonical path before historical graph snapshot keys and file-hash reuse keys
- [x] require canonical path before Phase 17 history/diff deduplication keys
- [x] keep non-file artifacts explicit:
  - [x] label/content-derived IDs allowed only when no repo path exists
  - [x] ID payload must mark identity kind: `repo_path`, `synthetic_path`, `artifact_label`, or `external`
- [x] add tests across content store, session store, MCP save/search, and adapter bridge ingestion

Why:
- cross-store joins and future sidecar indexes break when graph uses one path form and content/session use another
- hashing raw paths bakes bugs permanently into IDs

### Patch P4 — Audit and migration guardrails

- [x] audit all hashing/keying call sites for raw path usage
- [x] add lint-like tests or targeted regression tests for forbidden raw path hashing
- [x] document migration behavior for existing noncanonical rows
- [x] add diagnostic to `doctor` / `db_check` for noncanonical path rows
- [x] decide whether to rewrite existing rows during rebuild or require clean rebuild
- [x] document invariant in AGENTS/install instructions so agents preserve path identity

Why:
- existing code has multiple local path-normalization helpers
- future cache/sidebar/retrieval features must not reintroduce raw path keys

### Patch P completion criteria

- [x] one canonical repo path identity type exists
- [x] ALL path-derived keys are documented as canonical-before-hash/canonical-before-persist
- [x] graph store, content store, session snapshots, adapters, and MCP use it before hashing/keying
- [x] historical graph snapshots, file-hash reuse, and snapshot dedupe use the same identity
- [x] raw path hashing is covered by regression tests
- [x] diagnostics detect noncanonical persisted path rows
- [x] future sidecar/cache/index keys document canonical path as required seed

---

## Graph Readiness Source-of-Truth Patch

Atlas has persisted build state, graph freshness checks, health/debug tools, provenance, and adapter metadata, but there is no explicit invariant that one subsystem owns the answer to: "is the graph ready, searchable, and current enough to use?" That decision must not drift across CLI status, MCP status, query tools, impact analysis, review context, and adapters.

### Patch S1 — Canonical graph readiness record

- [ ] define a canonical `GraphReadiness` / `GraphState` model in shared core or graph service code
- [ ] include fields:
  - [ ] `repo_root`
  - [ ] `db_path`
  - [ ] `db_exists`
  - [ ] `db_open_error`
  - [ ] `build_state`
  - [ ] `build_last_error`
  - [ ] `graph_built`
  - [ ] `graph_queryable`
  - [ ] `graph_current`
  - [ ] `stale_index`
  - [ ] `pending_graph_changes`
  - [ ] `integrity_state`
  - [ ] `error_code`
  - [ ] `message`
  - [ ] `suggestions`
  - [ ] `last_indexed_at`
  - [ ] `indexed_file_count`
- [ ] distinguish readiness dimensions:
  - [ ] built versus missing
  - [ ] queryable versus blocked
  - [ ] current versus stale
  - [ ] corrupt/inconsistent versus merely stale
  - [ ] graph readiness versus retrieval/content index readiness
- [ ] make this record the only source allowed to decide graph readiness
- [ ] add tests for every readiness class and field derivation

Why:
- prevents drift between build lifecycle state, status output, query behavior, and adapter metadata
- makes readiness a contract instead of scattered boolean logic

### Patch S1.5 — Graph execution safety states

- [ ] define canonical graph execution states:
  - [ ] `fresh` — graph is built, queryable, current, and integrity-clean
  - [ ] `stale` — graph is queryable but behind graph-relevant working-tree changes
  - [ ] `partial` — graph is queryable but build/update/indexing stopped early or degraded
  - [ ] `corrupt` — graph has SQLite integrity errors, schema mismatch, orphan nodes, or dangling edges
- [ ] define feature behavior by state:
  - [ ] `fresh` -> full graph-backed features enabled
  - [ ] `stale` -> warn and allow graph-backed answers with freshness metadata
  - [ ] `partial` -> allow limited features only; block answers requiring complete graph facts
  - [ ] `corrupt` -> block graph-backed answers and require rebuild/quarantine flow
- [ ] define explicit override policy:
  - [ ] stale graph may run only when default policy allows stale reads or caller passes `--allow-stale` / MCP `allow_stale=true`
  - [ ] partial graph may run only for tools with documented degraded behavior or caller passes `--allow-partial` / MCP `allow_partial=true`
  - [ ] corrupt graph has no override for graph-backed answers
  - [ ] every allowed stale/partial response must include `safe_to_answer`, execution state, and freshness/degraded metadata
- [ ] define which tools are allowed in `partial` state:
  - [ ] status/debug/doctor allowed
  - [ ] direct symbol lookup allowed only when result provenance is complete enough
  - [ ] impact/review/analyze flows blocked or degraded unless completeness requirements are met
  - [ ] traversal blocked when missing edges could make answer unsafe
- [ ] expose execution state in CLI/MCP readiness output
- [ ] make query, impact, review, context, and analyze tools consume execution state before graph reads
- [ ] add tests for each state and allowed/blocked feature behavior

Why:
- agents need one simple safety state before deciding whether graph facts are usable
- stale, partial, and corrupt graphs require different behavior

### Patch S2 — Route CLI graph tools through canonical readiness

- [ ] update `atlas status` to emit canonical readiness directly
- [ ] update `atlas doctor` to reference canonical readiness instead of partially recomputing it
- [ ] update `atlas query` to consult readiness before search
- [ ] update `atlas impact` to consult readiness before impact traversal
- [ ] update `atlas review-context` to consult readiness before context assembly
- [ ] update reasoning/refactor graph-backed commands to consult readiness before graph reads
- [ ] define command behavior per readiness state:
  - [ ] fresh graph: full features enabled
  - [ ] missing graph: fail with build suggestion
  - [ ] interrupted/failed build: fail with lifecycle suggestion
  - [ ] stale graph: warn + allow only by configured policy or explicit stale override
  - [ ] partial graph: allow limited features only by documented degraded policy or explicit partial override
  - [ ] corrupt/inconsistent graph: fail closed
- [ ] add CLI tests proving all graph-backed commands consume same readiness decision

Why:
- query, impact, and review must not infer readiness from `Store::open` alone
- status output and command behavior must agree

### Patch S3 — Route MCP and adapters through canonical readiness

- [ ] update MCP `status` to surface canonical readiness, not redefine it
- [ ] add readiness block to graph-backed MCP responses:
  - [ ] `query_graph`
  - [ ] `get_context`
  - [ ] `get_impact_radius`
  - [ ] `get_review_context`
  - [ ] `get_minimal_context`
  - [ ] `symbol_neighbors`
  - [ ] `traverse_graph`
  - [ ] reasoning/refactor analysis tools
- [ ] replace ad hoc provenance/freshness readiness inference with canonical readiness fields
- [ ] keep provenance as identity metadata only:
  - [ ] `repo_root`
  - [ ] `db_path`
  - [ ] `indexed_file_count`
  - [ ] `last_indexed_at`
- [ ] ensure adapters never decide graph readiness independently
- [ ] add MCP tests proving graph-backed tools surface identical readiness for same repo/db

Why:
- MCP should surface graph readiness, not become another readiness authority
- provenance and readiness are related but not the same contract

### Patch S completion criteria

- [ ] one canonical graph readiness model exists
- [ ] CLI status, doctor, query, impact, and review consume that model
- [ ] MCP graph-backed tools surface that model and do not redefine readiness
- [ ] adapters only report or forward readiness, never compute their own
- [ ] stale/queryable and corrupt/blocked states are distinct
- [ ] fresh/stale/partial/corrupt execution states map to explicit allowed/blocked features
- [ ] tests prove all graph-backed paths agree on readiness for same repo and DB

---

## Operational Budget Policy Patch

Atlas already has per-file size caps, parse batch sizing, bounded queues, result node/file caps, session payload caps, and retrieval guardrail backlog items. What is still missing is one explicit operational budget policy across build, query, impact, review, context, and MCP output stages: how much work may be attempted, what happens when a hard budget is hit, and whether each stage fails open, fails closed, or returns degraded partial results.

### Patch B0 — Central budget policy and manager

- [x] define one shared `BudgetPolicy` / `BudgetManager` model for all bounded work
- [x] include budget namespaces:
  - [x] build/update
  - [x] graph traversal
  - [x] query candidates and seeds
  - [x] review/context extraction
  - [x] content/saved-context lookup
  - [x] MCP/CLI payload serialization
- [x] make all public graph/query/context/review/analysis paths receive budget policy from one source
- [x] support configured defaults plus per-call overrides within safe maximums
- [x] emit one shared budget result shape:
  - [x] `budget_status`
  - [x] `budget_hit`
  - [x] `budget_name`
  - [x] `budget_limit`
  - [x] `budget_observed`
  - [x] `partial`
  - [x] `safe_to_answer`
- [x] add tests proving scattered local caps cannot bypass central policy

Why:
- a list of limits is not enough if each tool interprets limits differently
- one budget manager prevents drift across traversal, context, and serialization layers

### Patch B1 — Build and update budgets

- [x] add explicit build/update budget config:
  - [x] `max_files_per_run`
  - [x] `max_total_bytes_per_run`
  - [x] `max_file_bytes`
  - [x] `max_parse_failures`
  - [x] `max_parse_failure_ratio`
  - [x] `max_wall_time_ms`
- [x] track budget counters during build/update:
  - [x] files discovered
  - [x] files accepted
  - [x] files skipped by byte budget
  - [x] bytes accepted
  - [x] bytes skipped
  - [x] parse failures
  - [x] budget stop reason
- [x] define behavior when budgets hit:
  - [x] hard fail for invalid config
  - [x] degraded partial build when skip budgets hit and policy allows
  - [x] fail closed when parse-failure budget is exceeded
  - [x] mark graph state as `build_failed` or `degraded` with reason
- [x] expose counters and stop reason in CLI/MCP build/update output
- [x] add tests for max file budget, max byte budget, parse failure threshold, and partial/degraded result reporting

Why:
- batch size prevents unbounded queues but does not cap total build work
- operators need predictable upper bounds for large repos and pathological changes

### Patch B2 — Query, seed, and traversal budgets

- [x] add explicit query/traversal budget config:
  - [x] `max_seed_nodes`
  - [x] `max_seed_files`
  - [x] `max_traversal_depth`
  - [x] `max_traversal_nodes`
  - [x] `max_traversal_edges`
  - [x] `max_query_candidates`
  - [x] `max_query_wall_time_ms`
- [x] apply seed budgets before graph expansion, impact analysis, and context assembly
- [x] distinguish seed truncation from result truncation
- [x] include seed-budget metadata in responses:
  - [x] requested seed count
  - [x] accepted seed count
  - [x] omitted seed count
  - [x] budget hit flags
  - [x] suggested narrower query
- [x] define behavior when seed budgets hit:
  - [x] fail closed for ambiguous unbounded seeds
  - [x] return partial bounded result for explicit file/symbol lists when policy allows
  - [x] require narrower input when omission would make result misleading
- [x] add tests for broad query seed explosion, explicit file list truncation, and traversal cap reporting

Why:
- `max_nodes` caps returned context, not necessarily seed loading or traversal work
- seed explosion can make bounded output misleading unless reported explicitly

### Patch B3 — Review/context payload budgets

- [x] add explicit review/context budget config:
  - [x] `max_review_source_bytes`
  - [x] `max_context_payload_bytes`
  - [x] `max_context_tokens_estimate`
  - [x] `max_file_excerpt_bytes`
  - [x] `max_saved_context_bytes`
  - [x] `max_mcp_response_bytes`
- [x] apply byte/token budgets before serializing CLI/MCP output
- [x] make truncation deterministic:
  - [x] preserve direct targets first
  - [x] preserve highest-ranked nodes/edges next
  - [x] preserve risk/test evidence before low-signal context
  - [x] drop saved artifacts before graph essentials unless intent says otherwise
- [x] include truncation metadata:
  - [x] bytes requested
  - [x] bytes emitted
  - [x] tokens estimated
  - [x] omitted node/file/source counts
  - [x] omitted byte counts
  - [x] continuation or narrower-query hints
- [x] add tests for review-context byte cap, saved-context cap, file excerpt cap, and MCP response cap

Why:
- node/file caps do not guarantee bounded serialized payload size
- large excerpts and saved artifacts can bypass graph caps unless independently budgeted

### Patch B4 — Fail-open versus fail-closed policy

- [x] define budget-hit behavior matrix for each stage:
  - [x] build
  - [x] update
  - [x] query
  - [x] impact
  - [x] review context
  - [x] minimal context
  - [x] saved-context retrieval
  - [x] MCP response serialization
- [x] classify each budget as hard or soft:
  - [x] hard budget returns error
  - [x] soft budget returns partial result with warning
  - [x] degraded budget marks graph/context state degraded
- [x] surface machine-readable budget status:
  - [x] `budget_status`
  - [x] `budget_hit`
  - [x] `budget_name`
  - [x] `budget_limit`
  - [x] `budget_observed`
  - [x] `partial`
  - [x] `safe_to_answer`
- [x] document which budget hits make agent answers unsafe
- [x] add tests for fail-open, fail-closed, and degraded cases

Why:
- budget behavior must be predictable, not inferred from missing rows or truncated arrays
- agents need to know whether partial context is safe to use

### Patch B completion criteria

- [ ] one central budget policy/manager exists and all bounded graph/query/context paths consume it
- [ ] build/update total work budgets exist and are reported
- [ ] query seed and traversal budgets exist and are reported
- [ ] review/context byte or token budgets exist and are reported
- [ ] MCP/CLI payload budgets are enforced by the same budget policy
- [ ] every budget has explicit fail-open, fail-closed, or degraded behavior
- [ ] CLI/MCP outputs include machine-readable budget status when limits hit
- [ ] tests cover budget hits across build, query, impact, review, and MCP serialization

---

## Context Escalation Contract Patch

Atlas has compact context tools, review context, symbol lookup, neighbor tools, and wider traversal tools, but the preferred order is currently only hinted in prompts and installed instructions. Make the core agent workflow explicit: start with the smallest bounded graph context that can answer the question, then escalate only when evidence says broader context is needed.

### Patch E1 — Define minimal-context-first workflow

- [ ] document canonical escalation order for review/change tasks:
  - [ ] `detect_changes` when files are unknown
  - [ ] `get_minimal_context` for first bounded triage
  - [ ] `get_review_context` only when changed-symbol, neighbor, or risk detail is needed
  - [ ] `explain_change` when deterministic risk/test-gap explanation is needed
  - [ ] `get_impact_radius` when explicit blast radius is needed
- [ ] document canonical escalation order for symbol/usage tasks:
  - [ ] `query_graph` / `resolve_symbol` first
  - [ ] `symbol_neighbors` for direct callers/callees/tests
  - [ ] `get_context` for bounded ranked context
  - [ ] `traverse_graph` only when one-hop context is insufficient
- [ ] define allowed reasons to escalate:
  - [ ] ambiguous symbol resolution
  - [ ] truncated result
  - [ ] missing caller/callee/test evidence
  - [ ] cross-file or cross-package risk
  - [ ] explicit user request for broader context
  - [ ] safety-critical uncertainty
- [ ] define anti-patterns:
  - [ ] starting review with full review context when minimal context is enough
  - [ ] using traversal before symbol resolution
  - [ ] using file search before graph tools answer structural questions
  - [ ] broad traversal without a bounded max depth and max nodes

Why:
- reduces token load and noisy context
- keeps graph workflows deterministic and cheap by default

### Patch E2 — Surface contract in MCP, prompts, and installed instructions

- [ ] update MCP tool descriptions to mention minimal-first escalation where relevant
- [ ] update `review_change` prompt to make minimal context first a requirement, not just a recommendation
- [ ] update `inspect_symbol` prompt to require direct-neighbor context before wider traversal
- [ ] update installed AGENTS instructions to state escalation order clearly
- [ ] update README MCP workflow section to match same order
- [ ] ensure wording is consistent across CLI install block, MCP prompts, and README

Why:
- agents follow tool descriptions and prompts more reliably than implicit design intent
- one workflow description prevents drift across docs and MCP metadata

### Patch E2.5 — Enforce minimal-context-first inside higher-level tools

- [ ] require higher-level tools to start from minimal bounded context internally unless explicitly bypassed:
  - [ ] `get_review_context`
  - [ ] `explain_change`
  - [ ] `get_impact_radius`
  - [ ] `analyze_safety`
  - [ ] `analyze_remove`
  - [ ] `analyze_dead_code`
  - [ ] `analyze_dependency`
  - [ ] refactor planning tools
- [ ] define explicit bypass reasons:
  - [ ] user requested full context
  - [ ] minimal context is truncated
  - [ ] minimal context reports ambiguity
  - [ ] tool requires full impact graph by contract
  - [ ] configured safety policy requires broader context
- [ ] include metadata showing whether minimal context was used, bypassed, or escalated:
  - [ ] `minimal_context_used`
  - [ ] `minimal_context_bypassed`
  - [ ] `escalation_reason`
  - [ ] `next_tools`
- [ ] add tests proving review/analyze/impact tools do not over-fetch when minimal context is sufficient

Why:
- workflow guidance is weaker than internal enforcement
- higher-level tools should not silently bypass bounded triage

### Patch E3 — Add escalation metadata and tests where practical

- [ ] include response metadata that helps decide whether to escalate:
  - [ ] `truncated`
  - [ ] `omitted_count`
  - [ ] `ambiguity`
  - [ ] `next_tools`
  - [ ] `recommended_escalation_reason`
- [ ] ensure `get_minimal_context` reports when review context would add useful detail
- [ ] ensure `symbol_neighbors` reports when traversal may be needed because caps were hit
- [ ] add prompt/registry snapshot tests for minimal-first contract wording
- [ ] add MCP response tests for escalation metadata on truncated/ambiguous outputs

Why:
- tools should tell agents when more context is justified
- escalation should be evidence-driven, not habit-driven

### Patch E completion criteria

- [ ] minimal-context-first contract is documented as required workflow
- [ ] higher-level tools internally start from minimal context or emit explicit bypass metadata
- [ ] MCP prompts, tool descriptions, README, and installed AGENTS instructions agree
- [ ] graph/context responses expose enough metadata to justify escalation
- [ ] tests protect contract wording and escalation metadata

---

## Graph Store Corruption Recovery Patch

Atlas can detect SQLite integrity failures, orphan nodes, dangling edges, stale graph state, and interrupted builds, but the operational policy for a damaged `.atlas/worldtree.db` is not explicit enough. Detection should lead to one clear outcome: quarantine unusable graph data, rebuild from repository source, and block graph-backed answers while stored graph facts are unsafe.

### Patch C1 — Graph DB corruption classification

- [ ] define graph-store health classes:
  - [ ] `healthy`
  - [ ] `stale`
  - [ ] `interrupted_build`
  - [ ] `failed_build`
  - [ ] `sqlite_corrupt`
  - [ ] `schema_mismatch`
  - [ ] `logical_inconsistency`
- [ ] classify evidence sources consistently:
  - [ ] `Store::open` errors
  - [ ] `PRAGMA integrity_check`
  - [ ] `PRAGMA foreign_key_check`
  - [ ] orphan-node scan
  - [ ] dangling-edge scan
  - [ ] graph build lifecycle state
  - [ ] freshness check against changed graph-relevant files
- [ ] ensure CLI and MCP use the same classification and `error_code` values
- [ ] add tests for each health class and error-code mapping

Why:
- makes corruption versus stale data explicit
- avoids treating dangling/orphan graph rows as a generic diagnostics warning

### Patch C2 — Quarantine and rebuild policy for `worldtree.db`

- [ ] define no partial salvage for graph DB corruption unless a future task explicitly adds verified salvage
- [ ] define recovery modes:
  - [ ] `manual_rebuild_required` — diagnostics report command; operator runs rebuild
  - [ ] `auto_quarantine_and_rebuild` — Atlas quarantines DB and rebuilds when command policy allows
  - [ ] `block_only` — graph-backed tools refuse answers but do not mutate DB
- [ ] define default recovery mode per entry point:
  - [ ] `status` / `doctor` / `db_check`: `block_only` diagnostics, no mutation
  - [ ] explicit `build` / `update`: `auto_quarantine_and_rebuild` when corruption is detected
  - [ ] graph-backed query/context/analyze tools: `block_only` with rebuild command
- [ ] require explicit flag for automatic quarantine outside build/update commands
- [ ] quarantine physically corrupt or logically inconsistent `.atlas/worldtree.db` before rebuilding
- [ ] use deterministic quarantine path with timestamp or collision-safe suffix
- [ ] keep quarantined DB for inspection instead of deleting it
- [ ] create fresh `worldtree.db` from migrations after quarantine
- [ ] run full graph rebuild from repository source after quarantine
- [ ] record rebuild result in graph build lifecycle state
- [ ] surface quarantine path, rebuild result, and failure reason in CLI JSON output
- [ ] surface same fields in MCP `build_or_update_graph`, `status`, `doctor`, and `db_check` where relevant
- [ ] add tests:
  - [ ] corrupt SQLite file is quarantined
  - [ ] logical dangling-edge inconsistency triggers rebuild policy
  - [ ] rebuild after quarantine creates usable fresh graph DB
  - [ ] failed rebuild leaves graph unavailable with actionable error

Why:
- graph data is derived from repo source, so clean rebuild is safer than partial salvage
- quarantine preserves evidence without serving unsafe facts

### Patch C3 — Block unsafe graph-backed answers

- [ ] block graph-backed query/context tools when health class is `sqlite_corrupt`, `schema_mismatch`, or `logical_inconsistency`
- [ ] return machine-readable failure with:
  - [ ] `error_code`
  - [ ] `health_class`
  - [ ] `db_path`
  - [ ] `quarantine_path` when available
  - [ ] recommended rebuild command
- [ ] allow non-graph diagnostics tools to keep working:
  - [ ] `status`
  - [ ] `doctor`
  - [ ] `db_check`
  - [ ] `debug_graph` only when DB can open safely
- [ ] distinguish stale-but-queryable graph state from corrupt-and-blocked graph state
- [ ] document agent behavior: do not answer from graph facts when corrupt/inconsistent
- [ ] add MCP tests that graph-backed tools fail closed on corrupt/inconsistent DB

Why:
- prevents confident answers from known-bad graph rows
- keeps diagnostics available while blocking unsafe context

### Patch C completion criteria

- [ ] graph DB health classes are explicit and shared by CLI/MCP
- [ ] corrupt graph execution state maps to block + quarantine + rebuild behavior
- [ ] auto rebuild, manual rebuild, and block-only recovery modes are explicit per command/tool
- [ ] corrupt or logically inconsistent `worldtree.db` is quarantined before rebuild
- [ ] rebuild from source is default policy; partial salvage is explicitly out of scope
- [ ] graph-backed tools fail closed when graph facts are corrupt or inconsistent
- [ ] diagnostics expose exact reason, quarantine path, and next command
- [ ] tests cover physical corruption, logical inconsistency, rebuild success, rebuild failure, and fail-closed query behavior

---

## Repo-Scoped MCP Singleton Patch

Atlas currently exposes MCP over stdio, which means each client session owns its own server process even when multiple clients target the same repo and the same `.atlas/worldtree.db`. Add one backend instance per canonical repo root plus DB path, with `atlas serve` acting as stdio broker that attaches to or starts that backend. This preserves current client config shape while preventing duplicate MCP server spawns and redundant runtime state.

### Patch M1 — Repo-scoped singleton identity and coordination

- [x] define singleton identity as canonical repo root plus canonical DB path
- [x] reuse canonical path rules from `atlas-repo` instead of adding local normalization helpers
- [x] define repo-local coordination artifacts under `.atlas/`:
  - [x] `mcp.instance.lock`
  - [x] `mcp.instance.json`
  - [x] `mcp.sock` on Unix
- [x] define metadata fields:
  - [x] `repo_root`
  - [x] `db_path`
  - [x] `socket_path`
  - [x] `pid`
  - [x] `protocol_version`
  - [x] `started_at`
- [x] define stale-instance cleanup rules for dead pid, missing socket, and mismatched repo or DB
- [x] add unit tests for identity derivation, metadata parsing, and stale cleanup decisions

Why:
- one MCP backend must be keyed by exact repo-plus-DB identity, not cwd or repo name
- lock and metadata contract must be explicit before broker or daemon work starts

### Patch M2 — Shared serving core and daemon transport

- [x] extract transport-agnostic request-serving core from MCP stdio transport
- [x] preserve current JSON-RPC behavior, worker-pool semantics, timeouts, and tool outputs
- [x] add daemon transport over Unix domain socket for Linux
- [x] define broker-to-daemon readiness handshake that validates:
  - [x] protocol compatibility
  - [x] `repo_root` match
  - [x] `db_path` match
- [x] keep initial daemon DB lifecycle conservative:
  - [x] allow current per-request `Store::open` path inside daemon
  - [x] defer connection pooling unless profiling proves need
- [x] add transport tests proving stdio and socket paths return identical responses for `initialize`, `tools/list`, `query_graph`, and `get_context`

Why:
- spawn dedupe alone is not enough; clients need attachable long-lived backend transport
- transport refactor must not change MCP-visible behavior

### Patch M3 — Stdio broker attach-or-spawn path

- [x] change `atlas serve` from direct stdio server to stdio broker or proxy
- [x] under exclusive lock:
  - [x] inspect existing metadata
  - [x] validate live daemon
  - [x] attach when healthy
  - [x] clean stale state and spawn when unhealthy or absent
- [x] relay stdin and stdout traffic between MCP client and daemon socket
- [x] fail closed with clear error when readiness handshake fails
- [x] ensure same repo plus same DB starts only one daemon even under concurrent `atlas serve` invocations
- [x] ensure same repo plus different DB paths may start separate daemon instances
- [x] add integration tests:
  - [x] two concurrent brokers for same repo plus DB create exactly one daemon
  - [x] same repo with different DB paths creates separate daemons
  - [x] stale socket or dead pid recovers on next attach attempt

Why:
- current clients still expect stdio, so broker compatibility is required for Copilot, Codex, and Claude installs
- attach-or-spawn behavior is core feature, not optional polish

### Patch M4 — Install, diagnostics, and behavior guarantees

- [x] keep generated Copilot, Claude, and Codex config shape unchanged:
  - [x] `type` remains `stdio`
  - [x] `command` remains `atlas`
  - [x] `args` still route through `atlas serve`
- [x] update install tests to confirm config shape stays stable
- [x] add concise diagnostics for attach-versus-spawn outcome, socket path, and stale cleanup
- [x] audit session, saved-context, and adapter flows so daemon mode preserves existing semantics
- [x] update README and MCP/setup docs to describe:
  - [x] repo-scoped singleton behavior
  - [x] repo-local coordination artifacts
  - [x] Linux-first Unix socket support
  - [x] stale-instance recovery behavior
  - [x] unchanged client config surface
- [x] add CLI quality-gate coverage for singleton serve behavior

Why:
- rollout should be transparent to existing MCP clients and install surfaces
- diagnostics and docs are necessary for debugging duplicate spawn or wrong-instance attachment

### Patch M completion criteria

- [x] one MCP backend instance exists per canonical repo root plus canonical DB path
- [x] `atlas serve` remains stdio-compatible for existing clients
- [x] broker attaches to running daemon or starts one under lock
- [x] same repo plus same DB cannot spawn duplicate daemons under race
- [x] same repo plus different DB paths can run separate daemons
- [x] daemon and stdio paths preserve identical MCP tool behavior
- [x] install outputs remain compatible with current Copilot, Claude, and Codex configs
- [x] tests cover identity, transport parity, concurrent attach-or-spawn, stale recovery, and install stability

---
