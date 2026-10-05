# atlas-metrics

In-process runtime metrics for Atlas: lock-free counters and fixed-bucket
histograms for build, query, parser, and MCP activity.

## Metric catalog

| Family | Type | Labels | Recorded by |
|--------|------|--------|-------------|
| `build_runs` | counter | `kind` (`build` / `update`) | `atlas-engine` run wrappers |
| `build_failures` | counter | `kind` | `atlas-engine` run wrappers |
| `build_duration_ms` | histogram | `kind` | `atlas-engine` run wrappers |
| `build_parsed_files` | histogram | `kind` | `atlas-engine` run wrappers (successful runs only) |
| `parser_parses_total` | counter | – | `atlas-parser` `ParserRegistry::parse` |
| `parser_tree_reuses_total` | counter | – | `atlas-parser` `ParserRegistry::parse` when `old_tree` was supplied |
| `parser_cache_reuse_ratio` | derived | – | `parser_tree_reuses_total / parser_parses_total` |
| `query_calls` | counter | `mode` | `atlas-search` `execute_query_with_embedding` |
| `query_duration_ms` | histogram | `mode` | `atlas-search` `execute_query_with_embedding` |
| `mcp_tool_calls` | counter | `tool`, `ok`/`error` | `atlas-mcp` tool dispatch |
| `mcp_tool_duration_ms` | histogram | `tool` | `atlas-mcp` tool dispatch |

## Design

- Counters use `AtomicU64` with relaxed ordering; histograms keep per-bucket,
  count, sum, min, and max atomics. Recording never allocates on the fast path
  for existing labels and never holds a lock while incrementing.
- Label registration is the only locked operation; a poisoned lock is recovered
  rather than propagated, so metrics can never take down a build or query.
- `Metrics` can be instantiated per test for isolated assertions. Production
  code records into the process-global registry returned by `metrics()`.
- `snapshot()` returns a serializable `MetricsSnapshot`; ratios and nested
  label maps are derived at snapshot time.

## Usage

```rust
let snapshot = atlas_metrics::snapshot();
let json = serde_json::to_value(&snapshot).expect("snapshot serializes");
```

Metric snapshots are surfaced by:

- `atlas build --json` / `atlas update --json` (`data.metrics` field for non-dry-run runs)
- the MCP `get_metrics` tool (live counters for the running server process)
