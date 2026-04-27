# atlas-review

Review context assembly and risk summarization for Atlas code changes. Bridges graph, impact, search, and saved-content signals into bounded review outputs for CLI and MCP callers.

## Public Surface

- **`build_context()`** — bounded context retrieval
  - Symbol, file, review, or impact intent
  - Graph traversal with depth and node limits
  - Test node inclusion and code-span extraction
  - Saved-artifact search integration

- **`ContextEngine`** — stateful context builder
  - Configurable depth, edge, and file caps
  - Token-budget enforcement
  - Semantic expansion and import control

- **`build_explain_change_summary()`** — change-risk analysis
  - Changed-symbol classification
  - Boundary violation detection
  - Test coverage gap identification
  - Risk scoring and confidence tiers

Higher-level assembly layer combining search, impact, and content signals.
