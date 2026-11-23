---
name: execute-patch-phases
description: Execute a phased ISSUES.md patch or roadmap section one phase at a time, with full implementation and wiring, targeted tests, a mandatory second-pass review, a durable Atlas memory checkpoint, host agent compaction when available, and automatic continuation until every phase and completion criterion in that section is done. Use when asked to implement or finish a phase, run all phases in a section such as External Grammar Offload Patch, or work through an ISSUES.md checklist end to end.
---

# execute-patch-phases

Run one phased section of `ISSUES.md` to completion. Work exactly one phase at
a time. Each phase gets implementation, wiring proof, targeted validation, a
mandatory second pass, then a durable memory checkpoint and host compaction
before the next phase starts.

## Hard rules

- One phase at a time. Never start the next phase until the current phase is
  fully ticked, verified, and checkpointed.
- Never mark a checklist item `[x]` during the implementation pass. Tick only
  after the second pass proves the item with concrete evidence.
- "Fully implemented" means wired end to end, reachable from the real runtime
  path, tested at every seam, and documented where the phase requires it. A
  helper that exists but is never called is not done.
- No stubs, `todo!()`, `unimplemented!()`, ignored tests, skipped tests, dead
  branches, or silent fallbacks.
- Follow `AGENTS.md`. Do not commit, do not move work to `SHIPPED.md`, and do
  not run destructive git commands unless the user explicitly asks.
- Keep every touched module below 1,000 LOC. Split before growing a file past
  the threshold.
- Atlas first for code discovery. Use graph tools before `grep`, file search,
  or broad reads. Exact files named by the user or already identified by Atlas
  may be read directly.
- Do not fix unrelated bugs or broken baseline tests. Report them instead.
- If the phase cannot be completed safely, stop with evidence. Do not tick a
  blocked item and do not continue to the next phase.

## 1. Select the target section

1. Locate the section the user named in `ISSUES.md`. It may be under
   `## Follow-Up Patches`, a roadmap phase section, or another headed block.
   If the target is ambiguous, ask; do not guess between sections.
2. Read the section's `Plan:` link, `Rules`, scope paragraph, phase headings,
   and `Patch completion criteria` (or equivalent acceptance block).
3. Treat each `#### Phase N — ...`, `#### Patch X — ...`, or similar headed
   checklist block as one phase unit. Do not treat the completion-criteria
   block or a rejected/non-goal block as a phase.
4. If the user named a specific phase, process only that phase. If the user
   asked for the whole section, process every phase unit in document order.
5. Read the linked plan only for the sections relevant to the current phase.
   Do not reload the whole plan every phase unless the phase spans it.

## 2. Start or resume the session

At the start of the session, before code exploration:

- Call `wake_up` once with the section topic if this session has not already
  done so.
- Recall prior phase memory:
  `memory_recall` with the section slug and phase tag, or
  `atlas memory recall --topic "issue-patch:<section-slug>" --json`.
- Read `current_focus`, recent decisions, changed files, and graph readiness
  from the wake-up pack. Do not re-derive facts already checkpointed.
- Call `status` only when graph readiness is unknown.
- Run `git --no-optional-locks status --short` to see pre-existing changes.
  Never overwrite or revert user work.

Before each later phase, recall the previous phase memory and the section
memory. The checkpoint is the handoff; chat history is not.

## 3. Build the phase contract

Before editing, write a short internal contract:

- phase heading and every checklist item in that phase
- expected runtime wiring path, from input to observable behavior
- files, symbols, config fields, schemas, and tests expected to change
- exact validation commands for the phase
- dependencies on earlier phases, later phases, external binaries, or fixtures
- risks: QN drift, golden changes, public API changes, migration concerns

Use Atlas to resolve symbols and call paths:

- `resolve_symbol`, `query_graph`, or `get_context` for targets
- `symbol_neighbors` or `trace_calls` for callers and callees
- `get_impact_radius` or `explain_change` for blast radius

If the phase needs a minimal prefix of later-phase work to wire correctly,
implement only that prefix. Record the dependency; do not implement the whole
later phase early.

## 4. Implement the phase

Work checklist item by item. For each item:

1. Find existing helpers, modules, types, or config patterns with Atlas first.
   Reuse them when practical.
2. Make the smallest cohesive change that satisfies the item.
3. Wire every applicable seam:
   - input surface: config, CLI flag, MCP argument, schema, env var, or file
   - parsing, defaults, and validation
   - propagation through engine options and registries; no hard-coded empty
     vectors or ignored fields left in intermediate layers
   - runtime selection or dispatch
   - persistence or metadata read/write when the phase stores state
   - observability: `status`, `doctor`, logs, diagnostics, error messages
   - tests at unit, integration, and CLI/MCP boundaries where they exist
   - docs, config templates, examples, migration notes, changelog
4. Add tests with the code, including negative and edge cases named by the
   phase. Assert behavior, not just successful compilation.
5. Preserve existing behavior unless the phase explicitly changes it. Keep
   golden outputs byte-identical when the checklist requires it.
6. Use `Result` and `?` in production paths. Add timeouts, output caps, and
   bounded resource use for external processes, files, and paths.
7. Update the linked plan only if implementation must deviate from its design.
   Record the deviation and rationale in the phase memory.

Do not edit `ISSUES.md` yet.

### Wiring gate

Before leaving implementation, prove each applicable seam:

| Seam | Proof required |
|---|---|
| Input | config/flag/argument is parsed and rejected when invalid |
| Defaults | defaults are applied and tested |
| Propagation | value reaches the consumer; no empty placeholder remains |
| Runtime | real dispatch selects the new behavior |
| Persistence | state is read and written on the intended path |
| Observability | failure and stale states are visible to operators |
| Tests | each seam has a unit or integration assertion |
| Docs | templates, README, and schemas match the code |

Mark a seam not applicable only with a concrete reason. "Not wired" is not an
acceptable final state.

## 5. Run targeted validation

Run the phase's own commands first. Then run the affected package checks:

```bash
cargo fmt --all
cargo test -p <package> <test_name> -- --exact
cargo test -p <package>
cargo clippy -p <package> --all-targets -- -D warnings
```

Use the actual package names and test targets from `Cargo.toml`. Add CLI,
fixture, or release checks when the phase requires them.

If validation fails:

- Fix the root cause, then rerun the same command.
- Do not add retries or longer sleeps to hide a flaky failure.
- If an unrelated baseline failure blocks validation, record the exact command
  and error. Do not fix it unless the user asks.

## 6. Mandatory second pass

This is a separate review pass, not a test rerun. Do it after implementation
and targeted validation. Do not skip it because tests passed.

1. Re-read every checklist item in the phase.
2. For each item, collect evidence: file and line, symbol name, test name, and
   command result. If you cannot point to evidence, the item is not done.
3. Review the full phase diff:
   `git --no-pager diff --stat` and `git --no-pager diff -- <files>`.
4. Check for missing wiring and false completion:
   - config accepted but never forwarded
   - parser/plugin registered but never selected
   - option parsed but ignored
   - build path updated while update or watch path still uses an empty default
   - CLI and MCP surfaces disagree
   - status/doctor cannot report the new failure state
   - helper exists but no production caller
   - tests cover only the happy path or only compilation
5. Check code quality and safety:
   - no dead code, unused imports, or unreachable branches introduced
   - no unwrap/expect on untrusted input or external process failure
   - error context is actionable
   - timeouts, output caps, path validation, and resource limits are present
   - no secrets or sensitive data in logs or memory
   - file-size rule still holds
6. Check documentation and fixtures:
   - docs and config template examples validate against the real schema
   - path-resolution rules match the implementation
   - changelog/README claims match shipped behavior
   - `ISSUES.md` is untouched until the phase passes
7. Run the broader affected test set and rerun `fmt` and `clippy` if the second
   pass changed code.
8. For large or high-risk phases, optionally use `spawn_agent` for a read-only
   independent diff review. Give it the phase checklist, changed files, and
   wiring seams; fix findings yourself.

If the second pass finds gaps, fix them and repeat the second pass. Only then
mark the phase complete.

## 7. Tick and checkpoint

1. Edit `ISSUES.md` and change `- [ ]` to `- [x]` only for items proven in the
   second pass. Preserve item wording, order, and surrounding headings.
2. For negative or audit items ("do not require X", "confirm no dylib path"),
   cite the audit evidence proving the prohibited behavior is absent.
3. Do not tick `Patch completion criteria` until the whole section is done.
4. Save a compact durable memory with `memory_store`:

```text
topic: issue-patch:<section-slug>
title: Phase <N> complete: <phase title>
scope: project
importance: high
tags: ["issues", "<section-slug>", "phase-<N>", "complete"]
text:
  section: <section title>
  phase: <N> — <phase title>
  status: complete
  files: <changed files or modules>
  wiring: <input -> propagation -> runtime -> observable path>
  tests: <commands and results>
  decisions: <deviations, gotchas, follow-ups>
  next: <next phase>
```

Keep the body under roughly 150 words. Do not paste full logs, diffs, or
secrets. If a memory for the same phase already exists, update it with
`patch_id` plus `patch_text` or link a replacement with `supersedes`; do not
create duplicate phase records.

Verify the checkpoint with `memory_recall` before compacting. If recall fails,
fix the memory first.

## 8. Compact the agent context

Compaction here means the host agent's context compaction, not Atlas ledger
cleanup.

- After the phase memory is verified, use the host-provided compaction action
  when this agent exposes one, for example `/compact` or `session/compact`.
- Use the exact mechanism available in the current agent. Do not invent a
  command. If no host compaction action exists, skip compaction and continue;
  the durable memory record is the fallback.
- Do not use Atlas `compact_session` as a substitute unless the user explicitly
  asks for Atlas session-ledger compaction.
- Never compact mid-phase, before tests pass, or before the phase memory is
  saved and recalled successfully.
- If hooks are available, record the pre-compaction event first:
  `record_session_event` with `pre-compact`.
- After compaction, immediately recall the just-saved phase memory and the
  section memory. Do not rely on the compacted transcript for phase facts.

## 9. Continue to the next phase

Unless the user asked for only one phase, continue automatically to the next
phase in the section after the checkpoint and compaction step.

Report progress in one to three lines: phase completed, validation result,
checkpoint saved, next phase. Then start the next phase from step 3, using the
recalled memory instead of re-reading everything.

## 10. Complete the section

After every phase unit is ticked:

1. Run the section-level validation and acceptance commands, such as
   `Phase 9` or `Patch completion criteria`. Typical full gates:
   ```bash
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   cargo build --release
   ```
   Run only gates the section actually requires; record binary-size or
   dependency-graph results when requested.
2. Verify every phase checkbox and every completion-criteria checkbox is
   supported by evidence.
3. Verify cross-phase wiring: build, update, watch, status, doctor, CLI, MCP,
   docs, and templates agree. No phase may leave a hard-coded default or an
   unreachable path behind.
4. Confirm no stale plan claims, stale README examples, or fixture drift.
5. Tick the completion criteria only after the gates pass.
6. Save a section-level memory:
   `title: Section complete: <section title>`, with phases covered, validation
   results, binary-size delta, deviations, and remaining risks.
7. Do not move the section to `SHIPPED.md` and do not commit. Report completion
   and offer both actions. If the user asks to ship, use the `ship-issues`
   skill. If asked for a commit message, provide the conventional header, body,
   and the required sign-off.

## 11. Blocker policy

Stop and ask the user only when:

- an action is destructive or irreversible
- a required grammar, binary, fixture, network resource, or tool is unavailable
- the target section or phase is ambiguous
- an unrelated baseline failure prevents trustworthy validation
- the phase requires a security-sensitive decision not covered by the plan
- the requested scope would require implementing a different phase

When blocked:

- save memory with `status: blocked` and the exact blocker,
- leave affected checklist items unticked,
- report the failing command, error, and the smallest decision needed,
- do not continue to the next phase.

## Final report

Summarize:

- section and phases completed
- files and modules changed
- wiring paths proved
- tests, clippy, fmt, release checks, and results
- deviations from the plan and why
- remaining risks or follow-ups
- commit message and sign-off if requested
