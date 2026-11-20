---
id: b54cefd9aaccbc46ee5d0b4e3082cec9150b80b88c1236ae81bdf7babebac695
repo_root: /home/pastel/Projects/atlas-context-router-rs
session_id: null
tool_name: analyze_architecture
analysis_kind: ""
predicted: CLI data includes counts with cycle_count and layer_violation_count
actual: "counts object absent entirely from CLI data (MCP returns it)"
correction: Add counts + module edges to CLI architecture JSON. packages/atlas-cli/src/commands/insights.rs.
related_symbol: "repo (CLI insights architecture --json)"
related_file: null
source_id: null
created_at: 2026-11-13T09:47:26Z
metadata_json: "{}"
---
## Predicted

CLI data includes counts with cycle_count and layer_violation_count

## Actual

counts object absent entirely from CLI data (MCP returns it)

## Correction

Add counts + module edges to CLI architecture JSON. packages/atlas-cli/src/commands/insights.rs.

