# atlas-refactor

Smart refactoring core for Atlas — deterministic rename, dead-code removal, import cleanup, and extract-function detection. Syntax-aware transforms backed by graph validation.

## Public Surface

- **`RefactorEngine`** — main entry point
  - `plan_rename()` — compute rename impact and generate patches
  - `apply_rename()` — execute rename with optional dry-run
  - Dead-code removal candidates and validation
  - Import cleanup and consolidation
  - Extract-function detection from selected ranges

- **Output**
  - `RefactorPlan` — scoped symbol changes with evidence
  - `RefactorResult` — unified diffs per file
  - Patch validation with rollback capability

Deterministic transforms using tree-sitter AST and graph-based call resolution.
