---
name: split-large-files
description: Split oversized code files reported by `mk lf` into proper modules until every code file stays under 1000 LOC, strictly one file per iteration. Use when asked to work through `mk lf`, reduce file sizes, break up a 1000+ line module, or split large files while preserving implementation, function count, and call sites exactly.
---

# split-large-files

Reduce every oversized code file from `mk lf` below 1,000 LOC. Each file is a
pure mechanical move into modules: same functions, same signatures, same
statements, same call sites, same runtime behavior. Work one file at a time and
start the next file only after the current file passes every gate.

## Hard invariants

- **One file at a time.** Never edit two oversized files in the same iteration.
  Never parallelize splits with sub-agents.
- **Pure move only.** Cut and paste existing code. Do not rename, deduplicate,
  reorder, rewrite, simplify, reformat by hand, remove dead code, merge tests,
  add helpers, or "fix while here".
- **Function parity.** The multiset of `fn` definitions (names and count) before
  and after must be identical across the whole split. Signatures stay
  byte-identical except visibility.
- **Call parity.** The multiset of call tokens before and after must be
  identical. No call added, removed, inlined, or duplicated.
- **Public API parity.** Every item reachable as `crate::<module>::Item` before
  the split stays reachable at the same path after the split via `pub use`.
- **Import/export parity.** Imports move with the code that uses them and are
  rebased when module depth changes. Visibility is the narrowest that compiles
  and is never widened just to fix a call. Parent re-exports are explicit.
- **Only code files.** Skip docs and non-code assets even when `mk lf` lists
  them: `.md`, `.mdx`, `.markdown`, `.rst`, `.adoc`, `.txt`, `.org`, config,
  data, lockfiles, snapshots. Only source files are split.
- **Size.** The original file and every new module file must be < 1,000 LOC.
  Aim for <= 800.
- **No commits.** Do not commit, do not run destructive git commands, do not
  touch unrelated files.
- **AGENTS.md still applies.** Atlas before `grep`/file search for code
  discovery. Keep the diff focused. Update `ISSUES.md` only when a matching open
  checklist item exists and the finished work fully satisfies it.

Allowed edits during a split are limited to:

- `mod` declarations and explicit `pub use` re-exports
- visibility modifiers on items and fields needed for cross-module access
  (`pub(super)`, `pub(crate)`)
- `use` lines, including adding `use super::*;`, moving imports into the module
  that uses them, and rebasing `use super::X` paths when module depth changes
- moving `#[cfg(test)]` gating onto the module declaration that owns the tests

If a body reference breaks, add a `use` import or alias. Never edit the body
statement to fix a path.

## 0. Preflight

```bash
git --no-optional-locks status --short
mk lf
bash .agents/skills/split-large-files/scripts/list-code-files.sh
```

- `list-code-files.sh` prints `<loc> <path>` for code files only and reports
  skipped non-code files on stderr.
- Work in `mk lf` order (largest first) unless the user named a file or order.
- If no code files remain, report and stop.
- If an extension is not obviously code, ask before splitting it.
- Record the worklist before editing; do not discover it ad hoc.

## 1. Per-file loop

Repeat this entire loop for exactly one file, then return to step 0 for the next
file.

### 1.1 Inventory with Atlas

- `get_context` with the file target to see its module, neighbors, and tests.
- `list_symbols` scoped to the file path to inventory `fn`/`method` nodes.
- `get_impact_radius` for files that import the module when the split may move
  public items.
- Read the whole file only after Atlas has identified it.

### 1.2 Snapshot the baseline

The verifier needs the pre-split file content:

```bash
slug="<package>-<file-stem>"
mkdir -p "/tmp/atlas-split/$slug"
cp "<file>" "/tmp/atlas-split/$slug/before.rs"
```

Use a package-qualified slug so repeated `mod.rs` names do not collide.

### 1.3 Choose split boundaries

- Map top-level items: module docs, imports, types, `impl` blocks, free
  functions, inline test modules.
- Group by cohesive responsibility: one type plus its impls, command handlers,
  loader/parser stages, rendering, persistence, test topics.
- Move whole items. Prefer whole `impl` blocks; if an `impl` must be split,
  replicate the exact original `impl` header (generics, bounds, `where`) and move
  complete methods only.
- Never split a single function; a file that is one giant function cannot be
  fixed by moving items. Stop and ask.
- Keep the dependency direction one-way. Put shared types/helpers in the parent
  or a `types`/`shared` child rather than creating cycles.
- Never add `#[path]`, `include!`, or duplicate code to break a cycle.
- Two to four new modules is typical. Do not micro-split.

### 1.4 Module layout

Match the file's existing neighborhood style; this repo uses both `mod.rs`
directories and `<stem>.rs` + `<stem>/` sibling directories.

- Existing `mod.rs` directory (for example `store/mod.rs`): move code into new
  sibling files, add `mod <topic>;`, and re-export from `mod.rs` with
  `pub use self::<topic>::{...};`.
- Existing `<stem>.rs` file: add `<stem>/<topic>.rs` and `mod <topic>;` in
  `<stem>.rs`. Converting the file into `<stem>/mod.rs` is allowed only when the
  split makes the parent mostly declarations; the module path must not change.
- New child modules start with `use super::*;` when they need parent imports.
  Keep imports in the file that actually uses them; remove imports that became
  unused in the parent so clippy stays clean.
- Preserve module doc comments (`//!`) in the parent. Do not invent new docs.
- Re-export every publicly reachable item from the parent so external
  `use crate::<module>::Item` paths compile unchanged.

### 1.5 Test modules

- Keep `#[cfg(test)]` on the declaration that owns the tests
  (`#[cfg(test)] mod tests;` / `#[cfg(test)] mod <topic>;`).
- Split by test topic, not by line count. Keep `#[test]` attributes, test names,
  assertions, and helper behavior unchanged.
- Move shared fixtures to the existing helper module (`tests/support.rs` or the
  local equivalent). Never duplicate helpers.
- No `#[ignore]`, no `#[should_panic]` changes, no test removal or merging.

### 1.6 Fix imports and visibility

Moving code changes privacy scope and import depth. Rust rules that drive this
pass:

- a child module can access private items of its ancestors
- a parent or sibling cannot access private items of a child
- `pub(super)` is visible in the parent module and all of its descendants,
  including siblings
- `pub(crate)` is visible anywhere in the crate
- plain `pub` is reachable outside the crate only when the item is also
  re-exported at the original public module path

Set the narrowest visibility that compiles:

| Moved item is used by | Visibility |
|---|---|
| only the new module | unchanged/private |
| the parent or a sibling | `pub(super)` |
| anywhere else in the crate | `pub(crate)` |
| external consumers before the split | `pub` plus parent re-export |
| struct/tuple field accessed across modules | same rule as the item |
| trait impl method | no visibility qualifier (the trait defines it) |

Import rules:

- Move each import into the module that uses it; delete imports that became
  unused in the parent. `cargo clippy -D warnings` catches `unused_imports`.
- New child modules start with `use super::*;` when they consume parent names.
  Use explicit imports instead when a glob would be ambiguous.
- When a file moves one level deeper, `use super::X` no longer points at the
  original parent. Rebase it to `use crate::<original>::X;` (preferred) or
  `use super::super::X;`. Rebase import paths only; never edit a function body
  to fix a path.
- Keep `pub(crate) use` / `pub(super) use` at the original visibility; do not
  promote them to `pub use`.
- If a moved `macro_rules!` is used outside its new module, add
  `pub(crate) use <macro>;` or `pub(super) use <macro>;`. Keep
  `#[macro_export]` unchanged.

Export rules:

- New modules created by the split stay private (`mod <topic>;`). Only
  pre-existing `pub mod` visibility is preserved. Expose items through parent
  `pub use`, not by making new module paths public.
- The parent module must re-export every item that was reachable as
  `crate::<module>::Item` before the split:
  `pub use self::<topic>::{A, B};`.
- List names explicitly. Do not use `pub use self::<topic>::*;` for public API;
  it expands the API and can create ambiguous exports.
- Never re-export a `pub(crate)` item as `pub`; use `pub(crate) use` for
  crate-only re-exports.
- One owner per name. If two children expose the same name, keep one definition
  and re-export it from one place.
- Types in public signatures must stay reachable from the original public path,
  otherwise rustc `private_interfaces` fails. Re-export them; do not widen
  unrelated internals.
- Preserve original `pub use` chains so downstream imports resolve to the same
  final items.

Never widen a `pub(crate)` or `pub(super)` item just to silence a compile error.
Regroup items or add a re-export instead.

### 1.7 Move and format

1. Cut blocks verbatim, including doc comments, attributes, and internal order.
2. Paste into the new module in the same relative order.
3. Apply the import/visibility pass in 1.6.
4. Run `cargo fmt --all` and let rustfmt do all formatting.

## 2. Validation gate

Run from the repo root. Replace `<pkg>` with the Cargo package name from
`packages/<dir>/Cargo.toml` (for `packages/atlas-cli` check the `[package]`
name, not the directory or binary name).

```bash
cargo fmt --all
cargo check -p <pkg>
cargo clippy -p <pkg> --all-targets --all-features -- -D warnings
cargo test -p <pkg>
bash .agents/skills/split-large-files/scripts/verify-pure-move.sh \
  "/tmp/atlas-split/<slug>/before.rs" <reduced-original> <new-module> [...]
mk lf
git --no-optional-locks status --short
```

Gate rules:

- Every command must pass before the next file starts.
- `verify-pure-move.sh` must print `PURE MOVE VERIFIED`. It checks LOC < 1000
  per output file, `fn` name multiset, call-token multiset, and normalized source
  line multiset. The line check is a multiset; the mandatory `--color-moved` diff
  below is what proves statement order inside each moved block.
- Import/export correctness is not covered by the verifier, which deliberately
  ignores `use`/`mod`/visibility scaffolding. Enforce it with `cargo check`,
  clippy (`unused_imports`, `private_interfaces`), and this public-path audit:
  ```bash
  # Externally reachable definitions and re-exports before the split.
  grep -nE '^pub (struct|enum|trait|type|const|static|fn|mod|use)\b' \
    "/tmp/atlas-split/<slug>/before.rs" \
    > "/tmp/atlas-split/<slug>/public-before.txt"

  # Parent definitions and re-exports after the split.
  grep -nE '^(pub (struct|enum|trait|type|const|static|fn|mod)\b|pub(\([^)]*\))? use\b)' \
    <parent-file>
  ```
  Every public definition from the baseline must be defined in the parent or
  named by an explicit parent `pub use`. `pub(crate)`/`pub(super)` items must
  stay non-public. Inspect preserved `pub use` chains for the same final names.
- `mk lf` must no longer list the split file, and no new module may appear.
- For a test-only split, run the targeted test filter first, then the full
  package test command.
- If `cargo clippy` reports unrelated pre-existing failures, record the exact
  command and error and stop; do not fix unrelated code.
- Optionally refresh the graph with `update_graph`
  (`change_source = working_tree`) before the next file.

## 3. Second pass (mandatory)

Do this after the gate, before touching the next file. It is a review pass, not
a test rerun.

1. Run:
   ```bash
   git --no-pager diff --stat -- <paths>
   git --no-pager diff --color-moved=zebra -w -- <paths>
   ```
2. Confirm moved blocks appear as moves: no rewritten statements, no reordered
   statements inside a moved block, no changed literals, no changed comments.
3. Confirm function and call counts from the verifier output match exactly.
4. Confirm every cross-module access uses the narrowest visibility: no `pub`
   added only to make a call compile, no `pub(crate)`/`pub(super)` widened.
5. Confirm imports moved with their users, unused parent imports were removed,
   and every `use super::X` path was rebased for the new module depth.
6. Confirm the parent `pub use` list is explicit, collision-free, and covers
   every formerly public item; no wildcard public re-export and no new `pub mod`
   was added.
7. Confirm the public-path audit and `private_interfaces` pass, and that no
   macro or trait-impl visibility rule was broken.
8. Confirm test names and assertions are unchanged and only their module path
   moved.
9. Confirm no unrelated files, docs, fixtures, or `ISSUES.md` changes.
10. If any check fails, fix the split and rerun the whole gate. Do not report the
    file done early.

## 4. Report and continue

Report one short block:

```text
File: <path> (<before> LOC) -> <file:loc, file:loc, ...>
Fn: <n> -> <n> | Calls: <n> -> <n> | Tests: <command> passed
Verifier: PURE MOVE VERIFIED | Next: <path>
```

Then immediately start the next file from step 0 unless the user asked to stop
or the loop is blocked.

## 5. Stop conditions

Stop and ask the user when:

- the file is docs or a non-code asset (skip it and continue instead)
- one function alone exceeds the limit, or the file cannot be split without
  rewriting logic, signatures, or public API paths
- a split requires renaming items, changing behavior, or duplicating code
- unrelated baseline test/clippy failures prevent trustworthy validation
- a required tool (`mk`, `cargo`, `rustfmt`) is unavailable
- the user's work would be overwritten or reverted

When blocked, leave the file untouched or clearly partial, report the exact
command/error, and do not move to the next file.

## 6. Finish the run

After no code files remain in `mk lf`:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo +stable clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
mk lf
```

Report a table of every split file: before LOC, new modules and their LOC,
function/call parity, visibility/re-export changes, and test result. Remaining
`mk lf` entries must be non-code only. Offer a conventional commit message with
the required sign-off but do not commit.
