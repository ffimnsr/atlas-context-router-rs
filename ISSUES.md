# TODO — Atlas Core Code Graph Engine in Rust

## Goal

Build the core local engine first:

- [ ] Scan a Git repository
- [ ] Detect changed files using Git diff
- [ ] Parse source files into symbols and relationships
- [ ] Index parsed code into SQLite
- [ ] Support fast local search using SQLite FTS5 + BM25
- [ ] Prepare for later impact analysis and review-context generation

## Non-Core for Later

- [ ] MCP server
- [ ] Visualization
- [ ] Embeddings / vector search
- [ ] Community detection
- [ ] Flow tracing
- [ ] Multi-repo registry
- [ ] Github Copilot / VSCode Copilot Chat / ChatGPT Codex install hooks
- [ ] Refactoring tools
- [ ] Export formats

---

## Phase 1 — Workspace Skeleton

### 1.1 Set up the Rust workspace

- [ ] Keep the workspace root with:
  - [ ] `packages/atlas-cli`
  - [ ] `packages/atlas-engine`
- [ ] Wire `atlas-cli` to depend on `atlas-engine`
- [ ] Add initial dependencies for:
  - [ ] CLI argument parsing
  - [ ] SQLite access
  - [ ] Git interaction
  - [ ] Serialization
  - [ ] Error handling
- [ ] Confirm the chosen SQLite crate supports FTS5
- [ ] Decide whether to use bundled SQLite for consistent FTS5 support
- [ ] Add initial CLI commands:
  - [ ] `atlas init`
  - [ ] `atlas scan`
  - [ ] `atlas update`
  - [ ] `atlas search`
  - [ ] `atlas status`

### 1.2 Create Rust module structure

- [ ] `packages/atlas-cli/src/main.rs`
- [ ] `packages/atlas-engine/src/lib.rs`
- [ ] `packages/atlas-engine/src/config.rs`
- [ ] `packages/atlas-engine/src/core.rs`
- [ ] `packages/atlas-engine/src/repo.rs`
- [ ] `packages/atlas-engine/src/gitdiff.rs`
- [ ] `packages/atlas-engine/src/scanner.rs`
- [ ] `packages/atlas-engine/src/parser/mod.rs`
- [ ] `packages/atlas-engine/src/parser/rust.rs`
- [ ] `packages/atlas-engine/src/store/mod.rs`
- [ ] `packages/atlas-engine/src/store/sqlite.rs`
- [ ] `packages/atlas-engine/src/search.rs`
- [ ] `packages/atlas-engine/src/impact.rs`

---

## Phase 2 — Core Types

### 2.1 Define file record

- [ ] Create `FileRecord`
- [ ] Include:
  - [ ] `path`
  - [ ] `language`
  - [ ] `hash`
  - [ ] `size`
  - [ ] `indexed_at`

### 2.2 Define node

- [ ] Create `Node`
- [ ] Include:
  - [ ] `id`
  - [ ] `kind`
  - [ ] `name`
  - [ ] `qualified_name`
  - [ ] `file_path`
  - [ ] `language`
  - [ ] `line_start`
  - [ ] `line_end`
  - [ ] `parent_name`
  - [ ] `signature`
  - [ ] `code`
  - [ ] `file_hash`
  - [ ] `extra_json`

### 2.3 Define edge

- [ ] Create `Edge`
- [ ] Include:
  - [ ] `id`
  - [ ] `kind`
  - [ ] `source_qualified`
  - [ ] `target_qualified`
  - [ ] `file_path`
  - [ ] `line`
  - [ ] `confidence`
  - [ ] `extra_json`

### 2.4 Define node kinds

- [ ] Create `NodeKind` enum
- [ ] Add variants for:
  - [ ] `File`
  - [ ] `Module`
  - [ ] `Import`
  - [ ] `Struct`
  - [ ] `Enum`
  - [ ] `Trait`
  - [ ] `Function`
  - [ ] `Method`
  - [ ] `Variable`
  - [ ] `Constant`
  - [ ] `Test`

### 2.5 Define edge kinds

- [ ] Create `EdgeKind` enum
- [ ] Add variants for:
  - [ ] `Contains`
  - [ ] `Imports`
  - [ ] `Calls`
  - [ ] `Defines`
  - [ ] `Implements`
  - [ ] `Extends`
  - [ ] `Tests`
  - [ ] `References`

### 2.6 Serialization and database mapping

- [ ] Decide which types derive `Debug`, `Clone`, `Serialize`, and `Deserialize`
- [ ] Decide whether `NodeKind` and `EdgeKind` are stored as strings or integers
- [ ] Add conversion helpers between SQLite rows and Rust domain types

---

## Phase 3 — SQLite Store

### 3.1 Database location

- [ ] Use default DB path: `.code-review-graph/codegraph.sqlite`
- [ ] Create `.code-review-graph` on `atlas init`
- [ ] Allow custom DB path through config or CLI flag

### 3.2 Schema

- [ ] Create `metadata` table
- [ ] Create `files` table
- [ ] Create `nodes` table
- [ ] Create `edges` table

### 3.3 Indexes

- [ ] Add indexes for:
  - [ ] `nodes.kind`
  - [ ] `nodes.file_path`
  - [ ] `nodes.qualified_name`
  - [ ] `nodes.language`
  - [ ] `edges.kind`
  - [ ] `edges.source_qualified`
  - [ ] `edges.target_qualified`
  - [ ] `edges.file_path`

### 3.4 SQLite pragmas

- [ ] Enable WAL
- [ ] Enable foreign keys
- [ ] Set busy timeout

### 3.5 Store API

- [ ] Implement `open`
- [ ] Implement `migrate`
- [ ] Implement `replace_file_graph`
- [ ] Implement `delete_file_graph`
- [ ] Implement `get_stats`
- [ ] Implement `get_nodes_by_file`
- [ ] Implement `get_edges_by_file`

### 3.6 Transaction behavior

`replace_file_graph` should do this in one transaction:

- [ ] Delete old FTS rows for file nodes
- [ ] Delete old nodes for file
- [ ] Delete old edges for file
- [ ] Upsert file record
- [ ] Insert new nodes
- [ ] Insert new edges
- [ ] Insert new FTS rows
- [ ] Commit

---

## Phase 4 — SQLite FTS5 Search

### 4.1 Create FTS table

- [ ] Create `nodes_fts`
- [ ] Index:
  - [ ] `qualified_name`
  - [ ] `name`
  - [ ] `kind`
  - [ ] `file_path`
  - [ ] `language`
  - [ ] `signature`
  - [ ] `code`

### 4.2 FTS sync strategy

- [ ] Use manual sync first
- [ ] Insert into FTS on node insert
- [ ] Remove old FTS rows on file replacement
- [ ] Add `rebuild_fts` for recovery

### 4.3 BM25 search

- [ ] Create `SearchResult`
- [ ] Join FTS rows back to `nodes`
- [ ] Order by BM25 score
- [ ] Limit results
- [ ] Remember: lower BM25 score is better in SQLite FTS5

### 4.4 Search filters

- [ ] Add `--limit`
- [ ] Add `--kind`
- [ ] Add `--language`
- [ ] Add `--file`

---

## Phase 5 — Repository and Diff Support

### 5.1 Repository scanning

- [ ] Walk the repository from a chosen root
- [ ] Respect `.gitignore`
- [ ] Skip generated or unsupported files
- [ ] Detect language from extension and file content when needed

### 5.2 Git diff support

- [ ] Detect changed files relative to:
  - [ ] working tree
  - [ ] staged changes
  - [ ] a base revision
- [ ] Normalize paths relative to repo root
- [ ] Handle file adds, deletes, renames, and modifications

---

## Phase 6 — Parsing

### 6.1 Parser interface

- [ ] Define a parser trait or equivalent abstraction
- [ ] Return parsed nodes and edges for a single file
- [ ] Return structured parse errors without aborting the whole scan

### 6.2 Rust parser

- [ ] Parse Rust modules
- [ ] Parse `use` imports
- [ ] Parse structs
- [ ] Parse enums
- [ ] Parse traits
- [ ] Parse functions
- [ ] Parse impl blocks and methods
- [ ] Parse constants and statics
- [ ] Parse tests
- [ ] Capture symbol spans and qualified names

### 6.3 Relationships

- [ ] Emit `contains` edges
- [ ] Emit `imports` edges
- [ ] Emit `defines` edges
- [ ] Emit `calls` edges where practical
- [ ] Emit `implements` edges for trait impls
- [ ] Emit `references` edges for later enrichment

---

## Phase 7 — CLI and UX

### 7.1 CLI behavior

- [ ] `atlas init` creates config and database
- [ ] `atlas scan` performs a full repository scan
- [ ] `atlas update` processes only changed files when possible
- [ ] `atlas search` queries indexed symbols
- [ ] `atlas status` prints index metadata and counts

### 7.2 Output

- [ ] Add human-readable output by default
- [ ] Add JSON output for scripting
- [ ] Define exit codes for common failure cases

---

## Phase 8 — Quality

### 8.1 Tests

- [ ] Add unit tests for core types
- [ ] Add schema migration tests
- [ ] Add store transaction tests
- [ ] Add parser fixture tests
- [ ] Add CLI smoke tests

### 8.2 Error handling and logging

- [ ] Standardize error types across crates
- [ ] Add contextual error messages
- [ ] Add debug logging for scan and index operations

### 8.3 Performance

- [ ] Measure scan time on a medium-size Rust repository
- [ ] Measure search latency
- [ ] Avoid unnecessary file reparsing during incremental updates
