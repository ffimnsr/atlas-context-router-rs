# atlas-reasoning

Autonomous code reasoning engine for Atlas — removal impact, dead code, refactor safety, rename radius, and change risk. Answers structural questions from graph + parser + store facts only; all results carry structured evidence and confidence tiers.

## Public Surface

- **`ReasoningEngine`** — main entry point
  - `analyze_removal()` — compute removal impact and blocking callers
  - `analyze_dependency()` — check if symbol can be removed safely
  - `analyze_dead_code()` — find likely dead-code candidates with certainty tiers
  - `analyze_safety()` — score refactor safety using fan-in/out and test adjacency

- **Output types**
  - `ReasoningEvidence` — ranked nodes, edges, files by relevance
  - `ConfidenceTier` — Definite, Probable, Weak classifications
  - Blocker flags and skip reasons for each finding

Deterministic, unprompted analysis with full evidence trails for explainability.
