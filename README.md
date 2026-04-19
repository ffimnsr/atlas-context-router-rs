# Atlas

Atlas reimplements core `code-review-graph` behavior in Rust.

Current scope stays narrow:

- repo scan
- parse
- persist graph in SQLite
- update incrementally from git diff
- search and traverse graph
- assemble review context

## MVP Status

MVP core path complete. Post-MVP work must not weaken these commands:

- `atlas init`
- `atlas build`
- `atlas status`
- `atlas query "some symbol"`
- `atlas update --base <ref>`
- `atlas impact --base <ref>`
- `atlas review-context --base <ref>`

Repository keeps explicit post-MVP gate in tests:

- CLI quality gates verify MVP command contract end to end on committed fixture repo.
- Deferred features stay documented outside v1 scope.

## Deferred Scope

Deferred features do not block core path. They stay tracked in [atlas-v2-todo.md](atlas-v2-todo.md) and listed in [COMPATIBILITY.md](COMPATIBILITY.md).

Examples:

- embeddings / hybrid retrieval
- communities and flows
- wiki / visualization / export
- watch mode and install hooks
- multi-repo and cloud integrations

Rule: keep Atlas centered on repo scan, parse, persist, update, search, review context. Optional features ship only after MVP gate stays green.
