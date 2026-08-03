# Output Stability Policy

Machine-readable Atlas outputs use explicit version tokens.

Current versions:

- CLI JSON envelope: `atlas_cli.v1`
- MCP JSON body contract: `structuredContent` object payloads plus JSON text mirrors

Published contract artifacts:

- `schemas/atlas_cli.v1/*.schema.json`

## `atlas_cli.v1`

Envelope shape:

```json
{
  "schema_version": "atlas_cli.v1",
  "command": "status|query|context|impact|review_context|explain_change|...",
  "data": {}
}
```

Notes:

- `command` uses stable machine token, not always raw CLI spelling. Example: CLI `review-context` emits `command = "review_context"`.
- JSON object key order is not contract surface.
- Additional optional fields may be added inside existing objects without version bump.

New `atlas_cli.*` version required for any of:

- remove field
- rename field
- change field type
- change field meaning incompatibly
- change enum token incompatibly
- change top-level envelope keys or semantics
- change `command` token for existing command

No version bump required for:

- new optional field
- tighter docs
- new schema examples
- additive command that uses its own new `command` token without changing existing ones

## MCP JSON output

Atlas MCP responses are JSON-only.

`structuredContent` is source of truth for machine-readable payloads. Text content is a JSON serialization of the same payload for transports and clients that expect text blocks.

Changes that remove fields, rename fields, change field types, or change field meaning incompatibly require coordinated schema and contract updates.
