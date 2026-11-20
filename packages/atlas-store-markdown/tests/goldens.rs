//! Golden round-trip coverage for every continuity record family.
//!
//! Each golden is canonical output: parse → serialize must reproduce the exact
//! bytes. These goldens are the schema contract for later phases.

use std::path::Path;

use atlas_store_markdown::MarkdownRecord;

fn assert_golden(name: &str, golden: &str) {
    let path = Path::new(name);
    let record = MarkdownRecord::parse(golden.as_bytes(), path)
        .unwrap_or_else(|error| panic!("{name}: parse failed: {error}"));
    let bytes = record
        .to_bytes(path)
        .unwrap_or_else(|error| panic!("{name}: serialize failed: {error}"));
    assert_eq!(
        std::str::from_utf8(&bytes).expect("utf8"),
        golden,
        "{name}: parse-then-serialize must be byte-identical"
    );
}

const MEMORY: &str = "---
id: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
title: Storage direction markdown truth
topic: architecture
scope: project
importance: high
tags:
  - storage
  - sqlite
created_at: 2026-11-19T13:26:36Z
updated_at: 2026-11-19T13:26:36Z
source_id: fb86b7e8c8452f7f76e0ad2bbd8ad1a7
links:
  - { id: abc123, relation: refines }
metadata: { category: decision }
---
Body text.
";

const DECISION: &str = "---
decision_id: d-001
session_id: s-001
query_text: how to store continuity records
source_ids:
  - fb86b7e8c8452f7f76e0ad2bbd8ad1a7
related_files:
  - docs/PLAN-MARKDOWN-STORAGE.md
related_symbols:
  - packages/atlas-session/src/store/memory.rs::fn::store_memory
created_at: 2026-11-19T13:26:36Z
updated_at: 2026-11-19T13:26:36Z
---
## Summary
Markdown is truth.
## Rationale
Version control and corruption recovery.
## Conclusion
Ship the migration.
";

const FEEDBACK: &str = "---
id: f-001
tool_name: query_graph
analysis_kind: dead_code
related_symbol: packages/atlas-core/src/health.rs::fn::classify_graph_store_error
related_file: packages/atlas-core/src/health.rs
created_at: 2026-11-19T13:26:36Z
---
## Predicted
Caller included.
## Actual
Caller missing.
## Correction
Rank distance-one callers first.
";

const MEMOIR_GRAPH: &str = "---
id: g-001
scope: project
name: atlas-architecture
description: Concept map for storage decisions
created_at: 2026-11-19T13:26:36Z
updated_at: 2026-11-19T13:26:36Z
metadata: {}
---
";

const MEMOIR_CONCEPT: &str = "---
id: c-001
name: markdown-truth
kind: concept
relations:
  - { relation: depends_on, target: derived-index, confidence: 1.0 }
---
Markdown is the source of truth.
";

const OVERVIEW: &str = "---
id: o-001
subject_id: sub-001
subject_kind: file
canonical_file_path: docs/PLAN-MARKDOWN-STORAGE.md
title: Markdown continuity plan
commit_sha: abc123def456
source_kind: manual
confidence: 0.8
created_at: 2026-11-19T13:26:36Z
updated_at: 2026-11-19T13:26:36Z
---
## Abstract (L0)
Plan for markdown truth.
## Overview (L1)
- responsibilities
- invariants
";

const TASK: &str = "---
id: t-001
title: Implement M0
status: open
created_at: 2026-11-19T13:26:36Z
updated_at: 2026-11-19T13:26:36Z
---
- [ ] add crate
- [x] run gates
";

const EXPERIENCE: &str = "---
id: e-001
source_uri: atlas://symbol/store_memory
target_uri: atlas://file/memory.rs
query_text: storage
use_count: 2
confidence: 0.9
created_at: 2026-11-19T13:26:36Z
updated_at: 2026-11-19T13:26:36Z
---
Edge note.
";

const ARTIFACT: &str = "---
source_id: a-001
source_type: review_context
label: Plan review
session_id: s-001
created_at: 2026-11-19T13:26:36Z
symbols:
  - store_memory
files:
  - docs/PLAN-MARKDOWN-STORAGE.md
---
Raw payload text.
";

const SESSION: &str = "---
session_id: s-001
repo_root: /home/pastel/Projects/atlas-context-router-rs
frontend: mcp
created_at: 2026-11-19T13:26:36Z
updated_at: 2026-11-19T13:26:36Z
event_count: 2
---
### e1 2026-11-19T13:23:36Z session-start p=4
Event body.
";

#[test]
fn memory_golden_round_trips() {
    assert_golden("memories/architecture/memory.md", MEMORY);
}

#[test]
fn decision_golden_round_trips() {
    assert_golden("decisions/decision.md", DECISION);
}

#[test]
fn feedback_golden_round_trips() {
    assert_golden("feedback/feedback.md", FEEDBACK);
}

#[test]
fn memoir_graph_golden_round_trips() {
    assert_golden("memoir/atlas-architecture/_graph.md", MEMOIR_GRAPH);
}

#[test]
fn memoir_concept_golden_round_trips() {
    assert_golden(
        "memoir/atlas-architecture/markdown-truth.md",
        MEMOIR_CONCEPT,
    );
}

#[test]
fn overview_golden_round_trips() {
    assert_golden("overview/file/sub-001.md", OVERVIEW);
}

#[test]
fn task_golden_round_trips() {
    assert_golden("tasks/t-001.md", TASK);
}

#[test]
fn experience_golden_round_trips() {
    assert_golden("experience/e-001.md", EXPERIENCE);
}

#[test]
fn artifact_golden_round_trips() {
    assert_golden("artifacts/a-001.md", ARTIFACT);
}

#[test]
fn session_golden_round_trips() {
    assert_golden("sessions/s-001/events.md", SESSION);
}

#[test]
fn all_goldens_reparse_to_equal_values() {
    for (name, golden) in [
        ("memory", MEMORY),
        ("decision", DECISION),
        ("feedback", FEEDBACK),
        ("memoir_graph", MEMOIR_GRAPH),
        ("memoir_concept", MEMOIR_CONCEPT),
        ("overview", OVERVIEW),
        ("task", TASK),
        ("experience", EXPERIENCE),
        ("artifact", ARTIFACT),
        ("session", SESSION),
    ] {
        let path = Path::new(name);
        let first = MarkdownRecord::parse(golden.as_bytes(), path).expect("parse");
        let reencoded = first.to_bytes(path).expect("encode");
        let second = MarkdownRecord::parse(&reencoded, path).expect("re-parse");
        assert_eq!(first, second, "{name}: value stability");
    }
}
