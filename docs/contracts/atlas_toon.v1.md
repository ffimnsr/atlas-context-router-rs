# `atlas_toon.v1`

Versioned Atlas TOON contract for MCP tool bodies.

Status:
- active
- version token: `atlas_toon.v1`
- MIME: `text/x-toon`

## Scope

Atlas supports deterministic TOON rendering for machine-facing MCP payloads.
Supported value kinds:

- objects
- arrays
- strings
- numbers
- booleans
- `null`

Atlas does not promise full arbitrary TOON producer behavior. Atlas promises
stable rendering for subset above plus rules below.

## Atlas Rules

1. Object keys sort ascending before encode.
2. Encoder output must round-trip through TOON decode back to same normalized JSON value.
3. Empty root object is invalid for TOON body. Atlas falls back to JSON.
4. Primitive arrays may render inline.
5. Uniform arrays of primitive-only objects may render in tabular form.
6. Mixed or nested arrays may render in expanded block form.
7. TOON fallback reason stays in MCP metadata fields, not inside TOON body.

## Examples

Primitive array:

```toon
tags[3]: reading,gaming,coding
```

Uniform object rows:

```toon
users[2]{active,id,name}:
  true,1,Alice
  false,2,Bob
```

## Versioning

New `atlas_toon.*` version required for any of:

- syntax change
- escaping change
- key ordering change
- round-trip acceptance rule change
- structural rendering change that alters valid output for same normalized JSON value

Additive docs, more examples, or stronger tests do not require new version.
