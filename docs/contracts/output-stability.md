# Output Stability Policy

Machine-readable Atlas outputs use explicit version tokens.

Current versions:

- CLI JSON envelope: `atlas_cli.v1`
- MCP TOON body contract: `atlas_toon.v1`

Published contract artifacts:

- `schemas/atlas_cli.v1/*.schema.json`
- `docs/contracts/atlas_toon.v1.md`

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

## `atlas_toon.v1`

`atlas_toon.v1` covers rendered MCP body text only.

Top-level MCP metadata such as `atlas_output_format` and fallback metadata are
outside TOON body contract and version independently from JSON-RPC/MCP surface.

New `atlas_toon.*` version required for any rendered-body compatibility break.
