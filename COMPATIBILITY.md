# Compatibility Breaks

This document records every intentional behavioral difference between the Rust
Atlas implementation and the upstream Python `code-review-graph` project.

---

## Naming and Paths

| Upstream (Python)               | Atlas (Rust)                     | Reason |
|---------------------------------|----------------------------------|--------|
| `.code-review-graph/` work dir  | `.atlas/` work dir               | cleaner product name |
| `codegraph.sqlite` DB filename  | `worldview.sqlite` DB filename   | clearer product language |
| no reserved config path         | `.atlas/config.toml` (reserved)  | forward-compatible slot |

## Architecture

| Upstream                        | Atlas                            | Reason |
|---------------------------------|----------------------------------|--------|
| Single Python monolith          | Cargo workspace of focused crates| maintainability, compile-time safety |
| `absolute` file paths in DB     | `repo-relative` file paths in DB | portability across machines |
| Giant single-file parser        | Per-language handler modules     | reviewability, independent extension |

## Data Model

| Upstream                        | Atlas                            | Reason |
|---------------------------------|----------------------------------|--------|
| No stable `NodeId` type         | `id: i64` on `Node` (typed `NodeId` planned) | explicit identity |
| DB schema not versioned         | `schema_version` migration table | safe upgrades |

## Git Integration

| Upstream                        | Atlas                            | Reason |
|---------------------------------|----------------------------------|--------|
| Python `subprocess` / `GitPython` | `std::process::Command` wrapping `git` CLI | no native libgit2 dep in v1; simpler build |
| Symlink follow behavior varies  | Symlink policy deferred (v1 skips) | correctness gap to address post-v1 |

## Deferred Features (not in v1)

The following upstream capabilities are explicitly excluded from Atlas v1 and
will not be ported until post-MVP:

- embeddings / vector search
- community detection
- flows / diagramming
- wiki generation
- visualization / graph export
- multi-repo registry
- install hooks / watchers
- auto-watch mode (`fsnotify` style)
- refactor / apply-refactor
- evaluation harness
- cloud provider integrations
