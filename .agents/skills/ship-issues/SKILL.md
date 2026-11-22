---
name: ship-issues
description: Move finished ISSUES.md items into SHIPPED.md. Use when the user asks to record shipped work, move completed checklist items or patch/phase blocks from ISSUES.md to SHIPPED.md, update the Still Open section, or write SHIPPED.md entries for landed work. Triggers include "ship this", "move finished items to SHIPPED", "update SHIPPED.md", "mark this shipped".
---

# ship-issues

Move finished work from `ISSUES.md` into `SHIPPED.md` as high-level capability
summaries. SHIPPED.md is an outcome ledger, not a changelog of phases or code.

## When an ISSUES.md item is "finished"

An item qualifies only when ALL hold:

- Every checklist line under it (including edge cases and acceptance criteria)
  is ticked `[x]`.
- Validation lines (workspace tests, clippy `-D warnings`, fmt) are ticked or
  the user confirms they pass.
- The user confirms the item should ship now, or an explicit instruction names
  it. Never move half-finished work on your own initiative.

If any sub-item is unticked, the item stays in ISSUES.md; mention the remainder
instead of moving.

## Writing the SHIPPED.md entry

Follow the style of existing SHIPPED.md sections (read one or two first):

- One `## <Capability Name>` section per shipped feature area, not per phase
  or per patch. Use a plain descriptive title (for example
  `## Markdown Continuity Storage`), never `Phase 3` or `Patch E3` style
  headings.
- Bullets describe shipped capability and behavior outcomes in present tense:
  "X is implemented", "X does Y". Summarize what the system now does, not the
  journey or the plan.
- NO phase numbers, part numbers, or patch identifiers anywhere in the entry.
  Collapse a whole phased plan (M0–M5, Phase 1–7) into one flat section.
- NO code, snippets, or implementation detail unless strictly needed to name a
  command, tool, file family, or table. Prefer prose: "the migration command
  exports legacy rows and verifies round-trips" beats quoting its flags.
- Keep each bullet one to two lines. Match the density of neighboring bullets.
- If a SHIPPED.md section for the same capability already exists, merge new
  bullets into it instead of adding a duplicate section.
- Never invent shipped behavior: every bullet must be grounded in the ticked
  ISSUES.md content you are moving.

## Updating both files

1. Add the new section to SHIPPED.md. New sections go directly above
   `## Still Open`, which stays the last section.
2. Update `## Still Open` in SHIPPED.md: drop moved items from the open list;
   keep genuinely open remainders.
3. Remove the finished block from ISSUES.md entirely: the checklist items,
   edge cases, acceptance criteria, and the surrounding phase/patch headings.
   Unticked `Additional Backlog` lines stay in ISSUES.md.
4. If a moved plan leaves a partial remainder (for example one language of a
  migration still pending), keep a short ISSUES.md stub describing only the
  remainder, and say so in SHIPPED.md `## Still Open`.
5. Leave ISSUES.md goal, roadmap layout, and non-finished sections untouched.

## Verification before finishing

- ISSUES.md contains no dangling headings for removed blocks (no heading
  whose body was deleted).
- SHIPPED.md still ends with `## Still Open` reflecting the true remainder.
- No `Phase N`, `Part N`, or unchecked-carryover text leaked into SHIPPED.md.
  Grep SHIPPED.md for `Phase` and `Part` as a self-check when moving phased
  plans; only historical section titles that already existed may remain.
