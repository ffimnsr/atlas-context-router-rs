# Atlas Tokenizer Plan — `atlas-tokenizer`

Status: proposed
Scope: new standalone crate `packages/atlas-tokenizer`, port of OpenAI `tiktoken` core.
Goal: replace default byte heuristic only after parity and stability gates pass.

---

## 1. Current State

Relevant facts:

- `packages/atlas-token-count` (211 LOC) is the only crate touching external tokenizer machinery.
  - `TokenCounter::{heuristic, from_file}`, `count_text`, `count_json_bytes`.
  - `TokenCountMethod::{Tokenizer{provider,model}, HeuristicBytes{bytes_per_token}}`.
  - Dependency: `tokenizers = "0.23"` with `fancy-regex` backend.
- Config surface: `[context.tokenizer]` in `packages/atlas-engine/src/config/context.rs`.
  - `provider = "heuristic" | "tokenizers"`, `model`, `tokenizer_file`, `fallback`, `bytes_per_token`.
- Runtime builder: `Config::token_counter` in `packages/atlas-engine/src/config/load.rs`.
- Accounting output: `build_token_accounting_meta` in `packages/atlas-review/src/context/payload.rs`.
- Template rendering: `packages/atlas-engine/src/config/template.rs`.
- Docs: `README.md` §context.tokenizer, `SHIPPED.md` token accounting entries.
- Gates: `packages/atlas-cli/tests/cli_quality_gates/tokenizer_budget.rs`,
  `packages/atlas-mcp/src/tools/tests/context_ops/tokenizer_budget.rs`.
- `deny.toml` ignores `RUSTSEC-2024-0436` (`paste` via `tokenizers`). Removing `tokenizers` removes that advisory and the `onig`/`fancy-regex` backend choice.
- Default provider remains `heuristic`: `bytes.div_ceil(4)`.

Current gaps:

- No OpenAI-compatible BPE. HuggingFace JSON tokenizers are not the same encoding as `cl100k_base` / `o200k_base`.
- Default accounting does not reflect real model tokens, so budget truncation is approximate.
- External `tokenizers` dependency is heavy and carries an ignored advisory.

---

## 2. Goals and Non-Goals

Goals:

- Standalone crate `atlas-tokenizer` with no `atlas-*` dependencies.
- Byte-for-byte encode and decode parity with official OpenAI `tiktoken` at a pinned upstream commit.
- Supported encodings: `r50k_base`, `p50k_base`, `p50k_edit`, `cl100k_base`, `o200k_base`, optional `o200k_harmony`.
- Model-to-encoding mapping matching upstream `tiktoken/model.py` prefix rules.
- Fully offline at runtime. Vocab assets vendored, checksummed, no network calls.
- `atlas-token-count` gains a `tiktoken` provider; existing heuristic remains fallback.
- Deterministic, `Send + Sync`, no `unsafe`, no panics on untrusted input.
- MSRV 1.95, edition 2024, workspace lints.

Non-goals (v1):

- Python bindings or `tiktoken_ext` plugin discovery.
- Runtime model metadata fetch.
- HuggingFace `tokenizer.json` support inside the new crate (legacy provider keeps it until removed).
- BPE training (`tiktoken/train.py`). Optional later feature.
- Exact performance parity with the upstream Rust extension on very large batches.

---

## 3. Target Architecture

```
packages/atlas-tokenizer/
  Cargo.toml
  README.md
  LICENSE-TIKTOKEN            # upstream MIT notice + vendored-data attribution
  assets/
    manifest.json             # upstream commit, file sha256, encoding names, special tokens
    encodings/
      r50k_base.tiktoken
      p50k_base.tiktoken
      p50k_edit.tiktoken
      cl100k_base.tiktoken
      o200k_base.tiktoken
      o200k_harmony.tiktoken  # optional feature
  src/
    lib.rs                    # re-exports, crate docs, feature docs
    error.rs                  # typed errors (thiserror)
    rank_file.rs              # .tiktoken parser + validation
    regex_split.rs            # fancy-regex wrapper, Unicode-aware split
    bpe.rs                    # core byte-pair encode/decode primitives
    encoding.rs               # encoding definitions: pattern, specials, assets
    model.rs                  # model -> encoding prefix map
    registry.rs               # EncodingRegistry / static lazy access
    batch.rs                  # optional rayon batch encode (feature = "batch")
  tests/
    parity.rs                 # golden vectors from official tiktoken
    roundtrip.rs              # decode(encode(x)) == x
    special_tokens.rs
    model_map.rs
    asset_checksum.rs
    golden/
      <encoding>.jsonl
  benches/
    encode.rs                 # criterion, baselines under benches/baselines/
```

Module budget: every file stays below 1,000 LOC. Split `encoding.rs` into
`encoding/{defs,cl100k,o200k,...}.rs` if it approaches the threshold.

### 3.1 Dependencies

Workspace-level additions:

- `fancy-regex` — tiktoken patterns require lookahead. Do not substitute `regex`.
- `base64` — decode `.tiktoken` rank lines.
- `thiserror` — typed errors.
- `sha2` — asset checksum verification in tests and optional strict runtime mode.
- `rayon` — optional, behind `batch` feature, off by default.
- `criterion` — dev only, bench.

No `reqwest`, no `tokio`, no `atlas-*`. Crate must build with
`cargo check -p atlas-tokenizer --no-default-features`.

Feature layout:

- `encoding-r50k`, `encoding-p50k`, `encoding-cl100k`, `encoding-o200k` (default on), `encoding-o200k-harmony` (off).
- `vendored` (default on) — embed assets with `include_bytes!`.
- `file-assets` (default on) — allow `Encoding::from_file` for custom `.tiktoken`.
- `batch` (off) — rayon batch API.
- `verify-assets` (off) — runtime sha256 check; tests always verify.

### 3.2 Public API Sketch

```rust
pub struct Encoding { /* immutable */ }

impl Encoding {
    pub fn get(name: EncodingName) -> Result<&'static Encoding>;
    pub fn for_model(model: &str) -> Result<&'static Encoding>;
    pub fn from_rank_file(path: &Path) -> Result<Encoding>;
    pub fn from_rank_bytes(bytes: &[u8], spec: EncodingSpec) -> Result<Encoding>;

    pub fn encode_ordinary(&self, text: &str) -> Vec<Rank>;
    pub fn encode(&self, text: &str, allowed_special: &[&str]) -> Result<Vec<Rank>>;
    pub fn encode_with_special_tokens(&self, text: &str) -> Result<Vec<Rank>>;
    pub fn decode(&self, tokens: &[Rank]) -> Result<String>;
    pub fn decode_bytes(&self, tokens: &[Rank]) -> Result<Vec<u8>>;
}

pub struct EncodingRegistry { /* explicit, testable */ }
pub fn default_registry() -> &'static EncodingRegistry;
```

Rules:

- `Encoding` immutable after construction, `Send + Sync`.
- `default_registry` uses `OnceLock` for immutable data only.
- Tests construct `EncodingRegistry` explicitly; no global mutation.
- No `unwrap`/`expect` on parsing or encoding paths.
- `#![forbid(unsafe_code)]` and `#![deny(missing_docs)]` in `lib.rs`.

### 3.3 Core BPE Contract

Rank file parsing (`rank_file.rs`):

- Format: `base64(token_bytes) <rank>` per line.
- Accept LF and CRLF; skip blank lines; skip `#` comment/header lines.
- Reject invalid base64, invalid rank, duplicate token, duplicate rank, rank above a configurable cap.
- Cap line length and total entries to avoid memory bombs on untrusted files.
- Build:
  - `mergeable_ranks: HashMap<Vec<u8>, Rank>`
  - `decoder: Vec<Option<Vec<u8>>>` indexed by rank
  - pair lookup map `(u32, u32) -> Rank` built lazily or eagerly behind a flag.
- Eager pair map costs memory; measure. If too large, lazy per-encoding cache.

Encode (`bpe.rs`):

1. Pre-scan input for special-token literals (longest match first) when specials are allowed.
2. Split ordinary segments with the encoding regex via `fancy-regex`.
3. For each segment: direct rank lookup fast path, else byte-pair merge loop:
   - start from single-byte ranks,
   - find lowest-rank adjacent pair,
   - merge until no pair has a rank.
4. Convert to `Rank` (`u32`) output.

Decode:

- Map ranks back to bytes; concatenate.
- `decode_bytes` strict on unknown ranks.
- `decode` mirrors Python default `errors="replace"`; add `decode_strict` if needed.
- Pin exact special-token decode behavior in parity tests instead of guessing.

Special tokens:

- Per-encoding map `HashMap<String, Rank>` plus reverse map.
- `encode` errors when input contains a special literal not in `allowed_special`, matching upstream semantics.
- `encode_ordinary` treats special literals as ordinary text.
- `encode_with_special_tokens` allows all encoding specials.
- Reject empty special token strings at construction.

---

## 4. Encoding Definitions

Pin upstream commit in `assets/manifest.json` and in crate docs.

Encodings and required data:

| Encoding | Pattern family | Specials |
|---|---|---|
| `r50k_base` | GPT-2 split | `<|endoftext|>` |
| `p50k_base` | GPT-2 split | `<|endoftext|>` |
| `p50k_edit` | GPT-2 split | `endoftext`, `fim_prefix`, `fim_middle`, `fim_suffix` |
| `cl100k_base` | cl100k split | `endoftext`, `fim_*`, `endofprompt` |
| `o200k_base` | o200k split | `endoftext`, `endofprompt` |
| `o200k_harmony` | o200k split | harmony reserved specials (feature-gated) |

Implementation rules:

- Copy exact regex pattern strings from upstream into constants with source URL and commit.
- Do not “simplify” patterns. Parity tests will catch drift.
- Model map is ordered longest-prefix-first, mirroring upstream `MODEL_PREFIX_TO_ENCODING`.
- Unknown model returns `Error::UnknownModel`; explicit encoding wins over model inference.
- `model` metadata is preserved for accounting even when encoding is explicit.

---

## 5. Assets and Supply Chain

Decision: vendor vocab files in-repo, no runtime download.

Rationale:

- Current docs promise local-only tokenizer inputs, never downloaded.
- CI and air-gapped installs must work.
- Deterministic output across machines.

Assets:

- Download from the official `openaipublic.blob.core.windows.net/encodings/` URLs listed by upstream at the pinned commit.
- Commit `.tiktoken` files under `assets/encodings/`.
- Record per-file `sha256`, byte size, upstream URL, upstream commit in `assets/manifest.json`.
- `LICENSE-TIKTOKEN` includes upstream MIT text and OpenAI copyright; crate remains `MIT OR Apache-2.0`.
- Maintainer-only refresh script: `scripts/fetch-tiktoken-assets.sh` (curl + sha256 + manifest update). Never run in CI or at build time.
- Expected repo growth: roughly 5–9 MB for all encodings; default feature set about 5 MB. If repo size becomes a concern, use Git LFS for `assets/encodings/*.tiktoken`; keep manifest in normal git.

Validation:

- `asset_checksum.rs` test recomputes sha256 for every embedded asset and fails on mismatch.
- `vendored` off: encodings resolve from explicit file paths only; model inference returns `Error::EncodingUnavailable`.
- `verify-assets` runtime feature is opt-in; default avoids per-start hashing cost.

---

## 6. Integration Plan

### 6.1 `atlas-token-count`

Add without breaking current call sites:

```rust
pub enum TokenCounter {
    Tokenizer { /* existing HF path */ },
    Tiktoken { encoding: Arc<atlas_tokenizer::Encoding>, model: Option<String> },
    Heuristic { bytes_per_token: usize },
}

pub enum TokenCountMethod {
    Tokenizer { provider: String, model: Option<String> },
    Tiktoken { encoding: String, model: Option<String> },
    HeuristicBytes { bytes_per_token: usize },
}
```

Constructors:

- `TokenCounter::from_encoding(name, model)`
- `TokenCounter::from_model(model)`
- `TokenCounter::from_tiktoken_file(path, model)`
- Keep `from_file` for HF JSON until removal phase.

Accounting mapping:

- `Tiktoken` maps to `provider = "tiktoken"`, `encoding = <name>`, `model`.
- `payload.rs` adds optional `encoding` field. Additive; keep existing fields unchanged.
- `payload_accounting_text` includes encoding when present.

### 6.2 Config

Add provider:

```toml
[context.tokenizer]
provider = "tiktoken"          # heuristic | tokenizers | tiktoken
encoding = "o200k_base"        # optional when model resolves it
model = "gpt-4o"               # optional metadata + inference
# tokenizer_file = "tokenizers/custom.tiktoken"  # optional custom tiktoken file
fallback = "heuristic"         # heuristic | fail_closed
bytes_per_token = 4
```

Validation matrix:

- `heuristic`: `tokenizer_file` and `encoding` must be unset.
- `tokenizers`: `tokenizer_file` required; `encoding` must be unset.
- `tiktoken`: at least one of `encoding`, `model`, `tokenizer_file`; if both `model` and `encoding`, they must agree after model resolution, otherwise error naming both keys.
- Unknown encoding name errors at config load with the config key in the message.
- Keep `fallback` semantics identical: load failure → heuristic or fail closed.

Touch points:

- `packages/atlas-engine/src/config/context.rs` — enum variant, field, validation.
- `packages/atlas-engine/src/config/load.rs` — build counter, relative file resolution.
- `packages/atlas-engine/src/config/template.rs` — commented tiktoken example; default config stays heuristic until Phase 7.
- `packages/atlas-engine/src/config/tests/{mod,template}.rs` — validation and snapshot tests.
- `packages/atlas-cli/src/commands/maintenance.rs` — `config_runtime_json` gains `encoding`.
- `README.md` tokenizer section — new provider and precedence rules.

### 6.3 Default Switch (Phase 7 only)

After gates, change `ContextTokenizerConfig::default()`:

- provider `heuristic` → `tiktoken`
- encoding: `o200k_base`
- fallback: `heuristic` (safe fallback remains)
- `bytes_per_token` stays for fallback and explicit heuristic mode.

Do not switch default while `tokenizers` provider is still the only alternative for HF JSON files.

---

## 7. Test and Quality Gates

### 7.1 Parity Corpus

Generate goldens with official Python `tiktoken` pinned to the same upstream commit
in a maintainer script; commit JSONL, never run Python in CI:

```
packages/atlas-tokenizer/tests/golden/<encoding>.jsonl
{ "text": "...", "tokens": [1,2,3], "bytes": "...", "special": false }
```

Corpus requirements per encoding:

- empty string, single ASCII char, long ASCII
- Unicode: Latin, CJK, Arabic, Devanagari, combining marks
- emoji and ZWJ sequences
- whitespace runs, tabs, CRLF, lone `\r`, trailing spaces
- contractions and apostrophe variants
- numbers 1–4+ digits
- code snippets: Rust, Python, JSON, SQL, Markdown
- special-token literals, allowed and disallowed
- long repeated text (merge-loop stress)
- text containing every byte value via Unicode escapes

### 7.2 Test Matrix

- `parity.rs`: exact rank-vector equality for all default encodings.
- `roundtrip.rs`: `decode(encode_ordinary(x)) == x` for corpus and proptest-generated Unicode strings.
- `special_tokens.rs`: allowed/disallowed semantics, longest-match precedence, decode behavior.
- `model_map.rs`: every upstream model prefix resolves correctly; unknown model errors.
- `asset_checksum.rs`: embedded assets match manifest sha256.
- `rank_file.rs` unit tests: malformed base64, duplicate ranks, oversized lines, comments, CRLF.
- `bpe.rs` unit tests: empty, single byte, no-merge, deep merge, max token length.
- Integration: CLI + MCP tokenizer tests extended with `provider = "tiktoken"` and exact expected counts.
- Fuzz target under workspace `fuzz/`: arbitrary `&str` and malformed rank files; assert no panic and roundtrip.
- Bench: criterion encode/decode throughput per encoding; store baseline under `benches/baselines/`.

### 7.3 Stability Gates Before Default Flip

All must hold:

1. 100% golden parity for `cl100k_base` and `o200k_base`; zero known mismatches.
2. Roundtrip and fuzz suites green; no panic found.
3. No network dependency: crate graph has no HTTP client; tests pass with networking disabled.
4. Startup: lazy encoding init p95 ≤ 25 ms on CI baseline hardware.
5. Encode throughput: no more than 20% regression versus recorded crate baseline; context build overhead ≤ 5 ms for a typical 16 KiB payload.
6. Config fallback and fail-closed tests green for all providers.
7. Two consecutive minor releases with provider `tiktoken` opt-in and no parity regression reports.
8. `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo fmt --all`, `cargo deny check` green.
9. Docs, template, README, and `SHIPPED.md` updated; docs drift guard green.

### 7.4 Test Execution Rules

- Iterate with narrow filters: `cargo test --quiet -p atlas-tokenizer <module>`.
- Run full package once before handoff.
- Serialize heavy workspace runs.
- Avoid fixed sleeps; no shared cwd/env mutation.

---

## 8. Phases

### Phase 0 — Decisions and Sign-Off

- [ ] Confirm vendored assets in git (recommended) vs build-time fetch.
- [ ] Confirm default encodings: `cl100k_base` + `o200k_base` (recommended).
- [ ] Confirm default encoding for Phase 7: `o200k_base` (recommended).
- [ ] Confirm removal timeline for `provider = "tokenizers"` and the `tokenizers` dependency.
- [ ] Confirm `o200k_harmony` inclusion.
- [ ] Pin upstream tiktoken commit/tag in the plan.

Exit: decisions recorded in this doc.

### Phase 1 — Crate Scaffold

- [ ] Create `packages/atlas-tokenizer` with `Cargo.toml`, `README.md`, `LICENSE-TIKTOKEN`.
- [ ] Add workspace member and `[workspace.dependencies]` entries.
- [ ] Add `#![forbid(unsafe_code)]`, `#![deny(missing_docs)]`, workspace lints.
- [ ] Add empty modules with doc comments and feature flags.
- [ ] Add `cargo check -p atlas-tokenizer --no-default-features` gate.

Exit: crate builds empty, clippy clean.

### Phase 2 — Rank File and BPE Core

- [ ] Implement `rank_file.rs` parser and validation caps.
- [ ] Implement `bpe.rs` byte-pair merge encode and decode.
- [ ] Add unit tests for malformed input and merge edge cases.
- [ ] Add property test: `decode_bytes(encode_ordinary(x)) == x.as_bytes()` for Unicode corpus.

Exit: BPE core correct without encoding definitions.

### Phase 3 — Encoding Definitions and Model Map

- [ ] Add `EncodingSpec` constants with exact upstream regex patterns and specials.
- [ ] Implement `encoding.rs` construction per encoding.
- [ ] Implement `model.rs` longest-prefix map.
- [ ] Implement `registry.rs` with explicit registry and static default.
- [ ] Add `special_tokens.rs` and `model_map.rs` tests.

Exit: encode/decode works for all encodings using file-loaded ranks.

### Phase 4 — Assets and Offline Packaging

- [ ] Add `scripts/fetch-tiktoken-assets.sh` with URL list, sha256, manifest generation.
- [ ] Vendor default encoding assets; add `o200k_harmony` behind feature.
- [ ] Implement `include_bytes!` asset lookup and manifest validation.
- [ ] Add `asset_checksum.rs`.
- [ ] Verify offline build and run with no network.

Exit: default build embeds assets; no runtime I/O needed.

### Phase 5 — Parity and Bench

- [ ] Add maintainer golden generator using official Python tiktoken at pinned commit.
- [ ] Commit golden JSONL for all supported encodings.
- [ ] Implement `parity.rs`; fix all mismatches.
- [ ] Add criterion bench and record baselines.
- [ ] Add fuzz target.
- [ ] Run parity, roundtrip, fuzz, clippy, fmt, deny.

Exit: Phase 7 gate items 1–5 pass.

### Phase 6 — `atlas-token-count` and Config Integration

- [ ] Add `Tiktoken` variants to `TokenCounter` and `TokenCountMethod`.
- [ ] Add constructors and update `count_text` / `count_json_bytes`.
- [ ] Update `atlas-engine` config enums, validation, loader, template.
- [ ] Update `payload.rs` accounting with optional `encoding`.
- [ ] Update `payload_accounting_text` and `maintenance.rs` runtime JSON.
- [ ] Update README, SHIPPED, config docs.
- [ ] Add config tests: valid matrix, mismatch errors, unknown encoding, fallback, fail closed.
- [ ] Extend CLI and MCP tokenizer quality gates with tiktoken cases.

Exit: opt-in `provider = "tiktoken"` works end to end; heuristic still default.

### Phase 7 — Default Flip and Cleanup

- [ ] Change `ContextTokenizerConfig::default()` to tiktoken + `o200k_base`.
- [ ] Update template, README, docs drift guard.
- [ ] Keep heuristic fallback; verify fallback metadata.
- [ ] Deprecate or remove `provider = "tokenizers"` per Phase 0 decision.
- [ ] Remove `tokenizers` dependency and the `paste` advisory ignore in `deny.toml`.
- [ ] Update `SHIPPED.md` and changelog.
- [ ] Run full workspace tests, clippy, fmt, deny, and bench regression check.

Exit: default token accounting is OpenAI-compatible and offline.

### Phase 8 — Optional Follow-Ups

- [ ] BPE training helper behind `train` feature, mirroring `tiktoken/train.py`.
- [ ] `o200k_harmony` default-feature decision based on model usage.
- [ ] Batch encode tuning and memory profiling.
- [ ] Optional encoding registry injection in long-lived services if startup cost matters.

---

## 9. File Change Map

New:

- `packages/atlas-tokenizer/**` (see §3)
- `scripts/fetch-tiktoken-assets.sh`
- `docs/PLAN-TOKENIZER.md` (this file)

Modified:

- `Cargo.toml` — workspace member and dependencies.
- `Cargo.lock` — regenerated.
- `packages/atlas-token-count/Cargo.toml` — add `atlas-tokenizer`.
- `packages/atlas-token-count/src/lib.rs` — new counter variant, metadata, tests.
- `packages/atlas-engine/src/config/context.rs` — provider, encoding, validation.
- `packages/atlas-engine/src/config/load.rs` — build tiktoken counter.
- `packages/atlas-engine/src/config/template.rs` — commented example.
- `packages/atlas-engine/src/config/tests/mod.rs` — config tests.
- `packages/atlas-engine/src/config/tests/template.rs` — snapshots.
- `packages/atlas-review/src/context/payload.rs` — `encoding` field and provider mapping.
- `packages/atlas-review/src/context/sufficiency.rs` — only if provider matching is exhaustive.
- `packages/atlas-cli/src/commands/maintenance.rs` — runtime config JSON.
- `packages/atlas-cli/tests/cli_quality_gates/tokenizer_budget.rs` — tiktoken cases.
- `packages/atlas-mcp/src/tools/tests/context_ops/tokenizer_budget.rs` — tiktoken cases.
- `README.md` — provider docs and precedence.
- `SHIPPED.md` — completed entries.
- `deny.toml` — remove `paste` advisory ignore after `tokenizers` removal.
- `ISSUES.md` — checklist items and completion ticks.

---

## 10. Risk Register

| Risk | Impact | Mitigation |
|---|---|---|
| Regex patterns drift from upstream | Wrong token counts | Copy exact patterns, pin commit, golden parity gate |
| Vendored assets bloat repo | Clone size | Default only `cl100k` + `o200k`; optional features; LFS fallback |
| Pair-map memory blowup | High RSS | Benchmark eager vs lazy; cap entries; lazy per encoding |
| Special-token semantics mismatch | Wrong encode/decode | Golden cases for allowed/disallowed/longest match |
| Unicode split differences from `regex` vs `fancy-regex` | Parity failures | Use `fancy-regex`; copy patterns verbatim; parity corpus |
| Startup cost from eager decoding | Latency regression | Lazy init per encoding; measure p95 gate |
| Config breaking change on default flip | User surprise | One release opt-in; fallback stays heuristic; docs and changelog |
| Removal of `tokenizers` breaks HF users | Compatibility break | Phase 0 decision; deprecation release if retained |
| Asset corruption | Wrong counts or panic | Manifest sha256 test; optional runtime verify |
| Advisory regressions | CI deny failure | `cargo deny check` in gate; no new unknown sources |

---

## 11. Open Questions

1. Vendor all assets in normal git or use Git LFS?
2. Keep `provider = "tokenizers"` for one deprecation release, or remove immediately after default flip?
3. Should `atlas-tokenizer` be `publish = false` like other packages? Recommended yes.
4. Should `o200k_harmony` ship by default?
5. Does any consumer need token *bytes* (not just counts) in accounting output, e.g. for debugging truncation?
