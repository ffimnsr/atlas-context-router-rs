# atlas-core

Core domain models, kinds, and error types shared across Atlas crates. Defines cross-crate types used by graph build, update, query, review, and transport layers so other crates avoid re-declaring storage-specific or transport-specific copies.

## Public Surface

- **`AtlasError`, `Result`** — unified error type for all Atlas operations

- **`kinds`** — graph node and edge classifications
  - `NodeKind` (function, struct, class, trait, etc.)
  - `EdgeKind` (calls, imports, containment, etc.)
  - `Language` (Rust, Go, Python, TypeScript, etc.)

- **`model`** — persisted and returned data structures
  - `Node`, `Edge` — graph primitives with location metadata
  - `QualifiedName` — canonical symbol identifiers

- **`budget`, `health`, `error`** — operational contracts
  - Budget constraints for build and analysis operations
  - Health states for graph and index validity
  - Error codes and failure contexts

Provides stable shared vocabulary for all layers; most other Atlas crates depend on this.
