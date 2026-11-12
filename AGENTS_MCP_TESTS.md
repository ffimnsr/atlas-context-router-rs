# Agent MCP Testing Guide: Atlas Context Router (`AGENTS_MCP_TESTS.md`)

> **Authoritative Test Setup & Concrete Test Suite for AI Agents and Harnesses Interacting with Atlas MCP Server**  
> Target Workspace: `/home/pastel/Projects/atlas-context-router-rs`  
> Tools Covered: **80 MCP Tools** | Test Cases: **244 Real Concrete Tests**  

---

## 1. Overview & Setup in Plain English

Atlas is an offline-first code intelligence, graph navigation, and bounded context router. Instead of forcing AI agents to browse large codebases via brute-force file scanning and grep, Atlas parses source files into a semantic directed graph stored in `.atlas/worldtree.db` (functions, structs, enums, files, modules, and edges like calls, imports, defines, contains, references) and maintains an event continuity ledger in `.atlas/context.db`.

When agents interact with Atlas over the **Model Context Protocol (MCP)**, the most common pitfall is **noisy, bloated, or ambiguous responses** that blow past context token budgets or confuse the agent's reasoning. This document provides a complete testing framework written in plain English, with **3 to 4 concrete, noise-free tests for every single MCP tool** supported by Atlas, using real symbols, functions, and files found in this repository.

---

## 2. Test Setup & Operating Environment

### 2.1 Workspace & File System Layout
- **Repository Root**: `/home/pastel/Projects/atlas-context-router-rs`
- **Graph Database**: `.atlas/worldtree.db` (SQLite relational graph of nodes, edges, files, and AST properties)
- **Session & Content Database**: `.atlas/context.db` (continuity ledger, memoir graphs, decision memory, saved context artifacts)
- **Configuration File**: `.atlas/config.toml` (retrieval settings, embedding endpoints, token budgets)

### 2.2 Server Launch & Transport Modes
Agents can run test suites against Atlas using either of two transports:
1. **Stdio Transport (Standard CLI/Agent Hook)**:
   ```bash
   cargo run -p atlas-mcp --bin atlas -- mcp
   ```
   Or using the pre-compiled binary: `./target/debug/atlas mcp`.
2. **Stateless HTTP Transport (Microservice / Multi-Agent Gateway)**:
   ```bash
   cargo run -p atlas-mcp --bin atlas -- mcp --transport http --port 3000
   ```
   Requests are sent as `POST /mcp` with protocol headers (`params._meta` with protocol versions `2026-07-28` or `2025-11-25`).

### 2.3 Diagnostic Prequalification Sequence
Before executing functional tests, every agent harness must run this 3-step sequence:
1. `broker_status`: Verify the broker process is responsive and worker threads are healthy.
2. `status`: Check if the code graph is ready (`graph_state.graph_built == true`). If `stale_index == true`, call `update_graph` with `change_source: { kind: 'working_tree' }`.
3. `wake_up`: Prime the session continuity ledger and inspect past resume snapshots.

---

## 3. The 5 Golden Rules for Clean, Accurate, Non-Noisy MCP Data

To ensure that MCP responses never flood agent context windows with useless tokens:

1. **Use Exact Fully Qualified Names (`qn`) over Natural Language Phrases**:
   - *Bad (Noisy)*: `query_graph(text="where does error classification happen?")` (triggers retry guidance).
   - *Good (Clean)*: `resolve_symbol(name="classify_graph_store_error")` -> yields `packages/atlas-core/src/health.rs::fn::classify_graph_store_error` -> pass directly to `symbol_neighbors`, `traverse_graph`, or `trace_calls`.
2. **Always Enforce Strict Bounding Limits (`limit`, `max_depth`, `max_nodes`)**:
   - Core types like `Store` or `Result` have thousands of transitive edges. Never run unbounded traversals.
   - Set `max_depth: 2` to `3` and `max_nodes: 10` to `30` in graph traversal and trace tools.
3. **Scope Searches with `subpath`, `file`, and `kind` Constraints**:
   - When listing symbols or searching content, specify `subpath: "packages/atlas-core/src"` and `kind: "function"` to eliminate synthetic files, fuzz fixtures, and test mocks.
4. **Assert on Stable Structured Content (`structuredContent`)**:
   - Atlas MCP tools return the `stable-object` contract. Test harnesses must validate exact JSON properties (`best_match.qualified_name`, `summary.caller_count`, `impacted_files`) rather than parsing raw text strings.
5. **Strictly Separate Code Graph Tools from Companion Content Search Tools**:
   - Use `query_graph`, `symbol_neighbors`, `traverse_graph`, `trace_calls` for code symbols and call hierarchies.
   - Use `search_files`, `search_content`, `read_file_excerpt`, `get_docs_section` for Markdown documentation, TOML manifests, and YAML workflows.

---

## 4. MCP Tools Test Suites (80 Tools, 244 Concrete Tests)

### Category: Health, Diagnostics & Broker Infrastructure

#### Tool 1: `broker_status`
**Description**: Returns a lightweight liveness and health check for the MCP broker process itself, reporting PID, uptime, server version, and worker threads.

##### Test 1.1: Verify Broker Process Liveness and Threading
**Intent in Plain English**: Verify that the background broker process is running, responsive, and configured with healthy worker thread pools before initiating graph operations.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Check `structuredContent.status == 'ok'`
- Check `structuredContent.worker_threads >= 1`
- Check `structuredContent.uptime_seconds >= 0`
- Verify `structuredContent.server_version` matches the semantic version string in `Cargo.toml`

##### Test 1.2: Stateless Broker Readiness Verification
**Intent in Plain English**: Ensure the broker health probe succeeds independently of whether the code graph database is currently locked, stale, or being updated.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify the call completes synchronously under 50ms without initiating disk I/O on `worldtree.db`
- Confirm `structuredContent.pid > 0` and process ID is valid

##### Test 1.3: Post-Stress Broker Liveness Confirmation
**Intent in Plain English**: Re-check broker status following heavy batch operations to confirm no worker thread pool exhaustion or deadlocks occurred.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.worker_threads` remains intact
- Confirm no error payload is returned in `structuredContent`

---

#### Tool 2: `status`
**Description**: Returns the core graph health summary: database existence, node/edge counts, build state, retrieval index state, and stale index warnings.

##### Test 2.1: Verify Graph Index Integrity and Symbol Capacity
**Intent in Plain English**: Confirm the graph database (`.atlas/worldtree.db`) exists, is openable, and contains the expected baseline volume of parsed symbols.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Check `structuredContent.db_state.open_ok == true`
- Check `structuredContent.node_count > 20000` (actual parsed nodes on this repo: ~25,279)
- Check `structuredContent.edge_count > 80000` (actual parsed edges: ~100,038)
- Verify `structuredContent.safe_for_symbol_lookup == true`

##### Test 2.2: Detect Pending Working-Tree Changes (Stale Index Detection)
**Intent in Plain English**: Verify whether unindexed git changes exist in working directory and inspect the machine-readable failure category.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Inspect `structuredContent.graph_state.stale_index` boolean flag
- If stale, verify `structuredContent.failure_category == 'stale_index'` and `graph_state.pending_graph_changes` lists modified files (e.g. `packages/atlas-mcp/src/tools/dispatch.rs`)
- Verify `summary.mcp_suggestions` provides the exact remedy: `update_graph with change_source=working_tree`

##### Test 2.3: Inspect Companion Retrieval Content Index
**Intent in Plain English**: Verify that the companion content database (`.atlas/context.db`) is available and indexed for textual searches.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Check `structuredContent.retrieval_index.available == true`
- Check `structuredContent.retrieval_index.searchable == true`
- Verify `retrieval_index.content_db_path` ends with `.atlas/context.db`

---

#### Tool 3: `doctor`
**Description**: Performs comprehensive repository health checks: git root detection, .atlas directory, config file, DB integrity, graph build state, and path canonicality.

##### Test 3.1: Full Repository Pre-Flight Health Suite
**Intent in Plain English**: Run all system checks prior to relying on graph facts to guarantee zero corrupted rows or misconfigured paths.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.checks` array contains passes for `git_root`, `atlas_dir`, `db_open`, and `graph_built`
- Check that every check object has `pass == true` or structured warnings
- Verify overall health outcome is reported without unhandled exceptions

##### Test 3.2: Path Identity Invariant Validation
**Intent in Plain English**: Ensure that all indexed file and symbol paths follow canonical repo-relative naming with zero Windows/Linux path format drift.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Check `structuredContent.checks` where `check == 'path_canonicality'`
- Verify `noncanonical_path_rows == 0`
- Ensure no non-canonical path rebuild warnings are emitted

##### Test 3.3: Database Schema and Invariant Consistency Check
**Intent in Plain English**: Confirm table schemas match the expected migration versions for `nodes`, `edges`, and `files`.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Check `checks` entry for `schema_version` passes
- Confirm no `schema_mismatch` error code is emitted

---

#### Tool 4: `db_check`
**Description**: Executes SQLite PRAGMA integrity_check, scans for orphan nodes and dangling edges, and validates session continuity tables.

##### Test 4.1: Execute SQLite Low-Level PRAGMA Integrity Check
**Intent in Plain English**: Validate that the SQLite database file on disk has zero corrupted B-trees, invalid pages, or broken indices.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.ok == true`
- Verify `structuredContent.sqlite_integrity == 'ok'`

##### Test 4.2: Bounded Scan for Structural Orphan Nodes
**Intent in Plain English**: Check for orphan nodes that have no inbound or outbound edges, bounded by a small limit to keep output non-noisy.

**MCP Arguments (JSON)**:
```json
{
  "limit": 10
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.orphan_count` is returned as an integer
- Verify `structuredContent.orphan_samples` array length is <= 10

##### Test 4.3: Validate Dangling Edge Integrity & Memories Schema
**Intent in Plain English**: Confirm all edge endpoints point to valid existing node IDs and the session `memories` table schema is intact.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.dangling_edge_count == 0`
- Verify `structuredContent.memories_schema_valid == true`

---

#### Tool 5: `debug_graph`
**Description**: Returns internal graph diagnostics: node/edge counts by kind, top files by node count, orphan nodes, and dangling edges.

##### Test 5.1: Inspect Breakdown of Node Kinds
**Intent in Plain English**: Verify that the parser has classified symbols into expected kinds (function, struct, enum, trait, file, module).

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.nodes_by_kind.function > 3000`
- Verify `structuredContent.nodes_by_kind.struct > 1000`
- Verify `structuredContent.nodes_by_kind.file > 900`

##### Test 5.2: Inspect Edge Breakdown by Relationship Kind
**Intent in Plain English**: Verify that the graph persistence layer captured relational edges across the codebase.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.edges_by_kind.calls > 20000`
- Verify `structuredContent.edges_by_kind.contains > 15000`
- Verify `structuredContent.edges_by_kind.references > 10000`

##### Test 5.3: Bounded Top Files by Symbol Density
**Intent in Plain English**: Identify the top 5 largest or most symbol-dense files in the repository to target for refactoring modularity.

**MCP Arguments (JSON)**:
```json
{
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.top_files` array length == 5
- Verify each item in `top_files` has `file_path` and `node_count`

---

#### Tool 6: `get_metrics`
**Description**: Returns in-process runtime metrics for Atlas: counter and histogram data for build/update duration, tree-cache reuse ratio, query latency, and MCP tool call counts.

##### Test 6.1: Inspect Tree-Cache Reuse Ratio
**Intent in Plain English**: Verify that the incremental parser tree cache is active and measuring AST subtree reuse across invocations.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.metrics` contains `parser_tree_cache_hit_ratio` or counter families
- Verify value is between 0.0 and 1.0

##### Test 6.2: Verify MCP Tool Invocations Telemetry Counters
**Intent in Plain English**: Confirm that tool invocation counters correctly record agent activity without memory leaks.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.tool_call_counts` exists
- Verify counter for `status` or `get_metrics` is >= 1

##### Test 6.3: Query Latency Distribution Check
**Intent in Plain English**: Inspect FTS and traversal query latency metrics to confirm sub-millisecond graph query responsiveness.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify histogram data or average duration exists under `query_latency`
- Confirm no anomalous timeout counters are incremented

---

### Category: Symbol Search & Index Exploration

#### Tool 7: `list_symbols`
**Description**: Deterministic paginated symbol inventory for the repo. Filter by kind, language, file-path prefix, or repo id with offset/limit.

##### Test 7.1: Enumerate Rust Functions in Core Package with Bounded Limit
**Intent in Plain English**: List real functions strictly within `packages/atlas-core/src` using small page sizes to prevent context overload.

**MCP Arguments (JSON)**:
```json
{
  "subpath": "packages/atlas-core/src",
  "kind": "function",
  "language": "rust",
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.returned == 5`
- Verify all items in `symbols` have `kind == 'function'` and `language == 'rust'`
- Verify `symbols[0].file` starts with `packages/atlas-core/src/`
- Check that `next_offset == 5` and `has_more == true`

##### Test 7.2: Enumerate Struct Types in MCP Package
**Intent in Plain English**: Retrieve declared structs in `packages/atlas-mcp/src` to inspect data transfer schemas.

**MCP Arguments (JSON)**:
```json
{
  "subpath": "packages/atlas-mcp/src",
  "kind": "struct",
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify every returned symbol has `kind == 'struct'`
- Verify `symbols` have valid `line_start` and `qualified_name`

##### Test 7.3: Verify Deterministic Pagination Continuity
**Intent in Plain English**: Ensure paging with `offset` produces an uninterrupted, non-overlapping stream of symbols.

**MCP Arguments (JSON)**:
```json
{
  "subpath": "packages/atlas-core/src",
  "kind": "function",
  "limit": 5,
  "offset": 5
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.offset == 5`
- Verify returned symbols differ from the first page (offset 0)
- Verify stable ordering by file path and line number

##### Test 7.4: Discover Package Metadata Symbols
**Intent in Plain English**: Query synthetic package symbols generated from workspace `Cargo.toml` files.

**MCP Arguments (JSON)**:
```json
{
  "kind": "package",
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify returns packages like `atlas-core`, `atlas-mcp`, `atlas-cli`
- Verify `qualified_name` follows format `package::cargo:...`

---

#### Tool 8: `resolve_symbol`
**Description**: Resolves a symbol name or qualified name to its canonical graph symbol. Resolves ambiguity and provides follow-up traversal suggestions.

##### Test 8.1: Resolve Exact Qualified Name with Zero Ambiguity
**Intent in Plain English**: Resolve `packages/atlas-core/src/health.rs::fn::classify_graph_store_error` directly to verify instant exact resolution.

**MCP Arguments (JSON)**:
```json
{
  "name": "packages/atlas-core/src/health.rs::fn::classify_graph_store_error"
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.summary.status == 'resolved'`
- Verify `structuredContent.ambiguity.ambiguous == false`
- Verify `structuredContent.best_match.name == 'classify_graph_store_error'`
- Verify `best_match.file_path == 'packages/atlas-core/src/health.rs'`
- Verify `suggestions[0].next_tools` lists `symbol_neighbors` and `traverse_graph`

##### Test 8.2: Disambiguate Short Symbol Identifier with File & Kind Filter
**Intent in Plain English**: Resolve `classify_graph_store_error` while providing file and kind filters to avoid false matches in test files.

**MCP Arguments (JSON)**:
```json
{
  "name": "classify_graph_store_error",
  "file": "packages/atlas-core/src/health.rs",
  "kind": "function"
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.best_match.qualified_name == 'packages/atlas-core/src/health.rs::fn::classify_graph_store_error'`
- Verify `best_match.kind == 'function'`

##### Test 8.3: Normalize Public Kind Alias 'fn' to 'function'
**Intent in Plain English**: Verify that user/agent alias `kind: 'fn'` is correctly mapped to internal `function` token.

**MCP Arguments (JSON)**:
```json
{
  "name": "budget_stage_rules",
  "kind": "fn"
}
```

**Expected Clean Output & Assertions**:
- Verify resolves to `packages/atlas-core/src/budget.rs::fn::budget_stage_rules`
- Verify `best_match.kind == 'function'`

##### Test 8.4: Graceful Handling of Non-Existent Symbol
**Intent in Plain English**: Verify that looking up a nonexistent symbol returns zero matches cleanly without throwing an error.

**MCP Arguments (JSON)**:
```json
{
  "name": "non_existent_symbol_in_repo_12345"
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.summary.match_count == 0`
- Verify `structuredContent.best_match` is null
- Verify `structuredContent.warnings` is empty or informative

---

#### Tool 9: `query_graph`
**Description**: Full-text searches the code graph by symbol name, qualified name, or supported intent phrases ('who calls', 'what breaks', 'tests for').

##### Test 9.1: Exact Symbol Full-Text Search (Symbol-Centric)
**Intent in Plain English**: Query symbol `classify_graph_store_error` with `include_files=false` to return exact symbol nodes and zero file node clutter.

**MCP Arguments (JSON)**:
```json
{
  "text": "classify_graph_store_error",
  "include_files": false,
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.matches` contains the function node
- Verify no node in `matches` has `kind == 'file'`
- Check `structuredContent.summary.total_matches >= 1`

##### Test 9.2: Execute 'who calls <symbol>' Intent Phrase
**Intent in Plain English**: Verify that the Atlas query grammar parses the intent prefix and resolves caller references.

**MCP Arguments (JSON)**:
```json
{
  "text": "who calls classify_graph_store_error",
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify the query does not fail with 'natural language rejected'
- Verify `matches` contains callers such as `packages/atlas-core/src/readiness.rs`

##### Test 9.3: Scoped Regex Query Inside Subpath
**Intent in Plain English**: Perform a structural regex pattern search across symbol names restricted to `packages/atlas-core/src`.

**MCP Arguments (JSON)**:
```json
{
  "regex": "classify_[a-z_]+_error",
  "subpath": "packages/atlas-core/src",
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify returned symbols match regex pattern
- Verify all matches reside inside `packages/atlas-core/src`

##### Test 9.4: Reject Vague Natural Language Queries with Clean Guidance
**Intent in Plain English**: Ensure unstructured natural language queries are rejected early with structured guidance to use concrete identifiers.

**MCP Arguments (JSON)**:
```json
{
  "text": "where is the place that handles error codes?"
}
```

**Expected Clean Output & Assertions**:
- Verify tool returns an error or structured guidance
- Verify suggestions indicate using a concrete symbol identifier

---

#### Tool 10: `batch_query_graph`
**Description**: Runs multiple query_graph searches in a single round-trip, returning structuredContent with per-item results and summaries.

##### Test 10.1: Multi-Symbol Batch Lookup
**Intent in Plain English**: Look up two independent functions in a single round trip to reduce network and broker overhead.

**MCP Arguments (JSON)**:
```json
{
  "items": [
    {
      "text": "classify_graph_store_error",
      "limit": 3
    },
    {
      "text": "budget_stage_rules",
      "limit": 3
    }
  ]
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.results` array has length 2
- Verify `results[0].query` matches `classify_graph_store_error`
- Verify `results[1].query` matches `budget_stage_rules`
- Verify each item has independent `matches` array

##### Test 10.2: Per-Item Subpath Scoping in Batch
**Intent in Plain English**: Run batch queries where each item targets a distinct subpath in the repository.

**MCP Arguments (JSON)**:
```json
{
  "items": [
    {
      "text": "derive_integrity_state",
      "subpath": "packages/atlas-core/src"
    },
    {
      "text": "tool_graph_requirement",
      "subpath": "packages/atlas-mcp/src"
    }
  ]
}
```

**Expected Clean Output & Assertions**:
- Verify `results[0].matches[0].file_path` starts with `packages/atlas-core/src`
- Verify `results[1].matches[0].file_path` starts with `packages/atlas-mcp/src`

##### Test 10.3: Batch Enforcing Item Limit (Max 20 Items)
**Intent in Plain English**: Confirm the server enforces the 20-item cap to prevent resource exhaustion.

**MCP Arguments (JSON)**:
```json
{
  "items": [
    {
      "text": "item_0"
    },
    {
      "text": "item_1"
    },
    {
      "text": "item_2"
    },
    {
      "text": "item_3"
    },
    {
      "text": "item_4"
    },
    {
      "text": "item_5"
    },
    {
      "text": "item_6"
    },
    {
      "text": "item_7"
    },
    {
      "text": "item_8"
    },
    {
      "text": "item_9"
    },
    {
      "text": "item_10"
    },
    {
      "text": "item_11"
    },
    {
      "text": "item_12"
    },
    {
      "text": "item_13"
    },
    {
      "text": "item_14"
    },
    {
      "text": "item_15"
    },
    {
      "text": "item_16"
    },
    {
      "text": "item_17"
    },
    {
      "text": "item_18"
    },
    {
      "text": "item_19"
    },
    {
      "text": "item_20"
    }
  ]
}
```

**Expected Clean Output & Assertions**:
- Verify server returns a validation error detailing maximum items is 20

---

#### Tool 11: `explain_query`
**Description**: Explains how a query_graph invocation would be executed: tokenisation, FTS phrase construction, regex validation, and expected search path.

##### Test 11.1: Explain Tokenization for Identifier Query
**Intent in Plain English**: Pre-flight inspect how `classify_graph_store_error` is tokenized by SQLite FTS5.

**MCP Arguments (JSON)**:
```json
{
  "text": "classify_graph_store_error"
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.fts_phrase` is generated
- Verify `structuredContent.tokens` contains individual word tokens
- Verify expected search path indicates FTS index lookup

##### Test 11.2: Pre-Validate Regex Syntax Before Execution
**Intent in Plain English**: Test whether a regex string compiles cleanly against SQLite UDF regex engine.

**MCP Arguments (JSON)**:
```json
{
  "regex": "^(classify|format)_[a-z]+$"
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.regex_valid == true`
- Verify no regex compilation errors are reported

##### Test 11.3: Explain Intent Phrase Decomposition
**Intent in Plain English**: Inspect the internal AST produced for `what breaks classify_graph_store_error`.

**MCP Arguments (JSON)**:
```json
{
  "text": "what breaks classify_graph_store_error"
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.intent` is parsed as `what_breaks`
- Verify target symbol identifier is extracted as `classify_graph_store_error`

---

#### Tool 12: `list_graph_stats`
**Description**: Returns node/edge counts, edge kind breakdown, and language distribution for the indexed code graph.

##### Test 12.1: Verify Total Node and Edge Counts
**Intent in Plain English**: Check high-level graph volume metrics for whole repository.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.node_count > 20000`
- Verify `structuredContent.edge_count > 80000`
- Verify `structuredContent.file_count > 900`

##### Test 12.2: Inspect Programming Language Distribution
**Intent in Plain English**: Verify that Rust is the primary language with TOML and Markdown accurately accounted for.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.languages.rust.nodes > 15000`
- Verify `structuredContent.languages.toml` exists
- Verify `structuredContent.languages.markdown` exists

##### Test 12.3: Verify Relational Edge Kind Proportions
**Intent in Plain English**: Ensure critical semantic edge kinds (`calls`, `defines`, `imports`, `contains`) exist in healthy quantities.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.edge_kinds.calls > 20000`
- Verify `structuredContent.edge_kinds.contains > 15000`

---

### Category: Graph Traversal, Tracing & Structural Relationships

#### Tool 13: `symbol_neighbors`
**Description**: Returns the immediate graph neighbourhood of a symbol: callers, callees, call edge sites with source lines, test nodes, containment siblings, and import-linked nodes.

##### Test 13.1: Inspect Callers and Callees of Core Error Classifier
**Intent in Plain English**: Trace upward callers and downward callees of `classify_graph_store_error` with exact call site source lines.

**MCP Arguments (JSON)**:
```json
{
  "qname": "packages/atlas-core/src/health.rs::fn::classify_graph_store_error"
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.summary.caller_count == 2` (callers: `internal_error_code`, `derive_integrity_state`)
- Verify `structuredContent.summary.callee_count == 2` (callees: `is_schema_mismatch_error`, `looks_like_logical_inconsistency`)
- Verify `structuredContent.call_sites` array contains 4 entries with direction ('incoming' / 'outgoing') and source lines

##### Test 13.2: Inspect Containment Siblings in Same Module
**Intent in Plain English**: Retrieve sister functions, structs, and enums declared in the same file `packages/atlas-core/src/health.rs`.

**MCP Arguments (JSON)**:
```json
{
  "qname": "packages/atlas-core/src/health.rs::fn::classify_graph_store_error"
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.siblings` includes `GraphHealthInput` struct
- Verify `structuredContent.siblings` includes `GraphStoreHealthClass` enum
- Verify `structuredContent.siblings` includes `is_schema_mismatch_error` function

##### Test 13.3: Neighborhood of Leaf Utility Function
**Intent in Plain English**: Verify that a leaf function (`now_utc`) has zero callees and only callers.

**MCP Arguments (JSON)**:
```json
{
  "qname": "packages/atlas-core/src/clock.rs::fn::now_utc"
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.summary.callee_count == 0`
- Verify `structuredContent.summary.caller_count >= 1`

---

#### Tool 14: `traverse_graph`
**Description**: Bi-directional graph traversal from a specific symbol (qualified name), returning all nodes and edges reachable within depth hops, capped by budget.

##### Test 14.1: Bounded Bidirectional Traversal (Depth 2, Max Nodes 10)
**Intent in Plain English**: Traverse 2 hops outward from `classify_graph_store_error` while enforcing a strict 10-node limit to avoid blowing up LLM context.

**MCP Arguments (JSON)**:
```json
{
  "from_qn": "packages/atlas-core/src/health.rs::fn::classify_graph_store_error",
  "max_depth": 2,
  "max_nodes": 10
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.nodes.length <= 10`
- Verify `structuredContent.depth == 2`
- Verify `structuredContent.root_symbol == 'packages/atlas-core/src/health.rs::fn::classify_graph_store_error'`
- Verify `structuredContent.traversal_budget.accepted_node_budget == 10`

##### Test 14.2: Traverse Impacted Files Discovery
**Intent in Plain English**: Discover which external files are structurally connected within 2 hops of the health classifier.

**MCP Arguments (JSON)**:
```json
{
  "from_qn": "packages/atlas-core/src/health.rs::fn::classify_graph_store_error",
  "max_depth": 2,
  "max_nodes": 15
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.impacted_files` contains `packages/atlas-core/src/health.rs`
- Verify `structuredContent.impacted_files` contains `packages/atlas-core/src/readiness.rs`
- Verify `summary.impacted_file_count == 2`

##### Test 14.3: Depth-1 Shallow Neighborhood Expansion
**Intent in Plain English**: Execute single-hop immediate edge expansion to verify direct relational boundaries.

**MCP Arguments (JSON)**:
```json
{
  "from_qn": "packages/atlas-core/src/health.rs::fn::classify_graph_store_error",
  "max_depth": 1,
  "max_nodes": 5
}
```

**Expected Clean Output & Assertions**:
- Verify all emitted edges originate from or point directly to the root symbol
- Verify no transitive edges are emitted

---

#### Tool 15: `trace_calls`
**Description**: Directional call-chain trace from a symbol: follows callers (up), callees (down), or both, bounded by depth and node caps, and optionally finds shortest path to target.

##### Test 15.1: Trace Upward Callers (How Many Callers Lead to Symbol)
**Intent in Plain English**: Trace call chains strictly upwards from `classify_graph_store_error` to see what functions eventually depend on it.

**MCP Arguments (JSON)**:
```json
{
  "from": "packages/atlas-core/src/health.rs::fn::classify_graph_store_error",
  "direction": "up",
  "max_depth": 2
}
```

**Expected Clean Output & Assertions**:
- Verify trace moves in the 'up' direction (incoming calls)
- Verify upward chain identifies `internal_error_code` and `derive_integrity_state`
- Verify clean node chain without expression-level synthetic nodes

##### Test 15.2: Trace Downward Callees (What Functions This Symbol Invokes)
**Intent in Plain English**: Trace call chains downwards from `classify_graph_store_error` to explore downstream execution dependencies.

**MCP Arguments (JSON)**:
```json
{
  "from": "packages/atlas-core/src/health.rs::fn::classify_graph_store_error",
  "direction": "down",
  "max_depth": 2
}
```

**Expected Clean Output & Assertions**:
- Verify trace moves in the 'down' direction (outgoing calls)
- Verify downward chain includes `is_schema_mismatch_error` and `looks_like_logical_inconsistency`

##### Test 15.3: Shortest Call Path Between Caller and Downstream Callee
**Intent in Plain English**: Find the shortest direct invocation path between `derive_integrity_state` and `is_schema_mismatch_error`.

**MCP Arguments (JSON)**:
```json
{
  "from": "packages/atlas-core/src/readiness.rs::fn::derive_integrity_state",
  "to": "packages/atlas-core/src/health.rs::fn::is_schema_mismatch_error",
  "direction": "down"
}
```

**Expected Clean Output & Assertions**:
- Verify shortest path returned connects `derive_integrity_state` -> `classify_graph_store_error` -> `is_schema_mismatch_error`
- Verify path length is 2 hops

##### Test 15.4: Bounded Bidirectional Trace with Calls Only
**Intent in Plain English**: Trace both callers and callees with a 20-node cap, filtering strictly to 'calls' edges.

**MCP Arguments (JSON)**:
```json
{
  "from": "packages/atlas-core/src/health.rs::fn::classify_graph_store_error",
  "direction": "both",
  "edge_kinds": [
    "calls"
  ],
  "max_depth": 2,
  "max_nodes": 20
}
```

**Expected Clean Output & Assertions**:
- Verify only `calls` edges are traversed (no `contains` or `references`)
- Verify node count <= 20

---

#### Tool 16: `cross_file_links`
**Description**: Finds files that reference symbols defined in the given file, ordered by coupling strength. Weak same_package edges excluded by default.

##### Test 16.1: Default Strong Semantic Cross-File References
**Intent in Plain English**: Inspect external files with direct semantic symbol references to `packages/atlas-core/src/health.rs` without fallback noise.

**MCP Arguments (JSON)**:
```json
{
  "file": "packages/atlas-core/src/health.rs"
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.source_file == 'packages/atlas-core/src/health.rs'`
- Verify returns structured `coupling_metric`

##### Test 16.2: Include Same-Package Fallback Links
**Intent in Plain English**: Include intra-crate package coupling by setting `include_weak: True`.

**MCP Arguments (JSON)**:
```json
{
  "file": "packages/atlas-core/src/health.rs",
  "include_weak": true
}
```

**Expected Clean Output & Assertions**:
- Verify `linked_files` contains `packages/atlas-core/src/readiness.rs`
- Verify `via_symbols` lists `classify_graph_store_error`, `graph_health_error_message`
- Verify `coupling_metric >= 4.0`

##### Test 16.3: Bounded Top Linked Files (Limit 1)
**Intent in Plain English**: Cap cross-file links to the single most tightly coupled file.

**MCP Arguments (JSON)**:
```json
{
  "file": "packages/atlas-core/src/health.rs",
  "include_weak": true,
  "limit": 1
}
```

**Expected Clean Output & Assertions**:
- Verify `linked_files.length == 1`
- Verify rank 1 file is `packages/atlas-core/src/readiness.rs`

---

#### Tool 17: `concept_clusters`
**Description**: Clusters files related to seed files by shared symbol references, returning co-dependent file groups ordered by coupling density.

##### Test 17.1: Cluster Files Around Core Health Seed
**Intent in Plain English**: Cluster all files in the repository that share symbols with `packages/atlas-core/src/health.rs`.

**MCP Arguments (JSON)**:
```json
{
  "files": [
    "packages/atlas-core/src/health.rs"
  ]
}
```

**Expected Clean Output & Assertions**:
- Verify `clusters[0].files` contains `packages/atlas-core/src/readiness.rs`
- Verify `clusters[0].density >= 0.7`
- Verify `clusters[0].shared_symbols` lists `packages/atlas-core/src/health.rs::fn::classify_graph_store_error`

##### Test 17.2: Multi-File Seed Clustering
**Intent in Plain English**: Seed cluster analysis with both health and readiness files to find secondary consumers.

**MCP Arguments (JSON)**:
```json
{
  "files": [
    "packages/atlas-core/src/health.rs",
    "packages/atlas-core/src/readiness.rs"
  ]
}
```

**Expected Clean Output & Assertions**:
- Verify clusters return downstream consumers in `atlas-mcp` and `atlas-cli`
- Verify `summary.seed_file_count == 2`

##### Test 17.3: Bounded Cluster Results (Limit 3)
**Intent in Plain English**: Cap cluster groups to top 3 to keep response compact for agent reasoning.

**MCP Arguments (JSON)**:
```json
{
  "files": [
    "packages/atlas-core/src/health.rs"
  ],
  "limit": 3
}
```

**Expected Clean Output & Assertions**:
- Verify `clusters.length <= 3`
- Verify clusters are strictly sorted in descending density order

---

### Category: Bounded Context Assembly & Change Review

#### Tool 18: `get_context`
**Description**: Builds bounded context around a symbol, file, or change-set under strict token budgets and truncation policies.

##### Test 18.1: Build Bounded Context Around Core Symbol with Strict Token Budget
**Intent in Plain English**: Assemble context package around `classify_graph_store_error` capped at 1,000 tokens to prevent bloating context windows.

**MCP Arguments (JSON)**:
```json
{
  "target": {
    "kind": "query",
    "query": "classify_graph_store_error"
  },
  "token_budget": 1000,
  "max_nodes": 5
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.context_files` is populated
- Verify `structuredContent.token_count <= 1000`
- Verify declarations and immediate callers are included

##### Test 18.2: Build File-Target Bounded Context
**Intent in Plain English**: Assemble context for `packages/atlas-core/src/health.rs` with max 10 nodes.

**MCP Arguments (JSON)**:
```json
{
  "target": {
    "kind": "file",
    "path": "packages/atlas-core/src/health.rs"
  },
  "max_nodes": 10
}
```

**Expected Clean Output & Assertions**:
- Verify context includes primary symbols in `health.rs`
- Verify caller references from `readiness.rs` are attached

##### Test 18.3: Multi-File Changeset Context Merging Code and Docs
**Intent in Plain English**: Assemble combined context for a code file and its companion documentation file.

**MCP Arguments (JSON)**:
```json
{
  "target": {
    "kind": "files",
    "paths": [
      "packages/atlas-core/src/health.rs",
      "docs/error_codes.md"
    ]
  },
  "max_nodes": 15
}
```

**Expected Clean Output & Assertions**:
- Verify both code and documentation assets are merged under one bounded policy
- Verify truncation flag is boolean

---

#### Tool 19: `get_minimal_context`
**Description**: Auto-detects changed files from git and returns a compact review bundle: changed symbols, immediate impact, and risk flags with lower overhead than get_review_context.

##### Test 19.1: Minimal Review Bundle on Working Tree Changes
**Intent in Plain English**: Get high-signal minimal review context for currently modified files.

**MCP Arguments (JSON)**:
```json
{
  "change_source": {
    "kind": "working_tree"
  }
}
```

**Expected Clean Output & Assertions**:
- Verify returns `changed_symbols`
- Verify returns `impact_summary` and risk flags
- Verify payload stays well under 2,000 tokens

##### Test 19.2: Minimal Context Against Base Git Ref
**Intent in Plain English**: Inspect minimal change impact against `HEAD~1`.

**MCP Arguments (JSON)**:
```json
{
  "change_source": {
    "kind": "base",
    "base": "HEAD~1"
  },
  "max_nodes": 10
}
```

**Expected Clean Output & Assertions**:
- Verify evaluates changes between `HEAD~1` and current state
- Verify respects `max_nodes: 10`

##### Test 19.3: Minimal Context on Staged Git Index
**Intent in Plain English**: Evaluate staged changes prior to git commit.

**MCP Arguments (JSON)**:
```json
{
  "change_source": {
    "kind": "staged"
  }
}
```

**Expected Clean Output & Assertions**:
- Verify handles staged changes or returns empty bundle cleanly if nothing is staged

---

#### Tool 20: `get_review_context`
**Description**: Assembles comprehensive review context for changed files: changed symbols, impacted neighbors, critical edges, and risk summary.

##### Test 20.1: Comprehensive Review Context on Working Tree
**Intent in Plain English**: Assemble full review context for modified files with a 4,000 token budget.

**MCP Arguments (JSON)**:
```json
{
  "change_source": {
    "kind": "working_tree"
  },
  "token_budget": 4000,
  "max_depth": 2
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.changed_symbols` is populated
- Verify `impacted_neighbors` lists direct consumers
- Verify `critical_edges` highlights breaking changes

##### Test 20.2: Scoped Review Context with Exact Directory Filter
**Intent in Plain English**: Restrict review context analysis strictly to `packages/atlas-mcp`.

**MCP Arguments (JSON)**:
```json
{
  "change_source": {
    "kind": "working_tree"
  },
  "exact_dir": "packages/atlas-mcp"
}
```

**Expected Clean Output & Assertions**:
- Verify changes outside `packages/atlas-mcp` are excluded from the review bundle

##### Test 20.3: Include Explicit Ranking Evidence
**Intent in Plain English**: Include ranking reasons explaining why specific neighbor symbols were selected for review.

**MCP Arguments (JSON)**:
```json
{
  "change_source": {
    "kind": "working_tree"
  },
  "include_ranking_evidence": true,
  "max_nodes": 20
}
```

**Expected Clean Output & Assertions**:
- Verify impacted neighbors carry evidence scores and ranking factors

---

#### Tool 21: `get_impact_radius`
**Description**: Computes nodes and files affected when given change sources are modified, returning compact, capped results.

##### Test 21.1: Calculate Impact Radius for Working Tree Changes
**Intent in Plain English**: Calculate how many files and symbols across the workspace could be affected by current edits.

**MCP Arguments (JSON)**:
```json
{
  "change_source": {
    "kind": "working_tree"
  },
  "max_depth": 2,
  "max_nodes": 30
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.affected_files` lists dependent files
- Verify `structuredContent.affected_nodes` length <= 30
- Verify summary reports total blast radius

##### Test 21.2: Shallow Depth-1 Direct Blast Radius
**Intent in Plain English**: Determine only direct 1-hop consumers of modified files.

**MCP Arguments (JSON)**:
```json
{
  "change_source": {
    "kind": "working_tree"
  },
  "max_depth": 1
}
```

**Expected Clean Output & Assertions**:
- Verify no transitive 2-hop files appear in `affected_files`

##### Test 21.3: Clean Working State Impact Radius
**Intent in Plain English**: Verify that running against `HEAD` with clean tree returns empty blast radius cleanly.

**MCP Arguments (JSON)**:
```json
{
  "change_source": {
    "kind": "base",
    "base": "HEAD"
  }
}
```

**Expected Clean Output & Assertions**:
- Verify `affected_files` is empty or minimal
- Verify no error is thrown

---

#### Tool 22: `explain_change`
**Description**: Advanced impact analysis for changed files: risk level, changed-symbol breakdown by change kind (api/signature/internal), boundary violations, and test coverage gaps.

##### Test 22.1: Classify Working Tree Change Kinds and Risk Band
**Intent in Plain English**: Inspect whether current working tree modifications altered public APIs or internal implementations.

**MCP Arguments (JSON)**:
```json
{
  "change_source": {
    "kind": "working_tree"
  }
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.risk_level` is categorized (low/medium/high)
- Verify `changed_symbols` groups changes into `api`, `signature`, or `internal`
- Verify `test_coverage_gaps` flags any untested modified symbols

##### Test 22.2: Detect Architectural Boundary Violations in Changes
**Intent in Plain English**: Verify that changes do not introduce prohibited layer violations between packages.

**MCP Arguments (JSON)**:
```json
{
  "change_source": {
    "kind": "working_tree"
  }
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.boundary_violations` is reported as an array

##### Test 22.3: Bounded Change Explanation
**Intent in Plain English**: Enforce strict node caps on impact explanation.

**MCP Arguments (JSON)**:
```json
{
  "change_source": {
    "kind": "working_tree"
  },
  "max_depth": 2,
  "max_nodes": 15
}
```

**Expected Clean Output & Assertions**:
- Verify output remains bounded and deterministic

---

#### Tool 23: `detect_changes`
**Description**: Lists files changed since a base git ref, with per-file node counts from the graph.

##### Test 23.1: Detect Working Tree Changed Files with Graph Node Counts
**Intent in Plain English**: List all modified files on disk and check how many graph nodes each modified file contains.

**MCP Arguments (JSON)**:
```json
{
  "change_source": {
    "kind": "working_tree"
  }
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.files` lists modified files (e.g. `packages/atlas-mcp/src/tools/dispatch.rs`)
- Verify each file entry has `node_count` reflecting parsed symbols in that file

##### Test 23.2: Detect Staged Git Files
**Intent in Plain English**: Inspect files in the git staging index.

**MCP Arguments (JSON)**:
```json
{
  "change_source": {
    "kind": "staged"
  }
}
```

**Expected Clean Output & Assertions**:
- Verify returns staged file list and per-file node counts

##### Test 23.3: Detect Changes Relative to Remote Branch
**Intent in Plain English**: Compare local branch against `origin/main`.

**MCP Arguments (JSON)**:
```json
{
  "change_source": {
    "kind": "base",
    "base": "origin/main"
  }
}
```

**Expected Clean Output & Assertions**:
- Verify changed files count reflects commit delta

---

### Category: Graph Mutation, Build & Embedding Lifecycle

#### Tool 24: `build_graph`
**Description**: Scans, parses, and persists a complete code graph from scratch into .atlas/worldtree.db.

##### Test 24.1: Full Code Graph Rebuild Invocation
**Intent in Plain English**: Trigger a complete repository scan and parse cycle using the canonical empty object input.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.build_status.state == 'built'`
- Verify `structuredContent.build_status.nodes_written > 20000`
- Verify `structuredContent.build_status.edges_written > 80000`
- Verify `structuredContent.build_status.files_processed > 900`

##### Test 24.2: Verify Build Status Post-Conditions
**Intent in Plain English**: Confirm `last_built_at` timestamp is updated and `last_error` is null.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify `build_status.last_error == null`
- Verify `build_status.quarantine_path == null`

##### Test 24.3: Enforce Empty Argument Contract
**Intent in Plain English**: Confirm the server validates that `build_graph` expects an empty JSON object.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify successful execution without requiring redundant parameters

---

#### Tool 25: `update_graph`
**Description**: Incrementally updates the code graph from one explicit change source (working_tree, staged, or base ref).

##### Test 25.1: Incremental Update from Working Tree Changes
**Intent in Plain English**: Refresh the graph for modified files without reparsing unchanged files in the workspace.

**MCP Arguments (JSON)**:
```json
{
  "change_source": {
    "kind": "working_tree"
  }
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.updated_files` contains recently modified files
- Verify `status` subsequently reports `graph_state.stale_index == false`

##### Test 25.2: Incremental Update from Staged Changes
**Intent in Plain English**: Run update strictly for staged changes.

**MCP Arguments (JSON)**:
```json
{
  "change_source": {
    "kind": "staged"
  }
}
```

**Expected Clean Output & Assertions**:
- Verify succeeds cleanly even when staged files list is empty

##### Test 25.3: Verify Invariant Consistency After Incremental Update
**Intent in Plain English**: Confirm that incremental graph updates do not introduce dangling edges or orphan nodes.

**MCP Arguments (JSON)**:
```json
{
  "change_source": {
    "kind": "working_tree"
  }
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.success == true`
- Confirm no integrity check warnings are returned

---

#### Tool 26: `embed_chunks`
**Description**: Generates and persists embeddings for retrieval chunks lacking vectors. Requires [search.embedding] config. Supports dry_run.

##### Test 26.1: Dry Run Pending Embeddings Count
**Intent in Plain English**: Count how many text chunks require vector embeddings without invoking external embedding provider APIs.

**MCP Arguments (JSON)**:
```json
{
  "dry_run": true
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.dry_run == true`
- Verify `structuredContent.pending_chunks` is returned as an integer >= 0
- Confirm zero HTTP calls are made to embedding backends

##### Test 26.2: Bounded Embedding Batch Size
**Intent in Plain English**: Enforce chunk limit on embedding generation.

**MCP Arguments (JSON)**:
```json
{
  "limit": 10,
  "dry_run": true
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.limit == 10`

##### Test 26.3: Inspect Provider Readiness Report
**Intent in Plain English**: Verify reporting of embedding model configuration (`all-minilm` default).

**MCP Arguments (JSON)**:
```json
{
  "dry_run": true
}
```

**Expected Clean Output & Assertions**:
- Verify reports provider URL status from `.atlas/config.toml`

---

#### Tool 27: `postprocess_graph`
**Description**: Runs explicit derived-analytics postprocessing after build/update without reparsing source files.

##### Test 27.1: Dry Run Derived Analytics Lifecycle Preview
**Intent in Plain English**: Preview which postprocessing stages (metrics, cycles, dead code) would execute without altering tables.

**MCP Arguments (JSON)**:
```json
{
  "dry_run": true
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.dry_run == true`
- Verify `stages` lists planned analytics passes (`metrics`, `architecture`, `risk`)

##### Test 27.2: Changed-Only Incremental Postprocessing
**Intent in Plain English**: Run derived analytics strictly on recently updated nodes.

**MCP Arguments (JSON)**:
```json
{
  "changed_only": true,
  "dry_run": true
}
```

**Expected Clean Output & Assertions**:
- Verify skips unchanged packages and stages

##### Test 27.3: Execute Single-Stage Analytics
**Intent in Plain English**: Execute only the 'metrics' calculation stage.

**MCP Arguments (JSON)**:
```json
{
  "stage": "metrics",
  "dry_run": true
}
```

**Expected Clean Output & Assertions**:
- Verify only the `metrics` stage is scheduled

---

### Category: Code Analysis, Metrics & Refactoring Safety

#### Tool 28: `analyze_safety`
**Description**: Scores how safe it is to refactor a symbol, returning fan-in, fan-out, test adjacency, cross-module callers, and safety score (0-1).

##### Test 28.1: Refactor Safety Score for Core Public Function
**Intent in Plain English**: Assess the refactoring risk of `classify_graph_store_error` to understand breaking change blast radius.

**MCP Arguments (JSON)**:
```json
{
  "symbol": "packages/atlas-core/src/health.rs::fn::classify_graph_store_error"
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.safety_band == 'risky'` (due to public API exposure and cross-module caller)
- Verify `structuredContent.fan_in == 4` and `fan_out == 3`
- Verify `structuredContent.cross_module_callers == 1`
- Verify `suggested_validations` recommends verifying cross-module consumers and integration tests

##### Test 28.2: Refactor Safety of Private Internal Helper
**Intent in Plain English**: Evaluate an internal private function to verify reduced blast radius.

**MCP Arguments (JSON)**:
```json
{
  "symbol": "packages/atlas-core/src/health.rs::fn::is_schema_mismatch_error"
}
```

**Expected Clean Output & Assertions**:
- Verify `cross_module_callers == 0`
- Verify `fan_in` is lower than public API symbols

##### Test 28.3: Factor Evidence Verification
**Intent in Plain English**: Confirm factor evidence array details all scoring dimensions.

**MCP Arguments (JSON)**:
```json
{
  "symbol": "packages/atlas-core/src/health.rs::fn::classify_graph_store_error"
}
```

**Expected Clean Output & Assertions**:
- Verify `factor_evidence` includes keys: `fan_in`, `fan_out`, `cross_module_callers`, `linked_tests`

---

#### Tool 29: `analyze_remove`
**Description**: Analyses the impact of removing one or more symbols, returning impacted symbols, files, and tests separated by confidence tier.

##### Test 29.1: Removal Blast Radius for Core Function
**Intent in Plain English**: Analyze impact if `classify_graph_store_error` were removed from the codebase.

**MCP Arguments (JSON)**:
```json
{
  "symbols": [
    "packages/atlas-core/src/health.rs::fn::classify_graph_store_error"
  ]
}
```

**Expected Clean Output & Assertions**:
- Verify impacted callers are identified in the Definite tier (`internal_error_code`, `derive_integrity_state`)
- Verify impacted files include `packages/atlas-core/src/readiness.rs`
- Verify uncertainty flags are reported

##### Test 29.2: Bounded Removal Traversal (Depth 2, Max Nodes 15)
**Intent in Plain English**: Cap removal impact analysis to prevent unbounded cascade through entire dependency graph.

**MCP Arguments (JSON)**:
```json
{
  "symbols": [
    "packages/atlas-core/src/health.rs::fn::classify_graph_store_error"
  ],
  "max_depth": 2,
  "max_nodes": 15
}
```

**Expected Clean Output & Assertions**:
- Verify total impacted nodes <= 15

##### Test 29.3: Multi-Symbol Batch Removal Impact
**Intent in Plain English**: Analyze combined impact of removing both an error classifier and its helper.

**MCP Arguments (JSON)**:
```json
{
  "symbols": [
    "packages/atlas-core/src/health.rs::fn::classify_graph_store_error",
    "packages/atlas-core/src/health.rs::fn::is_schema_mismatch_error"
  ]
}
```

**Expected Clean Output & Assertions**:
- Verify combined removal analysis aggregates blast radius without duplicate counting

---

#### Tool 30: `analyze_dependency`
**Description**: Checks whether removing a symbol is safe by verifying it has no remaining semantic references. Returns removable verdict and blocking callers.

##### Test 30.1: Verify Referenced Symbol is NOT Removable
**Intent in Plain English**: Confirm `classify_graph_store_error` is flagged as non-removable due to active callers.

**MCP Arguments (JSON)**:
```json
{
  "symbol": "packages/atlas-core/src/health.rs::fn::classify_graph_store_error"
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.removable == false`
- Verify `structuredContent.blocking_callers` contains `packages/atlas-core/src/readiness.rs::fn::derive_integrity_state`
- Verify `structuredContent.confidence_tier == 'Definite'`

##### Test 30.2: Suggested Cleanups for Blocking Callers
**Intent in Plain English**: Check recommended cleanups provided by the tool when a symbol cannot be removed.

**MCP Arguments (JSON)**:
```json
{
  "symbol": "packages/atlas-core/src/health.rs::fn::classify_graph_store_error"
}
```

**Expected Clean Output & Assertions**:
- Verify `suggested_cleanups` identifies callers that must be refactored before removal

##### Test 30.3: Dependency Analysis of Inner Helper
**Intent in Plain English**: Check blocking callers for `is_schema_mismatch_error`.

**MCP Arguments (JSON)**:
```json
{
  "symbol": "packages/atlas-core/src/health.rs::fn::is_schema_mismatch_error"
}
```

**Expected Clean Output & Assertions**:
- Verify `removable == false`
- Verify blocking callers list `classify_graph_store_error`

---

#### Tool 31: `analyze_dead_code`
**Description**: Detects dead-code candidates: private/unexported symbols with no inbound semantic edges, not in entrypoints, not tests.

##### Test 31.1: Dead Code Candidate Scan in Core Subpath
**Intent in Plain English**: Detect unreferenced internal code candidates in `packages/atlas-core/src`.

**MCP Arguments (JSON)**:
```json
{
  "subpath": "packages/atlas-core/src",
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify `candidates` length <= 5
- Verify candidates carry certainty tiers ('definite' / 'probable' / 'speculative')
- Verify test functions and entrypoints are excluded

##### Test 31.2: Strict Code-Only Dead Code Filter
**Intent in Plain English**: Constrain dead code search strictly to code symbols (functions/structs), excluding docs.

**MCP Arguments (JSON)**:
```json
{
  "code_only": true,
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify no markdown or config candidate nodes are returned

##### Test 31.3: Dead Code Summary Statistics
**Intent in Plain English**: Retrieve aggregate summary metrics on unreferenced code across the repository.

**MCP Arguments (JSON)**:
```json
{
  "summary": true
}
```

**Expected Clean Output & Assertions**:
- Verify summary reports counts grouped by certainty tier

---

#### Tool 32: `assess_risk`
**Description**: Scores deterministic risk for one symbol with factor evidence and ranked findings (LOC, complexity, callers, public API exposure).

##### Test 32.1: Deterministic Risk Scoring for Core Function
**Intent in Plain English**: Score change risk for `classify_graph_store_error` in non-verbose compact form.

**MCP Arguments (JSON)**:
```json
{
  "symbol": "packages/atlas-core/src/health.rs::fn::classify_graph_store_error",
  "verbose": false
}
```

**Expected Clean Output & Assertions**:
- Verify `findings` contains risk classification
- Verify `factor_contributions` identifies `public_api_exposure`
- Verify output remains compact without flooding context

##### Test 32.2: Detailed Verbose Risk Factor Inspection
**Intent in Plain English**: Inspect comprehensive breakdown of risk weights and line range evidence.

**MCP Arguments (JSON)**:
```json
{
  "symbol": "packages/atlas-core/src/health.rs::fn::classify_graph_store_error",
  "verbose": true
}
```

**Expected Clean Output & Assertions**:
- Verify each factor contribution has exact line range evidence and node kinds

##### Test 32.3: Risk Comparison on Low-Coupling Symbol
**Intent in Plain English**: Assess risk of a self-contained leaf function (`now_utc`).

**MCP Arguments (JSON)**:
```json
{
  "symbol": "packages/atlas-core/src/clock.rs::fn::now_utc",
  "verbose": false
}
```

**Expected Clean Output & Assertions**:
- Verify lower overall risk classification compared to central error classifiers

---

#### Tool 33: `analyze_architecture`
**Description**: Analyzes module-level cycles, layer violations, and coupling hotspots across packages.

##### Test 33.1: Module Layer Violations and Hotspots Audit
**Intent in Plain English**: Audit architectural layering between packages (`atlas-core` -> `atlas-mcp` -> `atlas-cli`).

**MCP Arguments (JSON)**:
```json
{
  "verbose": false
}
```

**Expected Clean Output & Assertions**:
- Verify report lists module couplings and any layer violations
- Verify stays compact when `verbose: false`

##### Test 33.2: Bounded Architecture Findings (Limit 5)
**Intent in Plain English**: Retrieve top 5 architectural coupling hotspots.

**MCP Arguments (JSON)**:
```json
{
  "limit": 5,
  "verbose": false
}
```

**Expected Clean Output & Assertions**:
- Verify returned findings count <= 5

##### Test 33.3: Cyclic Dependency Scan
**Intent in Plain English**: Verify that zero cyclic package dependencies exist in the workspace.

**MCP Arguments (JSON)**:
```json
{
  "verbose": false
}
```

**Expected Clean Output & Assertions**:
- Verify cycle count is reported as 0 between primary workspace crates

---

#### Tool 34: `analyze_metrics`
**Description**: Analyzes graph health metrics, outliers, complexity hotspots, and coupling findings repo-wide.

##### Test 34.1: Identify Graph Complexity Hotspots
**Intent in Plain English**: Discover functions with high cyclomatic complexity and large fan-out.

**MCP Arguments (JSON)**:
```json
{
  "limit": 5,
  "verbose": false
}
```

**Expected Clean Output & Assertions**:
- Verify returns top 5 outlier functions
- Verify reports complexity metrics for each hotspot

##### Test 34.2: Repository Graph Density Metrics
**Intent in Plain English**: Check aggregate graph density, average node degree, and diameter.

**MCP Arguments (JSON)**:
```json
{
  "verbose": false
}
```

**Expected Clean Output & Assertions**:
- Verify summary reports global graph statistics cleanly

##### Test 34.3: Bounded Outlier Inspection (Limit 3)
**Intent in Plain English**: Retrieve exactly the top 3 extreme coupling outliers.

**MCP Arguments (JSON)**:
```json
{
  "limit": 3
}
```

**Expected Clean Output & Assertions**:
- Verify results array length == 3

---

#### Tool 35: `analyze_patterns`
**Description**: Detects repeated call chains, isolated structures, hubs, bottlenecks, and deep dependency paths.

##### Test 35.1: Detect Hub Symbols and Bottlenecks
**Intent in Plain English**: Identify structural hubs (symbols with high centrality) in the dispatch architecture.

**MCP Arguments (JSON)**:
```json
{
  "limit": 5,
  "verbose": false
}
```

**Expected Clean Output & Assertions**:
- Verify detects hubs like central dispatch routers (`tool_graph_requirement`)
- Verify reports bottleneck scores

##### Test 35.2: Deep Dependency Path Detection
**Intent in Plain English**: Identify the deepest call chains across packages to target for flattening.

**MCP Arguments (JSON)**:
```json
{
  "limit": 3,
  "verbose": false
}
```

**Expected Clean Output & Assertions**:
- Verify returns deep dependency chains with hop counts

##### Test 35.3: Isolated Structure Detection
**Intent in Plain English**: Identify disconnected subgraphs or standalone modules.

**MCP Arguments (JSON)**:
```json
{
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify reports isolated module islands if any exist

---

#### Tool 36: `find_large_functions`
**Description**: Finds large or complex functions repo-wide or within selected files using deterministic LOC and complexity thresholds.

##### Test 36.1: Repo-Wide Large Functions Discovery
**Intent in Plain English**: Find top 5 largest functions in the repository exceeding 100 LOC.

**MCP Arguments (JSON)**:
```json
{
  "threshold": 100,
  "limit": 5,
  "verbose": false
}
```

**Expected Clean Output & Assertions**:
- Verify returns functions exceeding 100 lines
- Verify each item reports `line_count` and `file_path`

##### Test 36.2: File-Specific Large Functions Check on Dispatch Module
**Intent in Plain English**: Audit `packages/atlas-mcp/src/tools/dispatch.rs` for functions exceeding 50 lines.

**MCP Arguments (JSON)**:
```json
{
  "files": [
    "packages/atlas-mcp/src/tools/dispatch.rs"
  ],
  "threshold": 50,
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify all returned functions are located in `dispatch.rs`
- Verify identifies dispatch match blocks

##### Test 36.3: Excluding Test Functions from LOC Audit
**Intent in Plain English**: Ensure test functions are excluded from large function audits.

**MCP Arguments (JSON)**:
```json
{
  "include_tests": false,
  "threshold": 150,
  "limit": 3
}
```

**Expected Clean Output & Assertions**:
- Verify no `#[test]` functions are returned

---

#### Tool 37: `find_complex_functions`
**Description**: Finds complex functions repo-wide or within selected files using deterministic cyclomatic and cognitive complexity thresholds.

##### Test 37.1: High Cyclomatic Complexity Scan (Threshold 15)
**Intent in Plain English**: Identify functions with cyclomatic complexity > 15 to prioritize for unit testing.

**MCP Arguments (JSON)**:
```json
{
  "complexity_threshold": 15,
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify all returned functions have cyclomatic complexity >= 15
- Verify exact complexity score is reported

##### Test 37.2: Cognitive Complexity Audit
**Intent in Plain English**: Scan for functions with cognitive complexity > 20 measuring mental burden.

**MCP Arguments (JSON)**:
```json
{
  "cognitive_threshold": 20,
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify functions with deeply nested branching are flagged

##### Test 37.3: File-Scoped Complexity Audit on Health Module
**Intent in Plain English**: Check complexity metrics specifically in `packages/atlas-core/src/health.rs`.

**MCP Arguments (JSON)**:
```json
{
  "files": [
    "packages/atlas-core/src/health.rs"
  ],
  "complexity_threshold": 5
}
```

**Expected Clean Output & Assertions**:
- Verify evaluates `select_graph_health_error_code` or `classify_graph_store_error`

---

#### Tool 38: `find_similar_functions`
**Description**: Finds semantically similar functions for one callable symbol using name, signature, body-shingle, and neighbor overlap.

##### Test 38.1: Find Similar Functions to Error Classifier
**Intent in Plain English**: Discover functions across the repo that perform similar error classification or mapping logic.

**MCP Arguments (JSON)**:
```json
{
  "symbol": "packages/atlas-core/src/health.rs::fn::classify_graph_store_error",
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify returns functions with signature and token shingle overlap
- Verify candidates like `classify_compiler_error` in `atlas-agent-events` appear

##### Test 38.2: Exclude Same-File Matches for Cross-Crate Deduplication
**Intent in Plain English**: Find similar functions strictly in other files to identify duplicate helper implementations.

**MCP Arguments (JSON)**:
```json
{
  "symbol": "packages/atlas-core/src/health.rs::fn::classify_graph_store_error",
  "include_same_file": false,
  "limit": 3
}
```

**Expected Clean Output & Assertions**:
- Verify no candidate in results resides in `packages/atlas-core/src/health.rs`

##### Test 38.3: Enforce Strict Minimum Similarity Score (0.6)
**Intent in Plain English**: Filter out weak matches by setting `min_score: 0.6`.

**MCP Arguments (JSON)**:
```json
{
  "symbol": "packages/atlas-core/src/health.rs::fn::classify_graph_store_error",
  "min_score": 0.6
}
```

**Expected Clean Output & Assertions**:
- Verify all returned candidates have similarity score >= 0.6

---

#### Tool 39: `find_duplicates`
**Description**: Finds exact-normalized and near-duplicate callable bodies using normalized token shingles.

##### Test 39.1: Repo-Wide Near-Duplicate Body Detection
**Intent in Plain English**: Scan for duplicated function bodies with `min_score: 0.85`.

**MCP Arguments (JSON)**:
```json
{
  "min_score": 0.85,
  "limit": 5,
  "verbose": false
}
```

**Expected Clean Output & Assertions**:
- Verify returns pairs of duplicate callable bodies with shingle similarity scores

##### Test 39.2: Scoped Duplicate Scan in Target Files
**Intent in Plain English**: Audit specific paired files for duplicated logic.

**MCP Arguments (JSON)**:
```json
{
  "files": [
    "packages/atlas-core/src/health.rs",
    "packages/atlas-core/src/readiness.rs"
  ],
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify duplicates strictly scoped to provided files

##### Test 39.3: Exclude Test Utilities from Duplicate Audit
**Intent in Plain English**: Ensure repeated test fixtures do not pollute production duplicate reports.

**MCP Arguments (JSON)**:
```json
{
  "include_tests": false,
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify test functions excluded from duplicates list

---

#### Tool 40: `infer_modules`
**Description**: Infers module buckets from package ownership, path layout, and dependency structure.

##### Test 40.1: Infer Workspace Module Architecture
**Intent in Plain English**: Inspect automatic module segmentation across crates in compact mode.

**MCP Arguments (JSON)**:
```json
{
  "verbose": false
}
```

**Expected Clean Output & Assertions**:
- Verify identifies primary modules (`core`, `mcp`, `store`, `parser`, `cli`)
- Verify reports package ownership relationships

##### Test 40.2: Bounded Module Listing (Limit 5)
**Intent in Plain English**: Retrieve top 5 module buckets by symbol count.

**MCP Arguments (JSON)**:
```json
{
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify returns exactly top 5 module clusters

##### Test 40.3: Inspect Module Boundaries and Export Ratios
**Intent in Plain English**: Check public API export ratio for inferred modules.

**MCP Arguments (JSON)**:
```json
{
  "verbose": true,
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify reports count of public symbols vs internal symbols per module

---

#### Tool 41: `label_components`
**Description**: Labels files and symbols with Atlas component taxonomy such as cli, mcp, parse, review_context, and session_continuity.

##### Test 41.1: Label MCP Dispatch File with Taxonomy
**Intent in Plain English**: Verify that `packages/atlas-mcp/src/tools/dispatch.rs` is classified under the `mcp` component taxonomy.

**MCP Arguments (JSON)**:
```json
{
  "files": [
    "packages/atlas-mcp/src/tools/dispatch.rs"
  ],
  "verbose": false
}
```

**Expected Clean Output & Assertions**:
- Verify component label for `dispatch.rs` includes `mcp`
- Verify classification confidence is high

##### Test 41.2: Label Core Function with Component Taxonomy
**Intent in Plain English**: Verify component categorization for `classify_graph_store_error`.

**MCP Arguments (JSON)**:
```json
{
  "symbols": [
    "packages/atlas-core/src/health.rs::fn::classify_graph_store_error"
  ]
}
```

**Expected Clean Output & Assertions**:
- Verify labeled under `core` or `health` component

##### Test 41.3: Bounded Taxonomy Listing
**Intent in Plain English**: Retrieve labeled components with a small limit.

**MCP Arguments (JSON)**:
```json
{
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify response stays compact

---

### Category: Companion Content & Asset Discovery

#### Tool 42: `search_files`
**Description**: Discovers files by name or path glob. Companion lookup for non-code assets (docs, config, SQL, Markdown, templates) after graph resolution.

##### Test 42.1: Discover Error Code Documentation by Glob
**Intent in Plain English**: Locate error code reference files across docs and code using glob `*error_codes*`.

**MCP Arguments (JSON)**:
```json
{
  "pattern": "*error_codes*"
}
```

**Expected Clean Output & Assertions**:
- Verify `matches` contains `docs/error_codes.md`
- Verify `matches` contains `packages/atlas-core/src/error_codes.rs`
- Verify `summary.returned_count == 2`

##### Test 42.2: Subpath-Restricted File Discovery
**Intent in Plain English**: Discover Rust source files strictly within `packages/atlas-repo/src`.

**MCP Arguments (JSON)**:
```json
{
  "pattern": "*.rs",
  "subpath": "packages/atlas-repo/src"
}
```

**Expected Clean Output & Assertions**:
- Verify all matched file paths start with `packages/atlas-repo/src/`
- Verify finds `lib.rs` and `canonical.rs`

##### Test 42.3: Case-Sensitive Exact Config Discovery
**Intent in Plain English**: Locate top-level `Cargo.toml` files with exact case sensitivity.

**MCP Arguments (JSON)**:
```json
{
  "pattern": "Cargo.toml",
  "case_sensitive": true
}
```

**Expected Clean Output & Assertions**:
- Verify matches include root `Cargo.toml` and package manifests

---

#### Tool 43: `search_content`
**Description**: Searches file contents by literal string or regex. Use as companion lookup when symbols depend on non-code text (config keys, errors, prompts).

##### Test 43.1: Search Exact Error String in Documentation
**Intent in Plain English**: Search for documentation of the error code `stale_index` inside `docs` subpath.

**MCP Arguments (JSON)**:
```json
{
  "query": "stale_index",
  "subpath": "docs"
}
```

**Expected Clean Output & Assertions**:
- Verify matches found in `docs/error_codes.md`
- Verify exact hit text and line numbers are returned

##### Test 43.2: Search Constant Literal in Core Subpath
**Intent in Plain English**: Locate references to the SQLite database name `worldtree.db` in `packages/atlas-core`.

**MCP Arguments (JSON)**:
```json
{
  "query": "worldtree.db",
  "subpath": "packages/atlas-core"
}
```

**Expected Clean Output & Assertions**:
- Verify finds constant definitions and references

##### Test 43.3: Symbol Search Anti-Pattern Warning Check
**Intent in Plain English**: Verify that searching code symbol names in search_content returns smart guidance suggesting query_graph or symbol_neighbors.

**MCP Arguments (JSON)**:
```json
{
  "query": "classify_graph_store_error",
  "subpath": "packages/atlas-core"
}
```

**Expected Clean Output & Assertions**:
- Verify `warnings` includes guidance: 'classify_graph_store_error looks like symbol name. Prefer query_graph or symbol_neighbors.'

---

#### Tool 44: `read_file_excerpt`
**Description**: Reads bounded file content from a repo-relative path using selector (range, ranges, context) without running full content search.

##### Test 44.1: Read Exact Function Line Range
**Intent in Plain English**: Read lines 62 to 70 of `packages/atlas-core/src/health.rs` containing `classify_graph_store_error`.

**MCP Arguments (JSON)**:
```json
{
  "file": "packages/atlas-core/src/health.rs",
  "selector": {
    "kind": "range",
    "start_line": 62,
    "end_line": 70
  }
}
```

**Expected Clean Output & Assertions**:
- Verify `snippets[0].lines[0]` line number == 62
- Verify text contains `pub fn classify_graph_store_error`
- Verify `snippets[0].range == [62, 70]`

##### Test 44.2: Read Multiple Non-Contiguous Line Ranges
**Intent in Plain English**: Read two separate function signatures in the same file in a single request.

**MCP Arguments (JSON)**:
```json
{
  "file": "packages/atlas-core/src/health.rs",
  "selector": {
    "kind": "ranges",
    "ranges": [
      [
        44,
        48
      ],
      [
        62,
        66
      ]
    ]
  }
}
```

**Expected Clean Output & Assertions**:
- Verify `snippets` length == 2
- Verify snippet 1 contains `is_schema_mismatch_error`
- Verify snippet 2 contains `classify_graph_store_error`

##### Test 44.3: Enforce Strict Line Truncation Limit
**Intent in Plain English**: Read top of `Cargo.toml` capped at 10 lines.

**MCP Arguments (JSON)**:
```json
{
  "file": "Cargo.toml",
  "selector": {
    "kind": "range",
    "start_line": 1,
    "end_line": 50
  },
  "max_lines": 10
}
```

**Expected Clean Output & Assertions**:
- Verify returned lines count == 10
- Verify `truncated == true`

---

#### Tool 45: `read_file_around_match`
**Description**: Reads grouped snippets around literal or regex matches inside one repo-relative file, with before/after line context.

##### Test 45.1: Retrieve Function Match Context Window
**Intent in Plain English**: Read 3 lines before and 3 lines after `pub fn classify_graph_store_error` in `packages/atlas-core/src/health.rs`.

**MCP Arguments (JSON)**:
```json
{
  "file": "packages/atlas-core/src/health.rs",
  "query": "pub fn classify_graph_store_error",
  "before": 3,
  "after": 3
}
```

**Expected Clean Output & Assertions**:
- Verify `match_mode == 'literal'`
- Verify `snippets[0].match` contains line 62
- Verify `before` contains 3 lines and `after` contains 3 lines
- Verify range is `[59, 65]`

##### Test 45.2: Regex Match Context Window
**Intent in Plain English**: Retrieve context around regex pattern `fn is_schema_[a-z]+`.

**MCP Arguments (JSON)**:
```json
{
  "file": "packages/atlas-core/src/health.rs",
  "query": "fn is_schema_[a-z]+",
  "is_regex": true,
  "before": 2,
  "after": 2
}
```

**Expected Clean Output & Assertions**:
- Verify `match_mode == 'regex'`
- Verify matched function is `is_schema_mismatch_error`

##### Test 45.3: Bounded Match Count (Max 2 Matches)
**Intent in Plain English**: Limit occurrences of common token `GraphStoreHealthClass` to 2 snippets.

**MCP Arguments (JSON)**:
```json
{
  "file": "packages/atlas-core/src/health.rs",
  "query": "GraphStoreHealthClass",
  "max_matches": 2
}
```

**Expected Clean Output & Assertions**:
- Verify `snippets.length <= 2`
- Verify `summary.matches.returned <= 2`

---

#### Tool 46: `get_docs_section`
**Description**: Resolves a Markdown section from a repo-relative documentation file using heading or line selectors, returning excerpt, heading path, and hash.

##### Test 46.1: Extract Exact Error Code Heading Section
**Intent in Plain English**: Retrieve the exact markdown section for `stale_index` from `docs/error_codes.md`.

**MCP Arguments (JSON)**:
```json
{
  "file": "docs/error_codes.md",
  "selector": {
    "kind": "heading",
    "heading": "`stale_index`"
  }
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.resolved == true`
- Verify `heading.title == '`stale_index`'`
- Verify `content` contains 'Indexed graph is older than current graph-relevant repo changes'
- Verify `line_start == 91` and `line_end == 95`

##### Test 46.2: Extract Documentation by Line Selector
**Intent in Plain English**: Retrieve the documentation block surrounding line 91.

**MCP Arguments (JSON)**:
```json
{
  "file": "docs/error_codes.md",
  "selector": {
    "kind": "line",
    "line": 91
  }
}
```

**Expected Clean Output & Assertions**:
- Verify `resolved == true`
- Verify excerpt encompasses the heading and explanation

##### Test 46.3: Handle Non-Existent Heading Gracefully
**Intent in Plain English**: Confirm querying a missing heading produces a structured candidate or error without crashing.

**MCP Arguments (JSON)**:
```json
{
  "file": "docs/error_codes.md",
  "selector": {
    "kind": "heading",
    "heading": "non_existent_heading_xyz"
  }
}
```

**Expected Clean Output & Assertions**:
- Verify tool execution error or `resolved == false`
- Verify suggests valid headings

---

#### Tool 47: `search_templates`
**Description**: Discovers template files (HTML, Jinja2, Handlebars, Tera, Liquid, Pug) by extension as companion lookup for UI/view dependencies.

##### Test 47.1: Discover Workspace Template Fixtures
**Intent in Plain English**: Find template files across the repo without subpath constraint.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify `matches` contains `packages/atlas-parser/tests/fixtures/html/core.html`
- Verify `matches` contains `packages/atlas-parser/tests/fixtures/html/bad_syntax.html`
- Verify `summary.returned_count >= 2`

##### Test 47.2: Filter Templates by Kind 'html'
**Intent in Plain English**: Restrict template discovery strictly to HTML templates.

**MCP Arguments (JSON)**:
```json
{
  "kind": "html"
}
```

**Expected Clean Output & Assertions**:
- Verify all returned matches have `kind == 'html'`

##### Test 47.3: Subpath Scoped Template Discovery
**Intent in Plain English**: Search for templates strictly inside `packages/atlas-parser`.

**MCP Arguments (JSON)**:
```json
{
  "subpath": "packages/atlas-parser"
}
```

**Expected Clean Output & Assertions**:
- Verify all matched paths start with `packages/atlas-parser/`

---

#### Tool 48: `search_text_assets`
**Description**: Discovers SQL, config (TOML/YAML/INI), environment (.env), and prompt files as non-code dependency companions.

##### Test 48.1: Discover Repository Configuration Files
**Intent in Plain English**: Find TOML and YAML configuration files across the repository.

**MCP Arguments (JSON)**:
```json
{
  "kind": "config"
}
```

**Expected Clean Output & Assertions**:
- Verify `matches` contains `Cargo.toml`, `clippy.toml`, `deny.toml`, `tasks.yaml`
- Verify each item has `asset_kind == 'config'`

##### Test 48.2: Discover GitHub Action Workflows
**Intent in Plain English**: Find YAML workflows in `.github` subpath.

**MCP Arguments (JSON)**:
```json
{
  "subpath": ".github"
}
```

**Expected Clean Output & Assertions**:
- Verify matches include `.github/workflows/ci.yml` and `.github/workflows/release.yml`

##### Test 48.3: Bounded Asset Discovery (Max 10)
**Intent in Plain English**: Limit asset search results to 10 entries.

**MCP Arguments (JSON)**:
```json
{
  "kind": "config",
  "max_results": 10
}
```

**Expected Clean Output & Assertions**:
- Verify `matches.length <= 10`

---

### Category: Session Continuity, Event Ledger & Memory Operations

#### Tool 49: `wake_up`
**Description**: Bounded session-start recall for hookless agents: assembles compact context pack from resume snapshot, decision memory, changed files, and graph readiness.

##### Test 49.1: Cold Session Start Recall Pack Assembly
**Intent in Plain English**: Initialize a fresh agent session and verify the generated context pack contains graph readiness and recent changes.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.status == 'ok'`
- Verify `structuredContent.graph_state` reflects current index readiness
- Verify session start event is recorded in the continuity ledger

##### Test 49.2: Topic-Conditioned Wake Up
**Intent in Plain English**: Wake up an agent session conditioned on a specific work topic (`mcp_refactoring`).

**MCP Arguments (JSON)**:
```json
{
  "topic": "mcp_refactoring"
}
```

**Expected Clean Output & Assertions**:
- Verify recalls decisions and memories tagged with the topic

##### Test 49.3: Bounded Wake-Up Item Cap
**Intent in Plain English**: Enforce a 5-item maximum on context items to conserve startup tokens.

**MCP Arguments (JSON)**:
```json
{
  "max_items": 5
}
```

**Expected Clean Output & Assertions**:
- Verify total assembled context items <= 5

---

#### Tool 50: `get_session_status`
**Description**: Returns status of current MCP session: identity, event count, last compaction time, and resume snapshot presence.

##### Test 50.1: Inspect Active Session Status
**Intent in Plain English**: Verify that the current MCP session is active and tracks event counters.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.session_id` is a non-empty string
- Verify `structuredContent.event_count >= 0`
- Verify `has_resume_snapshot` boolean is returned

##### Test 50.2: Verify Compaction Metadata
**Intent in Plain English**: Check whether previous compactions ran and when last curation occurred.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify `last_compaction_at` is null or valid RFC3339 timestamp

##### Test 50.3: Specific Session ID Query
**Intent in Plain English**: Query status of an explicit named test session.

**MCP Arguments (JSON)**:
```json
{
  "session_id": "test_session_isolated"
}
```

**Expected Clean Output & Assertions**:
- Verify returns clean session statistics without cross-session pollution

---

#### Tool 51: `compact_session`
**Description**: Compacts and curates the session event ledger: removes stale events, deduplicates outputs, and promotes high-value events.

##### Test 51.1: Trigger Session Event Ledger Compaction
**Intent in Plain English**: Curate the active session event ledger to evict low-value telemetry events.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.curation_stats` is returned
- Verify reports preserved and evicted event counts

##### Test 51.2: Compaction Idempotency Verification
**Intent in Plain English**: Confirm calling `compact_session` repeatedly on an unchanged ledger is safe and changes nothing.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify subsequent call produces 0 new evictions

##### Test 51.3: Session-Scoped Compaction
**Intent in Plain English**: Compact a specific target session ID.

**MCP Arguments (JSON)**:
```json
{
  "session_id": "test_session_compact"
}
```

**Expected Clean Output & Assertions**:
- Verify compaction strictly modifies the target session ledger

---

#### Tool 52: `resume_session`
**Description**: Retrieves and optionally consumes the resume snapshot for current session, building a snapshot on demand if absent.

##### Test 52.1: Inspect Resume Snapshot Without Consuming
**Intent in Plain English**: Retrieve session resume snapshot while setting `mark_consumed: false`.

**MCP Arguments (JSON)**:
```json
{
  "mark_consumed": false
}
```

**Expected Clean Output & Assertions**:
- Verify `snapshot` contains key focus symbols and modified files
- Verify snapshot remains available for future invocations

##### Test 52.2: Consume and Invalidate Resume Snapshot
**Intent in Plain English**: Read and atomically mark the resume snapshot as consumed.

**MCP Arguments (JSON)**:
```json
{
  "mark_consumed": true
}
```

**Expected Clean Output & Assertions**:
- Verify snapshot is returned
- Verify subsequent `get_session_status` reports `has_resume_snapshot == false`

##### Test 52.3: On-Demand Snapshot Construction
**Intent in Plain English**: Verify that when no snapshot exists, one is synthesized on demand from recent session events.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify valid snapshot object is created and returned

---

#### Tool 53: `record_session_event`
**Description**: Session capture for hosts without native LLM hooks, mirroring atlas hook <event>: session-start, user-prompt, tool-use, stop, file-changed.

##### Test 53.1: Record User Prompt Event
**Intent in Plain English**: Record incoming user prompt in the session event ledger.

**MCP Arguments (JSON)**:
```json
{
  "event": "user-prompt",
  "payload": {
    "prompt": "Verify MCP tools test suite against atlas"
  }
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.stored == true`
- Verify `event_id` is assigned and returned

##### Test 53.2: Record Post-Tool-Use Telemetry Event
**Intent in Plain English**: Log successful tool execution event.

**MCP Arguments (JSON)**:
```json
{
  "event": "post-tool-use",
  "payload": {
    "tool": "status",
    "duration_ms": 15,
    "success": true
  }
}
```

**Expected Clean Output & Assertions**:
- Verify event stored in `.atlas/context.db`

##### Test 53.3: Record File Changed Event Triggering Freshness Check
**Intent in Plain English**: Simulate editor file modification event.

**MCP Arguments (JSON)**:
```json
{
  "event": "file-changed",
  "payload": {
    "file": "packages/atlas-core/src/health.rs"
  }
}
```

**Expected Clean Output & Assertions**:
- Verify event triggers freshness state evaluation

---

#### Tool 54: `save_context_artifact`
**Description**: Indexes and stores large tool outputs or context payloads, returning a source_id pointer for large content or a raw preview for small content.

##### Test 54.1: Store Large Payload Returning Source Pointer
**Intent in Plain English**: Persist a large JSON payload and verify Atlas returns a compact `source_id` pointer instead of inlining the text.

**MCP Arguments (JSON)**:
```json
{
  "label": "test_graph_analysis_report",
  "content": "{\"status\":\"ok\",\"nodes_checked\":25000,\"details\":\"Extended graph analysis content sample payload\"}",
  "content_type": "json"
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.source_id` is returned as a non-empty hash string
- Verify `structuredContent.stored == true`
- Confirm the full payload is not echoed back into context

##### Test 54.2: Store Small Note with Inline Preview
**Intent in Plain English**: Persist small string and verify compact preview handling.

**MCP Arguments (JSON)**:
```json
{
  "label": "short_test_note",
  "content": "All health checks green"
}
```

**Expected Clean Output & Assertions**:
- Verify returns source reference with preview

##### Test 54.3: Scoped Artifact Storage with Source Type
**Intent in Plain English**: Store artifact tagged with `source_type: 'tool_output'`.

**MCP Arguments (JSON)**:
```json
{
  "label": "mcp_tool_benchmark",
  "content": "Execution benchmark results: 240 tests passed",
  "source_type": "tool_output"
}
```

**Expected Clean Output & Assertions**:
- Verify `source_type` metadata is persisted

---

#### Tool 55: `read_saved_context`
**Description**: Retrieves the full content of a saved artifact by source_id, supporting paging via chunk_offset and max_bytes.

##### Test 55.1: Read Saved Artifact Content by Pointer
**Intent in Plain English**: Fetch saved payload using the `source_id` returned from `save_context_artifact`.

**MCP Arguments (JSON)**:
```json
{
  "source_id": "test_saved_artifact_pointer"
}
```

**Expected Clean Output & Assertions**:
- Verify retrieves persisted string or fails with clean structured error if ID does not exist
- Verify enforces session boundaries

##### Test 55.2: Bounded Paged Retrieval with Max Bytes
**Intent in Plain English**: Page through a large artifact using `max_bytes: 100` and `chunk_offset: 0`.

**MCP Arguments (JSON)**:
```json
{
  "source_id": "test_saved_artifact_pointer",
  "max_bytes": 100,
  "chunk_offset": 0
}
```

**Expected Clean Output & Assertions**:
- Verify returned bytes <= 100

##### Test 55.3: Graceful Handling of Missing Artifact
**Intent in Plain English**: Verify requesting a non-existent artifact produces a structured not-found error.

**MCP Arguments (JSON)**:
```json
{
  "source_id": "non_existent_source_id_xyz"
}
```

**Expected Clean Output & Assertions**:
- Verify returns error payload with `error_code == 'node_not_found'` or not-found status

---

#### Tool 56: `search_saved_context`
**Description**: Searches previously saved artifacts in the content store using BM25 + trigram fallback, returning preview matches.

##### Test 56.1: BM25 Search Over Saved Session Artifacts
**Intent in Plain English**: Search for saved analysis outputs mentioning 'graph analysis'.

**MCP Arguments (JSON)**:
```json
{
  "query": "graph analysis"
}
```

**Expected Clean Output & Assertions**:
- Verify `matches` array returns matched artifacts with preview snippets
- Verify `summary.total_matches` is reported

##### Test 56.2: Bounded Search Results (Limit 3)
**Intent in Plain English**: Cap search results to top 3 matches.

**MCP Arguments (JSON)**:
```json
{
  "query": "health checks",
  "limit": 3
}
```

**Expected Clean Output & Assertions**:
- Verify `matches.length <= 3`

##### Test 56.3: Filter Saved Context by Source Type
**Intent in Plain English**: Search strictly artifacts marked as `tool_output`.

**MCP Arguments (JSON)**:
```json
{
  "query": "benchmark",
  "source_type": "tool_output"
}
```

**Expected Clean Output & Assertions**:
- Verify all matched artifacts carry `source_type == 'tool_output'`

---

#### Tool 57: `cross_session_search`
**Description**: Searches saved context artifacts across all sessions for this repo, returning query, sessions, preview matches, and summary.

##### Test 57.1: Cross-Session Context Recall
**Intent in Plain English**: Search historical session artifacts for previous mentions of `classify_graph_store_error`.

**MCP Arguments (JSON)**:
```json
{
  "query": "classify_graph_store_error"
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.sessions` lists distinct session IDs where content appeared
- Verify `matches` contains preview text and source IDs

##### Test 57.2: Bounded Cross-Session Recall (Limit 5)
**Intent in Plain English**: Cap cross-session search to 5 items.

**MCP Arguments (JSON)**:
```json
{
  "query": "error classification",
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify `matches.length <= 5`

##### Test 57.3: Agent Partitioned Cross-Session Search
**Intent in Plain English**: Search across sessions created by a specific agent ID.

**MCP Arguments (JSON)**:
```json
{
  "query": "refactor",
  "agent_id": "agent_worker_1"
}
```

**Expected Clean Output & Assertions**:
- Verify respects agent partition boundaries

---

#### Tool 58: `get_context_stats`
**Description**: Returns storage statistics for current (or specified) session: event count, saved source count, chunk count, and DB paths.

##### Test 58.1: Inspect Active Session Storage Footprint
**Intent in Plain English**: Inspect database paths and counts for events, saved sources, and chunks.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.event_count >= 0`
- Verify `structuredContent.saved_source_count >= 0`
- Verify `structuredContent.content_db_path` ends with `.atlas/context.db`

##### Test 58.2: Inspect Target Session Storage Stats
**Intent in Plain English**: Query stats for a specific historical session.

**MCP Arguments (JSON)**:
```json
{
  "session_id": "test_session_stats"
}
```

**Expected Clean Output & Assertions**:
- Verify returns statistics for target session without error

##### Test 58.3: Chunk and Token Metrics Verification
**Intent in Plain English**: Verify chunk counts reflect indexed text asset segments.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify `chunk_count` is returned as integer

---

#### Tool 59: `purge_saved_context`
**Description**: Deletes saved artifacts. Provide session_id to delete artifacts for that session, or omit to apply age-based cleanup (default 30 days).

##### Test 59.1: Age-Based Context Cleanup (Keep Last 30 Days)
**Intent in Plain English**: Purge stale temporary artifacts older than 30 days.

**MCP Arguments (JSON)**:
```json
{
  "keep_days": 30
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.purged_count` is reported as integer
- Verify recent session artifacts remain intact

##### Test 59.2: Purge Ephemeral Test Session Artifacts
**Intent in Plain English**: Delete all artifacts belonging to a completed test session.

**MCP Arguments (JSON)**:
```json
{
  "session_id": "ephemeral_test_session"
}
```

**Expected Clean Output & Assertions**:
- Verify only artifacts from target session are removed

##### Test 59.3: Safe Deletion Boundary Verification
**Intent in Plain English**: Ensure keep_days 365 produces 0 deletions on fresh repository.

**MCP Arguments (JSON)**:
```json
{
  "keep_days": 365
}
```

**Expected Clean Output & Assertions**:
- Verify `purged_count == 0`

---

#### Tool 60: `get_global_memory`
**Description**: Returns cross-session global memory summary for repo: frequently-accessed symbols/files and recurring workflow patterns.

##### Test 60.1: Inspect Cross-Session Global Memory Summary
**Intent in Plain English**: Retrieve repository-wide hot symbols, frequently accessed files, and recurring patterns.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.frequently_accessed_symbols` is returned
- Verify `structuredContent.frequently_accessed_files` is returned

##### Test 60.2: Context-Conditioned Global Memory Matching
**Intent in Plain English**: Provide focus symbols to retrieve relevant historical sessions.

**MCP Arguments (JSON)**:
```json
{
  "focus_symbols": [
    "packages/atlas-core/src/health.rs::fn::classify_graph_store_error"
  ],
  "focus_files": [
    "packages/atlas-core/src/health.rs"
  ]
}
```

**Expected Clean Output & Assertions**:
- Verify ranks past sessions matching the focus context

##### Test 60.3: Bounded Global Memory Limit
**Intent in Plain English**: Cap global memory results to 5 entries.

**MCP Arguments (JSON)**:
```json
{
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify returned hot symbols list <= 5

---

#### Tool 61: `memory_store`
**Description**: Stores a memory record for this repo through the shared memory service, returning id, scope, importance, and source_id.

##### Test 61.1: Store Project-Scoped Architecture Memory
**Intent in Plain English**: Store an architectural invariant regarding health error classification.

**MCP Arguments (JSON)**:
```json
{
  "topic": "architecture",
  "title": "Health Error Classification Invariant",
  "text": "classify_graph_store_error maps errors to GraphStoreHealthClass enum variants",
  "importance": 4,
  "scope": "project"
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.id` is returned
- Verify `structuredContent.scope == 'project'`
- Verify `structuredContent.importance == 4`

##### Test 61.2: Store Session-Scoped Temporary Memory
**Intent in Plain English**: Store an ephemeral note for current session.

**MCP Arguments (JSON)**:
```json
{
  "topic": "testing",
  "title": "MCP Test Suite Status",
  "text": "Executed test pass for all 80 tools",
  "scope": "session"
}
```

**Expected Clean Output & Assertions**:
- Verify scope is set to `session`

##### Test 61.3: Store Memory with Linked Patch File
**Intent in Plain English**: Store a memory linked directly to a target file path.

**MCP Arguments (JSON)**:
```json
{
  "topic": "refactoring",
  "title": "Health module decoupling",
  "text": "Decoupled readiness evaluation from low-level SQLite parsing",
  "patch_file": "packages/atlas-core/src/health.rs"
}
```

**Expected Clean Output & Assertions**:
- Verify memory record links `patch_file`

---

#### Tool 62: `memory_recall`
**Description**: Recalls memories for this repo with lexical ranking, defaults, and visibility rules. Exact topic matches rank first.

##### Test 62.1: Recall Memories by Lexical Query
**Intent in Plain English**: Search persisted memories for 'classify_graph_store_error'.

**MCP Arguments (JSON)**:
```json
{
  "query": "classify_graph_store_error",
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify returns matched memories with relevance ranking
- Verify titles and text match query concepts

##### Test 62.2: Topic-Filtered Memory Recall
**Intent in Plain English**: Query memories filtered strictly to topic 'architecture'.

**MCP Arguments (JSON)**:
```json
{
  "query": "Classification",
  "topic": "architecture"
}
```

**Expected Clean Output & Assertions**:
- Verify memories matching topic 'architecture' rank first

##### Test 62.3: Shared Scope Recall (Project + Global Only)
**Intent in Plain English**: Recall only shared memories while excluding ephemeral session notes.

**MCP Arguments (JSON)**:
```json
{
  "query": "Invariant",
  "shared": true
}
```

**Expected Clean Output & Assertions**:
- Verify all returned memories have scope `project` or `global`

---

#### Tool 63: `memory_ops`
**Description**: Returns bounded typed-operator history for one memory row (store, update, evict). Read-only parity with atlas memory ops.

##### Test 63.1: Inspect Memory Operator Mutation History
**Intent in Plain English**: Inspect audit history of operators applied to a specific memory ID.

**MCP Arguments (JSON)**:
```json
{
  "memory_id": "mem_test_record_001"
}
```

**Expected Clean Output & Assertions**:
- Verify returns history or `found == false` cleanly if ID unknown
- Verify no unhandled exception on missing memory

##### Test 63.2: Verify Audit Trail Fields
**Intent in Plain English**: Confirm memory operator events carry operator identity and timestamps.

**MCP Arguments (JSON)**:
```json
{
  "memory_id": "mem_test_record_001"
}
```

**Expected Clean Output & Assertions**:
- Verify operator event records have valid timestamps

##### Test 63.3: Missing ID Non-Error Handling
**Intent in Plain English**: Verify nonexistent memory ID returns `found: false` instead of an error payload.

**MCP Arguments (JSON)**:
```json
{
  "memory_id": "non_existent_memory_99999"
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.found == false`

---

#### Tool 64: `search_decisions`
**Description**: Searches persisted decision memory for prior conclusions, linked evidence, and artifact references.

##### Test 64.1: Search Decisions on Architectural Boundaries
**Intent in Plain English**: Search previous engineering decisions regarding health checks and readiness.

**MCP Arguments (JSON)**:
```json
{
  "query": "health error classification"
}
```

**Expected Clean Output & Assertions**:
- Verify `matches` contains decision records with rationale and evidence links
- Verify `summary.total_matches` is reported

##### Test 64.2: Bounded Decision Query (Limit 3)
**Intent in Plain English**: Retrieve at most 3 historical decision records.

**MCP Arguments (JSON)**:
```json
{
  "query": "refactor",
  "limit": 3
}
```

**Expected Clean Output & Assertions**:
- Verify `matches.length <= 3`

##### Test 64.3: Empty Query Handling
**Intent in Plain English**: Search for a concept with zero decisions recorded.

**MCP Arguments (JSON)**:
```json
{
  "query": "completely_unrelated_decision_topic"
}
```

**Expected Clean Output & Assertions**:
- Verify `matches == []`
- Verify `summary.total_matches == 0`

---

#### Tool 65: `experience_inspect`
**Description**: Inspects query-conditioned experience edges touching one atlas:// URI in session.db.

##### Test 65.1: Inspect Experience Edges for Symbol URI
**Intent in Plain English**: Inspect dynamic experience edges touching `atlas://repo/current/symbol/packages/atlas-core/src/health.rs::fn::classify_graph_store_error`.

**MCP Arguments (JSON)**:
```json
{
  "uri": "atlas://repo/current/symbol/packages/atlas-core/src/health.rs::fn::classify_graph_store_error"
}
```

**Expected Clean Output & Assertions**:
- Verify returns experience edges recorded in `session.db`
- Verify does not mutate or contaminate code graph truth

##### Test 65.2: Directional Inbound Experience Inspection
**Intent in Plain English**: Filter experience edges to inbound references for a file URI.

**MCP Arguments (JSON)**:
```json
{
  "uri": "atlas://repo/current/file/packages/atlas-core/src/health.rs",
  "direction": "inbound"
}
```

**Expected Clean Output & Assertions**:
- Verify only inbound experience edges returned

##### Test 65.3: Bounded Experience Edge Limit
**Intent in Plain English**: Cap inspected experience edges to 5.

**MCP Arguments (JSON)**:
```json
{
  "uri": "atlas://repo/current/file/packages/atlas-core/src/health.rs",
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify results length <= 5

---

#### Tool 66: `feedback_record`
**Description**: Records a correction when an analysis prediction was wrong, stored as deterministic correction memory to adjust future confidence.

##### Test 66.1: Record Refactoring Safety Prediction Correction
**Intent in Plain English**: Record that `classify_graph_store_error` was predicted risky but determined safe due to internal testing harness usage.

**MCP Arguments (JSON)**:
```json
{
  "tool": "analyze_safety",
  "symbol": "packages/atlas-core/src/health.rs::fn::classify_graph_store_error",
  "predicted": "risky",
  "actual": "safe",
  "correction": "Cross-module call is in internal integration test harness only"
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.stored == true`
- Verify correction memory is assigned an ID
- Verify future analysis queries can access deterministic correction

##### Test 66.2: Record Dead Code Candidate False Positive
**Intent in Plain English**: Record feedback that a flagged dead code candidate is actually invoked via macro dispatch.

**MCP Arguments (JSON)**:
```json
{
  "analysis_kind": "dead_code",
  "file": "packages/atlas-core/src/health.rs",
  "symbol": "packages/atlas-core/src/health.rs::fn::looks_like_logical_inconsistency",
  "predicted": "dead",
  "actual": "live",
  "correction": "Called inside classify_graph_store_error branch"
}
```

**Expected Clean Output & Assertions**:
- Verify correction record persists with `analysis_kind == 'dead_code'`

##### Test 66.3: Enforce Required Prediction and Actual Fields
**Intent in Plain English**: Confirm the server enforces the required `predicted` and `actual` parameters.

**MCP Arguments (JSON)**:
```json
{
  "predicted": "high_risk",
  "actual": "low_risk"
}
```

**Expected Clean Output & Assertions**:
- Verify feedback successfully stored with minimal valid schema

---

### Category: Memoir Knowledge Graph Layer

#### Tool 67: `memoir_create`
**Description**: Creates a memoir concept graph in the continuity database as a separate knowledge layer from the code graph.

##### Test 67.1: Create Project-Scoped Architecture Memoir Graph
**Intent in Plain English**: Create a new memoir concept graph named 'ArchitectureConcepts' to store conceptual architectural models.

**MCP Arguments (JSON)**:
```json
{
  "name": "ArchitectureConcepts",
  "description": "Conceptual domain models and module relationships",
  "scope": "project"
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.created == true` or graph exists
- Verify graph is registered in continuity DB
- Verify separate knowledge layer: zero code graph nodes/edges created

##### Test 67.2: Duplicate Graph Name Deterministic Rejection
**Intent in Plain English**: Verify that creating a graph with an existing name in the same scope fails deterministically.

**MCP Arguments (JSON)**:
```json
{
  "name": "ArchitectureConcepts",
  "scope": "project"
}
```

**Expected Clean Output & Assertions**:
- Verify deterministic error indicating duplicate graph name in scope

##### Test 67.3: Create Session-Scoped Ephemeral Memoir Graph
**Intent in Plain English**: Create an ephemeral memoir graph scoped strictly to the current session.

**MCP Arguments (JSON)**:
```json
{
  "name": "SessionInvestigation",
  "scope": "session"
}
```

**Expected Clean Output & Assertions**:
- Verify graph created with `scope == 'session'`

---

#### Tool 68: `memoir_add_concept`
**Description**: Adds a concept to an existing memoir graph. Missing graphs are never auto-created. Concept names are unique per graph.

##### Test 68.1: Add Health Diagnostics Concept to Architecture Graph
**Intent in Plain English**: Add concept 'HealthDiagnostics' with rich description to the 'ArchitectureConcepts' graph.

**MCP Arguments (JSON)**:
```json
{
  "graph": "ArchitectureConcepts",
  "name": "HealthDiagnostics",
  "description": "Error classification, sqlite integrity checking, and graph readiness evaluation"
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.concept_id` is assigned
- Verify concept added to target graph

##### Test 68.2: Add Related Readiness Concept
**Intent in Plain English**: Add second concept 'ReadinessVerification' to enable conceptual linking.

**MCP Arguments (JSON)**:
```json
{
  "graph": "ArchitectureConcepts",
  "name": "ReadinessVerification",
  "description": "Derivation of integrity states and query readiness overrides"
}
```

**Expected Clean Output & Assertions**:
- Verify concept created with unique name

##### Test 68.3: Missing Graph Auto-Creation Rejection
**Intent in Plain English**: Confirm adding a concept to a nonexistent graph fails without auto-creating the graph.

**MCP Arguments (JSON)**:
```json
{
  "graph": "NonExistentMemoirGraph",
  "name": "OrphanConcept",
  "description": "Should fail"
}
```

**Expected Clean Output & Assertions**:
- Verify deterministic failure: 'graph not found'

---

#### Tool 69: `memoir_link`
**Description**: Links two memoir concepts with a controlled relation type (e.g. depends_on, relates_to). Rejects unknown relation types.

##### Test 69.1: Link Concepts with Controlled 'depends_on' Relation
**Intent in Plain English**: Establish directional link from 'ReadinessVerification' to 'HealthDiagnostics'.

**MCP Arguments (JSON)**:
```json
{
  "graph": "ArchitectureConcepts",
  "source": "ReadinessVerification",
  "target": "HealthDiagnostics",
  "relation": "depends_on",
  "confidence": 0.95
}
```

**Expected Clean Output & Assertions**:
- Verify edge created between concepts
- Verify `relation == 'depends_on'` and `confidence == 0.95`

##### Test 69.2: Normalize Alias Relation 'replaced_by'
**Intent in Plain English**: Verify that relation alias `replaced_by` reverses direction and normalizes properly.

**MCP Arguments (JSON)**:
```json
{
  "graph": "ArchitectureConcepts",
  "source": "LegacyHealthCheck",
  "target": "HealthDiagnostics",
  "relation": "replaced_by",
  "create_missing": true
}
```

**Expected Clean Output & Assertions**:
- Verify edge direction is normalized

##### Test 69.3: Reject Unknown Relation Type
**Intent in Plain English**: Verify that arbitrary unsupported relation types are rejected with a controlled relation list.

**MCP Arguments (JSON)**:
```json
{
  "graph": "ArchitectureConcepts",
  "source": "ReadinessVerification",
  "target": "HealthDiagnostics",
  "relation": "arbitrary_unsupported_link"
}
```

**Expected Clean Output & Assertions**:
- Verify tool fails with invalid relation error

---

#### Tool 70: `memoir_inspect`
**Description**: Inspects a concept's bounded memoir neighbourhood with canonical edge directions, clamped depth (1..=5), and node caps.

##### Test 70.1: Inspect Concept Immediate Neighborhood (Depth 1)
**Intent in Plain English**: Inspect connected concepts, edge directions, and confidence scores for 'HealthDiagnostics'.

**MCP Arguments (JSON)**:
```json
{
  "graph": "ArchitectureConcepts",
  "concept": "HealthDiagnostics",
  "depth": 1
}
```

**Expected Clean Output & Assertions**:
- Verify returns connected neighbors including 'ReadinessVerification'
- Verify edge direction is labeled ('outgoing' / 'incoming')

##### Test 70.2: Multi-Hop Traversal Clamped to Depth Ceiling
**Intent in Plain English**: Verify traversal depth is strictly clamped within 1..=5.

**MCP Arguments (JSON)**:
```json
{
  "graph": "ArchitectureConcepts",
  "concept": "HealthDiagnostics",
  "depth": 3
}
```

**Expected Clean Output & Assertions**:
- Verify depth clamped to maximum allowed ceiling

##### Test 70.3: Filter Neighborhood by Relation Type
**Intent in Plain English**: Inspect only `depends_on` relations touching the concept.

**MCP Arguments (JSON)**:
```json
{
  "graph": "ArchitectureConcepts",
  "concept": "HealthDiagnostics",
  "relation": "depends_on"
}
```

**Expected Clean Output & Assertions**:
- Verify all returned edges have `relation == 'depends_on'`

---

### Category: Overview Analysis & Caching

#### Tool 71: `overview_export`
**Description**: Exports bounded overview analysis packets for external LLM or human analysis with subject identity, commit SHA, graph freshness, and evidence.

##### Test 71.1: Export Overview Packet for Core Health File
**Intent in Plain English**: Export a bounded overview packet for `packages/atlas-core/src/health.rs` with token budget 2,000.

**MCP Arguments (JSON)**:
```json
{
  "subject": "packages/atlas-core/src/health.rs",
  "max_tokens": 2000
}
```

**Expected Clean Output & Assertions**:
- Verify packet contains canonical subject identity
- Verify commit SHA and graph freshness metadata attached
- Verify bounded evidence included without exceeding token budget

##### Test 71.2: Export Overview for Changed Files Only
**Intent in Plain English**: Export analysis packets strictly for modified files.

**MCP Arguments (JSON)**:
```json
{
  "changed_only": true,
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify packets generated only for modified files

##### Test 71.3: Export Symbol Overview with Callers and Callees
**Intent in Plain English**: Include relational call boundaries in export packet.

**MCP Arguments (JSON)**:
```json
{
  "subject": "packages/atlas-core/src/health.rs::fn::classify_graph_store_error",
  "include_callers": true,
  "include_callees": true
}
```

**Expected Clean Output & Assertions**:
- Verify callers and callees arrays populated in the packet

---

#### Tool 72: `overview_ingest`
**Description**: Ingests externally produced overview analysis JSON, validating packet IDs, subject identity, commit SHA, line spans, and redaction.

##### Test 72.1: Ingest Valid Overview Analysis Packet
**Intent in Plain English**: Persist an externally generated overview summary for `packages/atlas-core/src/health.rs`.

**MCP Arguments (JSON)**:
```json
{
  "analysis": {
    "subject": "packages/atlas-core/src/health.rs",
    "summary": "Core error classification and graph health diagnostics module",
    "confidence": 0.95
  }
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.ingested == true` or valid record id returned
- Verify validated before persistence

##### Test 72.2: Reject Malformed Overview Payload
**Intent in Plain English**: Verify that missing canonical subject or corrupt schemas are rejected.

**MCP Arguments (JSON)**:
```json
{
  "analysis": {
    "invalid_property": 123
  }
}
```

**Expected Clean Output & Assertions**:
- Verify schema validation error returned

##### Test 72.3: Large Body Overview Ingestion Routing
**Intent in Plain English**: Verify large body routing persists via content store.

**MCP Arguments (JSON)**:
```json
{
  "input": "{\"subject\":\"packages/atlas-mcp\",\"summary\":\"Full MCP protocol implementation\"}"
}
```

**Expected Clean Output & Assertions**:
- Verify successful ingestion routing

---

#### Tool 73: `overview_search`
**Description**: Searches overview memory with FTS plus explicit ranking (exact subject, title, freshness, confidence, recency).

##### Test 73.1: Full-Text Search Overview Memory
**Intent in Plain English**: Search persisted overview summaries for 'error classification'.

**MCP Arguments (JSON)**:
```json
{
  "query": "error classification",
  "limit": 5
}
```

**Expected Clean Output & Assertions**:
- Verify returns ranked overview records
- Verify ranking evidence includes freshness and confidence

##### Test 73.2: Package-Scoped Overview Search
**Intent in Plain English**: Restrict overview search to `atlas-core` package.

**MCP Arguments (JSON)**:
```json
{
  "query": "health",
  "package": "atlas-core"
}
```

**Expected Clean Output & Assertions**:
- Verify results strictly within `atlas-core`

##### Test 73.3: Filter by Freshness State
**Intent in Plain English**: Exclude stale overviews by setting `freshness: 'fresh'`.

**MCP Arguments (JSON)**:
```json
{
  "query": "graph",
  "freshness": "fresh"
}
```

**Expected Clean Output & Assertions**:
- Verify all returned overviews are fresh

---

#### Tool 74: `overview_show`
**Description**: Shows one overview record by subject id, qualified name, canonical file path, package, or module.

##### Test 74.1: Show Overview by Canonical File Path
**Intent in Plain English**: Retrieve overview for `packages/atlas-core/src/health.rs`.

**MCP Arguments (JSON)**:
```json
{
  "target": "packages/atlas-core/src/health.rs"
}
```

**Expected Clean Output & Assertions**:
- Verify returns overview record or clean candidate list if absent
- Verify subject matches file path

##### Test 74.2: Show Overview by Qualified Symbol Name
**Intent in Plain English**: Show overview for `packages/atlas-core/src/health.rs::fn::classify_graph_store_error`.

**MCP Arguments (JSON)**:
```json
{
  "subject": "packages/atlas-core/src/health.rs::fn::classify_graph_store_error"
}
```

**Expected Clean Output & Assertions**:
- Verify symbol-level overview resolved

##### Test 74.3: Ambiguous Subject Graceful Candidate List
**Intent in Plain English**: Verify ambiguous query returns candidate list rather than crashing.

**MCP Arguments (JSON)**:
```json
{
  "target": "health"
}
```

**Expected Clean Output & Assertions**:
- Verify returns candidate options if multiple matches exist

---

#### Tool 75: `overview_refresh_plan`
**Description**: Plans overview freshness refresh after changes: reports directly stale, possibly stale, and orphaned subjects with export commands.

##### Test 75.1: Read-Only Overview Freshness Inspection
**Intent in Plain English**: Inspect stale overview subjects without modifying state (`apply: false`).

**MCP Arguments (JSON)**:
```json
{
  "apply": false
}
```

**Expected Clean Output & Assertions**:
- Verify reports `directly_stale` and `orphaned` subject lists
- Verify suggested export commands provided

##### Test 75.2: Apply Overview Freshness Plan
**Intent in Plain English**: Apply refresh plan to mark stale overview records.

**MCP Arguments (JSON)**:
```json
{
  "apply": true
}
```

**Expected Clean Output & Assertions**:
- Verify marks stale records in continuity database

##### Test 75.3: Plan Refresh Relative to Base Commit
**Intent in Plain English**: Evaluate overview freshness delta since `HEAD~1`.

**MCP Arguments (JSON)**:
```json
{
  "since": "HEAD~1"
}
```

**Expected Clean Output & Assertions**:
- Verify plans staleness based on commit diff

---

### Category: MCP Registry, Introspection & Tool Documentation

#### Tool 76: `tool_list`
**Description**: Lists visible exported MCP tools in compact runtime inventory form with input schemas and result contracts.

##### Test 76.1: Full Tool Inventory Enumeration
**Intent in Plain English**: Enumerate all exported MCP tools to verify total registry size and completeness.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.tools` count is >= 79 (80 with `trace_calls`)
- Verify every tool descriptor has `name`, `description`, and `inputSchema`

##### Test 76.2: Filter Tool Inventory by Category 'graph'
**Intent in Plain English**: List only tools belonging to the 'graph' category.

**MCP Arguments (JSON)**:
```json
{
  "category": "graph"
}
```

**Expected Clean Output & Assertions**:
- Verify returned tools are members of graph family (`query_graph`, `traverse_graph`, `symbol_neighbors`)

##### Test 76.3: Verify Result Contract Annotations
**Intent in Plain English**: Verify that all tools declare `stable-object` result contract.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify tools advertise structuredContent output schemas

---

#### Tool 77: `tool_search`
**Description**: Searches visible exported MCP tools by name, title, or description with typo-tolerant fuzzy matching.

##### Test 77.1: Typo-Tolerant Tool Discovery
**Intent in Plain English**: Search for 'travers' and verify `traverse_graph` ranks at the top.

**MCP Arguments (JSON)**:
```json
{
  "query": "travers"
}
```

**Expected Clean Output & Assertions**:
- Verify `traverse_graph` is ranked #1 in matches
- Verify similarity score is high

##### Test 77.2: Search by Functional Intent (Dead Code)
**Intent in Plain English**: Search for 'dead code detection'.

**MCP Arguments (JSON)**:
```json
{
  "query": "dead code detection"
}
```

**Expected Clean Output & Assertions**:
- Verify `analyze_dead_code` returned as best match

##### Test 77.3: Search Call Chain Tracing Tools
**Intent in Plain English**: Search for 'trace callers and callees'.

**MCP Arguments (JSON)**:
```json
{
  "query": "trace callers and callees"
}
```

**Expected Clean Output & Assertions**:
- Verify matches include `trace_calls` and `symbol_neighbors`

---

#### Tool 78: `tool_help`
**Description**: Returns runtime manual documentation for one visible exported MCP tool by exact name.

##### Test 78.1: Retrieve Runtime Help for Graph Traversal
**Intent in Plain English**: Fetch documentation and parameter guidance for `traverse_graph`.

**MCP Arguments (JSON)**:
```json
{
  "name": "traverse_graph"
}
```

**Expected Clean Output & Assertions**:
- Verify `name == 'traverse_graph'`
- Verify returns usage guidance, `from_qn`, `max_depth`, `max_nodes` documentation
- Verify output contract detailed

##### Test 78.2: Retrieve Help for trace_calls Tool
**Intent in Plain English**: Fetch parameter documentation for `trace_calls`.

**MCP Arguments (JSON)**:
```json
{
  "name": "trace_calls"
}
```

**Expected Clean Output & Assertions**:
- Verify details `from`, `to`, `direction`, `edge_kinds`, `max_depth`

##### Test 78.3: Graceful Help Error for Unknown Tool
**Intent in Plain English**: Request help for non-existent tool name.

**MCP Arguments (JSON)**:
```json
{
  "name": "unknown_tool_xyz"
}
```

**Expected Clean Output & Assertions**:
- Verify returns error with suggestions for nearest valid tool names

---

#### Tool 79: `man`
**Description**: Returns authoritative runtime manual documentation for one visible exported MCP tool (requires namespace='mcp' and tool_name).

##### Test 79.1: Authoritative Manual for query_graph
**Intent in Plain English**: Retrieve authoritative manual pages for `query_graph` under namespace 'mcp'.

**MCP Arguments (JSON)**:
```json
{
  "namespace": "mcp",
  "tool_name": "query_graph"
}
```

**Expected Clean Output & Assertions**:
- Verify `structuredContent.manual` contains grammar rules and regex syntax
- Verify detailed description of intent phrases ('who calls', 'what breaks')

##### Test 79.2: Authoritative Manual for symbol_neighbors
**Intent in Plain English**: Retrieve manual documentation for `symbol_neighbors`.

**MCP Arguments (JSON)**:
```json
{
  "namespace": "mcp",
  "tool_name": "symbol_neighbors"
}
```

**Expected Clean Output & Assertions**:
- Verify details caller and callee edge classification rules

##### Test 79.3: Enforce Required MCP Namespace Parameter
**Intent in Plain English**: Verify that non-'mcp' namespaces are rejected.

**MCP Arguments (JSON)**:
```json
{
  "namespace": "cli",
  "tool_name": "query_graph"
}
```

**Expected Clean Output & Assertions**:
- Verify validation error requiring `namespace: 'mcp'`

---

#### Tool 80: `repo_registry`
**Description**: Inspects persisted multi-repo registry metadata under .atlas/, including registered repo identities, relationship kinds, and trust states.

##### Test 80.1: Inspect Workspace Repository Identity
**Intent in Plain English**: Verify the primary repository ID and registered root path.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify current repo is registered (`repo_4d07b53766d88ffd`)
- Verify root path matches current workspace directory
- Verify trust state is active

##### Test 80.2: Check for Bootstrap Warnings
**Intent in Plain English**: Confirm multi-repo registry contains zero bootstrap warnings or path conflicts.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify `warnings == []`

##### Test 80.3: Inspect Multi-Repo Relationships
**Intent in Plain English**: Verify registered inter-repo linkage metadata.

**MCP Arguments (JSON)**:
```json
{}
```

**Expected Clean Output & Assertions**:
- Verify registry metadata schema conforms to multi-repo specification

---

## 5. Summary & Verification Matrix for Agent Harnesses

| Family | Tool Count | Primary Verification Objective |
|---|---|---|
| **Health & Diagnostics** | 6 tools | SQLite integrity, process liveness, runtime metrics, staleness detection |
| **Search & Exploration** | 6 tools | Deterministic symbol pagination, exact QN resolution, FTS phrase intent |
| **Traversal & Relationships** | 5 tools | Callers/callees tracing, shortest call path, coupling density, clusters |
| **Context & Review** | 6 tools | Bounded review bundles, blast radius calculation, risk classification |
| **Build & Mutation** | 4 tools | Rebuild stats, incremental working-tree indexing, embedding dry runs |
| **Code Analysis & Safety** | 14 tools | Refactoring safety score, blocking callers, cyclomatic/cognitive complexity |
| **Companion Content** | 7 tools | Markdown doc heading extraction, line range excerpts, config asset search |
| **Session & Memory** | 18 tools | Ledger compaction, resume snapshot lifecycle, memory recall, feedback |
| **Memoir Knowledge** | 4 tools | Conceptual knowledge graphs, controlled relation types, depth clamping |
| **Overview Analysis** | 5 tools | Bounded analysis packets, LLM-free freshness planning, overview FTS |
| **MCP Registry & Introspection** | 5 tools | Runtime tool manual, inventory filtering, multi-repo metadata |
| **TOTAL** | **80 tools** | **Complete coverage of the Atlas MCP surface (244 test cases)** |

> **End of Test Specification — Generated for `atlas-context-router-rs`**
