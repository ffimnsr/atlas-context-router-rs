# atlas-mcp

MCP server exposing Atlas graph, impact, and review tools over stdio. Implements the Model Context Protocol (MCP) 2.0 specification to expose graph operations as tools available to LLMs and agents.

## Public Surface

**Tools** (20+)
- `list_graph_stats` — node/edge counts and language breakdown
- `query_graph` — FTS5 keyword search over symbols
- `batch_query_graph` — run up to 20 queries in one round-trip
- `get_context` — general context engine for symbol, file, review, or impact
- `get_review_context` — review bundle with symbols, neighbors, risk summary
- `get_impact_radius` — traversal from changed files
- `detect_changes` — git diff to changed-file list
- `build_or_update_graph` — full or incremental graph build
- `explain_change` — advanced risk analysis and test gaps
- `status` — health check and graph stats
- `doctor` — deep repo and DB diagnostics
- Plus: `traverse_graph`, `symbol_neighbors`, `resolve_symbol`, `search_content`, `analyze_safety`, `analyze_removal`, `analyze_dead_code`, and more

Listens on stdio or Unix socket; supports JSON-RPC 2.0 message framing.
