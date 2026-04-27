# atlas-repo

Repository discovery, file collection, diffing, and path utilities for Atlas. Keeps repo-relative file handling deterministic across graph build, history, persistence, and retrieval flows.

## Public Surface

- **`CanonicalRepoPath`** — normalized path identity
  - Unicode NFKC normalization
  - Relative path canonicalization
  - Collision-safe hashing for graph persistence
  - Used as identity key for all graph-derived data

- **`discover_package_owners()`** — workspace root detection and package mapping

- **`changed_files()`** — git diff integration
  - Stage vs working-tree change detection
  - Per-file content hashing
  - Dependency computation for incremental builds

- **`collect_supported_files()`** — filesystem traversal with ignore handling
  - `.gitignore` and `.atlas/ignore` respect
  - Language detection by extension

Deterministic path handling ensures graph metadata never carries non-canonical paths.
