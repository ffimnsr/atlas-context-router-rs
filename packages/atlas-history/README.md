# atlas-history

Historical graph metadata ingestion and querying for Atlas. Enables cross-version graph comparison, historical change analysis, and time-series insights over git-indexed commits.

## Public Surface

- **Commit metadata**
  - Deterministic git wrapper for commit selection and traversal
  - Commit metadata ingestion into SQLite

- **Historical builds**
  - Content-addressed node/edge storage keyed by git blob SHA
  - Checkout-free file reconstruction via `git show`
  - Historical graph snapshots per indexed commit

- **Lifecycle and diff**
  - Incremental update with missing-commit detection
  - Force-push divergence detection
  - Node and edge lifecycle computation
  - Diff any two indexed commits with file, node, edge, module, and architecture scopes

Complements graph persistence for deterministic temporal analysis.
