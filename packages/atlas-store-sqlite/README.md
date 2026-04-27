# atlas-store-sqlite

SQLite graph store and migrations for Atlas code graph persistence. Owns schema creation, graph persistence, history storage, and graph-backed query helpers over `.atlas/worldtree.db`.

## Public Surface

- **`Store`** — main store interface
  - `create()` — open or initialize graph database
  - `insert_graph()` — persist parsed nodes and edges
  - `update_graph()` — incremental graph updates
  - `impact_radius()` — traversal from changed files
  - `search_nodes()` — FTS and regex queries
  - `symbol_neighbors()` — immediate callers/callees/tests
  - `traverse_graph()` — bidirectional reachability walks

- **Analytics and maintenance**
  - `postprocess_status()` — derived-analytics state
  - `db_check()`, `debug_graph()` — corruption detection and diagnostics
  - `vacuum()` — freelist reclaim for bloated databases

- **Historical snapshots** — graph state at any indexed commit

Thread-confined ownership per concurrency policy; separate connections for concurrent access.
