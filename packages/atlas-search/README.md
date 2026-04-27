# atlas-search

FTS-backed graph search and ranking utilities for Atlas. Provides symbol retrieval via full-text search, regex filtering, and optional hybrid vector-assisted ranking.

## Public Surface

- **Search modes**
  - FTS (full-text search) over persisted graph records
  - Regex structural scans with SQL UDF postfiltering
  - Hybrid FTS + embedding vector fusion (optional)
  - Graph expansion over neighboring nodes

- **Ranking**
  - Evidence-backed scoring primitives
  - Symbol name and qualified-name boosting
  - Fuzzy typo recovery for near-miss lookups
  - Semantic expansion reranking

- **Modules**
  - `semantic` — graph-neighbor expansion and reranking
  - `embed` — optional vector embedding integration
  - `eval` — ranking metrics and benchmark harness

Foundation for `atlas query` CLI command and MCP search tools.
