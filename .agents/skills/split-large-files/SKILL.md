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

- `mod` declarations and `pub use` re-exports
- visibility modifiers needed for cross-module access (`pub(crate)`,
  `pub(super)`)
- `use` lines, including adding `use super::*;` to a new child module
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

### 1.6 Move and format

1. Cut blocks verbatim, including doc comments, attributes, and internal order.
2. Paste into the new module in the same relative order.
3. Add only the allowed `mod`/`pub use`/visibility/`use` edits.
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
4. Confirm visibility edits are only those required for compilation; public
   paths are preserved by `pub use`.
5. Confirm test names and assertions are unchanged and only their module path
   moved.
6. Confirm no unrelated files, docs, fixtures, or `ISSUES.md` changes.
7. If any check fails, fix the split and rerun the whole gate. Do not report the
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
function/call parity, and test result. Remaining `mk lf` entries must be
non-code only. Offer a conventional commit message with the required sign-off
but do not commit.
