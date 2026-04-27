# atlas-adapters

Transport and frontend adapter layer for Atlas context memory integration. Provides thin adapter implementations so CLI, MCP, and other surfaces can funnel session events and large outputs through the content and session stores without coupling to transport details.

## Public Surface

- **Modules**
  - `artifacts` — artifact storage and retrieval adapters
  - `bridge` — transport-agnostic event and output bridging
  - `events` — event type definitions for session lifecycle
  - `hooks` — CLI and MCP adapter hook interfaces
  - `redact` — payload redaction and sanitization

- **Key Types**
  - `AdapterHooks` — contract for transport-specific event handling
  - `CliAdapter`, `McpAdapter` — concrete implementations
  - Reexports from `atlas-contextsave` and `atlas-session` for storage operations
