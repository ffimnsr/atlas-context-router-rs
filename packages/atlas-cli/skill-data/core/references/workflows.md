# Atlas workflows

Task recipes beyond the core loop. Each MCP call has a CLI mirror; run `atlas skills get core --ref cli` for every subcommand, argument, and flag.

## Review a change-set

1. `detect_changes` when changed files are unknown.
2. `get_minimal_context` for first bounded triage.
3. `get_review_context` when changed symbols, neighbors, or risk detail is needed.
4. `explain_change` when deterministic risk and test-gap explanation is needed.
5. `get_impact_radius` when explicit blast radius is needed.

```bash
atlas detect-changes --base origin/main
atlas review-context --base origin/main --format markdown --max-depth 3 --max-nodes 200
atlas explain-change --base origin/main --max-depth 5 --max-nodes 200
atlas impact --base origin/main --max-depth 5 --max-nodes 200
atlas review-context --files src/lib.rs src/session.rs
```

Add `--staged` to `detect-changes` or `explain-change` for staged diffs, or pass `--files` explicitly. Pass `--allow-stale` or `--allow-partial` only when a freshness warning is acceptable; otherwise refresh first.

## Inspect one symbol

1. `query_graph` or `resolve_symbol` to land on the canonical qualified name.
2. `symbol_neighbors` for direct callers, callees, and tests.
3. `get_context` for bounded ranked context.
4. `trace_calls` for a directional chain or a path to a target.
5. `traverse_graph` only for unfiltered multi-hop reachability.

```bash
atlas symbols --resolve handle_request
atlas query handle_request --kind function
atlas content neighbors src/lib.rs::fn::handle_request
atlas context "who calls handle_request"
atlas trace handle_request --direction up --depth 2
atlas trace build_graph --to reconcile_call_targets
```

`atlas content` mirrors the companion content tools: `files`, `grep`, `excerpt`, `match-context`, `templates`, `text-assets`, `neighbors`, `links`, and `clusters`. Use them after graph tools point at code, not instead of them.

## Decide whether code is safe to remove or rename

```bash
atlas analyze remove src/lib.rs::fn::legacy_parse --max-depth 3 --max-nodes 200
atlas analyze safety src/lib.rs::fn::legacy_parse
atlas analyze dependency src/lib.rs::fn::legacy_parse
atlas analyze dead-code
atlas insights risk src/lib.rs::fn::legacy_parse
atlas refactor rename --symbol src/lib.rs::fn::legacy_parse --to parse_legacy --dry-run
```

Keep `--dry-run` on refactors until the planned edits are reviewed, and run the project's tests after applying. `analyze dead-code` lists candidates; it does not prove a symbol is unused at runtime.

## Repo-wide analysis reports

```bash
atlas insights architecture
atlas insights metrics --limit 20
atlas insights patterns --limit 20
atlas insights large-functions --threshold 80 --mode large-or-complex
atlas insights complex-functions --complexity-threshold 15
atlas insights similar-functions src/lib.rs::fn::handle_request --limit 10
atlas insights duplicates --min-score 0.7
atlas insights infer-modules
atlas insights label-components --files src/lib.rs
```

Reports are deterministic and never call a model. They accept `--allow-stale` and `--allow-partial`; prefer refreshing the graph instead.

## Keep the graph fresh

`atlas_provenance` and `atlas_freshness` describe the graph the answer came from. If freshness says stale after edits, refresh before making claims:

```bash
atlas update --working-tree
atlas update --base origin/main
```

A missing or corrupt database needs a full rebuild with `build_graph` or `atlas build`.
