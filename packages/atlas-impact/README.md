# atlas-impact

Impact scoring, boundary analysis, and test reachability for Atlas. Provides advanced impact analysis via weighted traversal and change classification from graph-backed impact results.

## Public Surface

- **`analyze()`** — comprehensive impact computation
  - Weighted traversal with decay factors per hop
  - Boundary detection (public API, cross-module edges)
  - Test adjacency and reachability scoring
  - Change classification (signature, internal, API)

- **Output**
  - `AdvancedImpactResult` with scored nodes, boundaries, test gaps
  - Evidence-backed confidence tiers
  - Exclusion and skip-reason annotations

Foundation for risk assessment and test-impact features in reasoning and review engines.
