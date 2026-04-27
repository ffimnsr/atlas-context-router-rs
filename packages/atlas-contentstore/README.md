# atlas-contentstore

Durable artifact content store for Atlas context memory integration. Stores large command outputs, tool results, and context payloads in `.atlas/context.db`, separate from the graph database and session database. Each `ContentStore` instance owns one thread-confined SQLite connection; concurrent access uses separate store instances and separate connections.

## Public Surface

- **`ContentStore`** — main store interface
  - `create()` — open or create context database
  - `save_artifact()` — store large payloads with chunking
  - `read_artifact()` — retrieve artifact by source ID
  - `search_saved_context()` — BM25 search over artifacts
  - `cleanup()` — manage retention and lifecycle

- **Modules**
  - `chunking` — payload fragmentation for large artifacts
  - `store` — core ContentStore implementation

Thread-confined SQLite ownership per AGENTS.md concurrency policy.
