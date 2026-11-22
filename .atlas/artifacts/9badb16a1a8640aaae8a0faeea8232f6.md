---
id: 9badb16a1a8640aaae8a0faeea8232f6
label: Feedback FTS trigger fix + migration state
source_type: mcp_artifact
session_id: 37c572fbdf19010b928e654c054fed12c4fec03cb7ceb1c1be3703963fc02bfb
agent_id: null
content_type: text/plain
identity_kind: artifact_label
identity_value: Feedback FTS trigger fix + migration state
repo_roots:
  - /home/pastel/Projects/atlas-context-router-rs
repo_ids: []
created_at: 2026-11-20T12:32:24.309325234Z
---
# Migration run + feedback FTS trigger bug fix — 2026-11-20

## Post-install migration state
- New binary 1.7.12 (git c8aef87) live: 82 tools incl. memory_link, updated skill guidance confirmed.
- `atlas migrate markdown --dry-run`: 7 legacy memory rows pending; real run reconciled them (records_written=120 across families); re-run pending=0 (idempotent).
- [REDACTED] already markdown truth from earlier phases.

## Bug found during verification
`atlas doctor` warned: all 104 `.atlas/feedback/*.md` records failed markdown import with generic "SQL logic error".

### Root cause
Migration 010 created `feedback_records_fts` as a REGULAR (contentful) fts5 table, but the `feedback_records_ad`/`_au` triggers use the FTS5 'delete' command form — only legal on contentless/external-content tables (migration 013's memories_fts does it correctly with `content='memories'`). Result: every UPDATE/DELETE on `feedback_records` failed with generic SQLITE_ERROR. Dormant before markdown storage because the write path only INSERTed; the import upsert (UPDATE branch) exposed it on every store open. Reproduced on a session.db copy: plain UPDATE errored; manual fts DELETE/INSERT fine; integrity-check fine.

### Fix
- New migration `[REDACTED].sql`: drops triggers + fts table, recreates as external-content (`content='feedback_records'`, matching 013 pattern), rebuilds index, restores original trigger bodies (now valid).
- Registered version 20 in `packages/atlas-session/src/migrations.rs`, LATEST_VERSION 19→20.
- Validated SQL against live session.db copy: UPDATE/DELETE succeed, FTS stays in sync, integrity-check green.
- Regression test `[REDACTED]` in [REDACTED].rs (UPDATE via import path, search, DELETE, FTS integrity).
- Golden doctor.json: session=19/19 → 20/20.

## Validation
- atlas-session 234/234; atlas-mcp 712/712; atlas-cli 280 + 283/283; clippy clean; fmt applied.

## User follow-up (order matters)
1. Rebuild + reinstall atlas binary (must include migration 020).
2. Do NOT manually stamp user_version on the live DB — old binary fail-closes on higher version; migration 020 applies automatically on next store open with the new binary.
3. Restart editor so the broker serves the new binary.
4. `atlas doctor` + `atlas db-check` — feedback warnings gone.
5. `atlas migrate markdown --dry-run` → pending 0 expected; `--finalize` optional after verifying tree.