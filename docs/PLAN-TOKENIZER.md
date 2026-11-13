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
- Vocab assets fetched from pinned upstream HTTPS URLs on first use, sha256-verified, cached locally. No assets committed to this repo.
- Warm cache works fully offline; `tokenizer_file` is the air-gap escape hatch.
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
  LICENSE-TIKTOKEN            # upstream MIT notice for ported code
  src/
    lib.rs                    # re-exports, crate docs, feature docs
    error.rs                  # typed errors (thiserror)
    rank_file.rs              # .tiktoken parser + validation
    regex_split.rs            # fancy-regex wrapper, Unicode-aware split
    bpe.rs                    # core byte-pair encode/decode primitives
    encoding.rs               # encoding specs: pattern, specials, source metadata
    model.rs                  # model -> encoding prefix map
    registry.rs               # EncodingLoader / cached encoding instances
    assets.rs                 # pinned upstream URLs, sha256, UPSTREAM_COMMIT
    download.rs               # HTTPS fetch + sha256 verify + size/time caps
    cache.rs                  # upstream-compatible cache key/dir + atomic writes
    batch.rs                  # optional rayon batch encode (feature = "batch")
  tests/
    parity.rs                 # golden vectors from official tiktoken
    roundtrip.rs              # decode(encode(x)) == x
    special_tokens.rs
    model_map.rs
    download_cache.rs         # cache hit/miss/hash mismatch/atomic write, no real network
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
- `sha2` — mandatory content verification on every load and cache hit.
- `sha1` — upstream-compatible cache key (`sha1(url)` hex).
- `reqwest` with `blocking` + `rustls` — HTTPS fetch, proxy env support, redirects. Reuses the workspace version; no direct `tokio` dependency.
- `rayon` — optional, behind `batch` feature, off by default.
- `criterion` — dev only, bench.

No `atlas-*` dependencies. Crate must build with
`cargo check -p atlas-tokenizer --no-default-features`.

Feature layout:

- `encoding-r50k`, `encoding-p50k`, `encoding-cl100k`, `encoding-o200k` (default on), `encoding-o200k-harmony` (off).
- `download` (default on) — fetch upstream assets over HTTPS, verify sha256, cache.
- `file-assets` (default on) — allow `Encoding::from_file` for custom `.tiktoken`.
- `batch` (off) — rayon batch API.

### 3.2 Public API Sketch

```rust
pub struct Encoding { /* immutable */ }

impl Encoding {
    pub fn get(name: EncodingName) -> Result<Arc<Encoding>>;
    pub fn for_model(model: &str) -> Result<Arc<Encoding>>;
    pub fn from_rank_file(path: &Path) -> Result<Encoding>;
    pub fn from_rank_bytes(bytes: &[u8], spec: EncodingSpec) -> Result<Encoding>;

    pub fn encode_ordinary(&self, text: &str) -> Vec<Rank>;
    pub fn encode(&self, text: &str, allowed_special: &[&str]) -> Result<Vec<Rank>>;
    pub fn encode_with_special_tokens(&self, text: &str) -> Result<Vec<Rank>>;
    pub fn decode(&self, tokens: &[Rank]) -> Result<String>;
    pub fn decode_bytes(&self, tokens: &[Rank]) -> Result<Vec<u8>>;
}

pub struct EncodingLoader { /* explicit cache config, testable */ }
impl EncodingLoader {
    pub fn new(config: CacheConfig) -> Self;
    pub fn load(&self, name: EncodingName) -> Result<Arc<Encoding>>;
    pub fn load_for_model(&self, model: &str) -> Result<Arc<Encoding>>;
}

pub fn default_loader() -> &'static EncodingLoader;
```

`CacheConfig` resolves the same env vars as upstream and can be overridden
explicitly by tests and by Atlas config.

Rules:

- `Encoding` immutable after construction, `Send + Sync`.
- `default_loader` uses `OnceLock` for immutable data only.
- Tests construct `EncodingLoader` explicitly with temp cache dirs; no global mutation.
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

Pin upstream commit in `src/assets.rs` (`UPSTREAM_COMMIT`) and in crate docs.
Reference the exact upstream GitHub file at that commit, not `main`.

Encodings and required data:

| Encoding | Pattern family | Specials |
|---|---|---|
| `r50k_base` | GPT-2 split | `<|endoftext|>` |
| `p50k_base` | GPT-2 split | `<|endoftext|>` |
| `p50k_edit` | GPT-2 split | `endoftext`, `fim_prefix`, `fim_middle`, `fim_suffix` |
| `cl100k_base` | cl100k split | `endoftext`, `fim_*`, `endofprompt` |
| `o200k_base` | o200k split | `endoftext`, `endofprompt` |
| `o200k_harmony` | o200k split | harmony reserved specials (feature-gated) |

`gpt2` is excluded in v1: upstream builds it from `vocab.bpe` + `encoder.json`
(data-gym format), not from a `.tiktoken` rank file. Add it later only if a
consumer needs the original GPT-2 encoding.

Implementation rules:

- Copy exact regex pattern strings from upstream into constants with source URL and commit.
- Do not “simplify” patterns. Parity tests will catch drift.
- Model map is ordered longest-prefix-first, mirroring upstream `MODEL_PREFIX_TO_ENCODING`.
- Unknown model returns `Error::UnknownModel`; explicit encoding wins over model inference.
- `model` metadata is preserved for accounting even when encoding is explicit.

---

## 5. Assets and Supply Chain

Decision: do **not** commit `.tiktoken` vocab files. Download them from the
pinned upstream URLs on first use, verify sha256, cache locally. Same model as
official `tiktoken`.

### 5.1 Pinned upstream references

No assets are copied into this repo. The source of truth is the exact upstream
file, referenced by commit:

- Encoding URLs and hashes:
  `https://github.com/openai/tiktoken/blob/3591ff175d6a80efbe4fcc7f0e219ddd4b8c52f1/tiktoken_ext/openai_public.py`
- Download and cache behavior:
  `https://github.com/openai/tiktoken/blob/3591ff175d6a80efbe4fcc7f0e219ddd4b8c52f1/tiktoken/load.py`
- Raw forms for maintainers:
  `https://raw.githubusercontent.com/openai/tiktoken/3591ff175d6a80efbe4fcc7f0e219ddd4b8c52f1/tiktoken_ext/openai_public.py`
  `https://raw.githubusercontent.com/openai/tiktoken/3591ff175d6a80efbe4fcc7f0e219ddd4b8c52f1/tiktoken/load.py`

`src/assets.rs` stores:

- `UPSTREAM_COMMIT = "3591ff175d6a80efbe4fcc7f0e219ddd4b8c52f1"`
- per-encoding `url`, `sha256`, and the upstream file reference in doc comments

Update procedure when upstream changes:

1. Diff the two upstream files above at the new commit.
2. Copy changed URL, sha256, regex pattern, and special-token values into
   `src/assets.rs` and `src/encoding.rs`.
3. Bump `UPSTREAM_COMMIT`.
4. Regenerate golden parity vectors.
5. Run parity, roundtrip, and fuzz gates.

### 5.2 Download and cache behavior

Mirror upstream `read_file_cached`:

- Cache dir precedence: `TIKTOKEN_CACHE_DIR`, then `DATA_GYM_CACHE_DIR`, then
  `std::env::temp_dir()/data-gym-cache`.
- Cache key: `sha1(url)` hex, same as upstream so a Python `tiktoken` cache can
  be reused.
- Cache hit + sha256 match: use it, no network.
- Cache hit + sha256 mismatch: delete entry and re-download.
- Download: HTTPS only, connect timeout 10s, total timeout 60s, redirect final
  URL must stay HTTPS, streamed read capped at 32 MiB.
- Verify sha256 before parsing. Never parse unverified bytes.
- Write temp file then atomic rename. Concurrent processes may duplicate a
  download; result stays correct.
- Default cache dir unwritable: ignore write failure like upstream and continue
  with in-memory data. Explicit user cache dir unwritable: return an error.
- No auth, no custom headers, no secrets in URLs.

`download` feature off: only `tokenizer_file` and `from_rank_bytes` work;
encoding names return `Error::DownloadDisabled`.

### 5.3 Asset sources

| Encoding | Upstream file | sha256 |
|---|---|---|
| `r50k_base` | `https://openaipublic.blob.core.windows.net/encodings/r50k_base.tiktoken` | `306cd27f03c1a714eca7108e03d66b7dc042abe8c258b44c199a7ed9838dd930` |
| `p50k_base` | `https://openaipublic.blob.core.windows.net/encodings/p50k_base.tiktoken` | `94b5ca7dff4d00767bc256fdd1b27e5b17361d7b8a5f968547f9f23eb70d2069` |
| `p50k_edit` | same file as `p50k_base` | same as `p50k_base` |
| `cl100k_base` | `https://openaipublic.blob.core.windows.net/encodings/cl100k_base.tiktoken` | `223921b76ee99bde995b7ff738513eef100fb51d18c93597a113bcffe865b2a7` |
| `o200k_base` | `https://openaipublic.blob.core.windows.net/encodings/o200k_base.tiktoken` | `446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d` |
| `o200k_harmony` | same file as `o200k_base` (specials defined in code) | same as `o200k_base` |

`gpt2` uses `vocab.bpe` + `encoder.json` and is out of scope for v1.

### 5.4 Failure and offline behavior

- Download or hash failure propagates as an encoding load error.
- Existing `[context.tokenizer]` fallback applies: `fallback = "heuristic"` uses
  the byte heuristic and records `fallback_reason`; `fail_closed` errors before
  payload truncation.
- Warm cache: fully offline.
- Air-gapped: set `tokenizer_file` to a local `.tiktoken` file; no download path
  is used.
- First run with default `tiktoken` provider requires network. Document this in
  README and the security model.

### 5.5 Licensing

- No OpenAI data committed to this repo.
- Ported code keeps upstream MIT attribution in `LICENSE-TIKTOKEN`; crate remains
  `MIT OR Apache-2.0`.
- Vocab files remain OpenAI-hosted and are fetched unmodified.

### 5.6 Supply-chain rules

- sha256 constants are copied from the pinned upstream file, not generated by Atlas.
- HTTPS + sha256 is the integrity boundary; upstream publishes no signatures.
- Cache entries are re-verified on every load.
- Reject non-HTTPS redirect targets.
- Tests never hit the real network by default; one ignored smoke test may fetch
  upstream when explicitly run.

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

### 6.3 Stability Switch and Legacy Removal (Phase 7 only)

Do not pick the default encoding or removal dates in Phase 0. Decide both
once `atlas-tokenizer` is stable.

After stability gates pass:

- provider default `heuristic` → `tiktoken`
- default encoding: chosen at Phase 7 from parity and bench results
- remove `provider = "heuristic"` as a selectable provider
- keep byte-based counting only as automatic fallback (no config option)
- remove `provider = "tokenizers"` and the `tokenizers` dependency
- remove `bytes_per_token` from the public config unless still needed by fallback

First run with the default provider may download the vocab unless cache is warm.
Document this and keep `tokenizer_file` documented for air-gapped use.

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
- `download_cache.rs`: local fixture HTTP server + temp cache; cover hit, miss, sha256 mismatch, oversized response, non-HTTPS redirect rejection, atomic rename, unwritable cache dir. No real network.
- `rank_file.rs` unit tests: malformed base64, duplicate ranks, oversized lines, comments, CRLF.
- `bpe.rs` unit tests: empty, single byte, no-merge, deep merge, max token length.
- Integration: CLI + MCP tokenizer tests extended with `provider = "tiktoken"` and exact expected counts; pre-populate cache or point `tokenizer_file` at a fixture so tests stay offline.
- Fuzz target under workspace `fuzz/`: arbitrary `&str` and malformed rank files; assert no panic and roundtrip.
- Bench: criterion encode/decode throughput per encoding; store baseline under `benches/baselines/`.
- Optional ignored smoke test: fetch real upstream URLs and verify sha256 when explicitly run.

### 7.3 Stability Gates Before Default Flip

All must hold:

1. 100% golden parity for `cl100k_base` and `o200k_base`; zero known mismatches.
2. Roundtrip and fuzz suites green; no panic found.
3. Network isolation: default tests pass with networking disabled; warm-cache run needs no network; download path covered by local fixture server.
4. Startup: warm-cache lazy encoding init p95 ≤ 150 ms on CI baseline hardware; record the actual baseline and tighten later if safe.
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

- [x] Do not commit vocab assets; download from pinned upstream URLs like official `tiktoken`.
- [x] Reference exact upstream `tiktoken_ext/openai_public.py` and `tiktoken/load.py` by commit.
- [ ] Defer default encoding choice to Phase 7 after parity and bench results.
- [ ] Defer `provider = "heuristic"` and `provider = "tokenizers"` removal to Phase 7, once stable.
- [ ] Confirm `o200k_harmony` inclusion.
- [x] Pin upstream tiktoken commit `3591ff175d6a80efbe4fcc7f0e219ddd4b8c52f1` in the plan.

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

### Phase 4 — Download and Cache

- [ ] Add `src/assets.rs` with `UPSTREAM_COMMIT`, per-encoding URL + sha256, and exact upstream GitHub file references.
- [ ] Implement `src/download.rs`: HTTPS only, timeouts, redirect scheme check, 32 MiB cap, sha256 verify.
- [ ] Implement `src/cache.rs`: upstream-compatible `sha1(url)` key, env dir precedence, atomic temp+rename, corrupt-entry recovery.
- [ ] Add `download_cache.rs` tests with a local fixture HTTP server; no real network.
- [ ] Implement `tokenizer_file` local path and `download` feature off behavior.
- [ ] Verify cold download once in an ignored smoke test, then warm-cache run works offline.

Exit: encoding load fetches, verifies, caches; default tests stay offline.

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

### Phase 7 — Stability Switch and Legacy Removal

Run only after all stability gates in §7.3 pass.

- [ ] Choose the default encoding from parity and bench results.
- [ ] Change `ContextTokenizerConfig::default()` to `tiktoken` + chosen encoding.
- [ ] Remove `provider = "heuristic"` as a selectable provider.
- [ ] Keep byte-based counting only as automatic fallback; verify fallback metadata.
- [ ] Remove `provider = "tokenizers"` and the `tokenizers` dependency.
- [ ] Remove the `paste` advisory ignore in `deny.toml`.
- [ ] Update template, README, security docs, and docs drift guard.
- [ ] Update `SHIPPED.md` and changelog.
- [ ] Run full workspace tests, clippy, fmt, deny, and bench regression check.

Exit: legacy heuristic and tokenizers providers removed; default is OpenAI-compatible, warm cache offline, cold start documents first-use download.

### Phase 8 — Optional Follow-Ups

- [ ] BPE training helper behind `train` feature, mirroring `tiktoken/train.py`.
- [ ] `o200k_harmony` default-feature decision based on model usage.
- [ ] Batch encode tuning and memory profiling.
- [ ] Optional encoding registry injection in long-lived services if startup cost matters.

---

## 9. File Change Map

New:

- `packages/atlas-tokenizer/**` (see §3)
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
| Upstream host unavailable on first use | Count falls back or errors | Existing `fallback` semantics; `tokenizer_file` air-gap path; warm cache avoids network |
| First-use download latency | Slow startup once | Async not needed; 60s cap; cache persists across runs |
| Cache poisoning or corrupt download | Wrong counts | Mandatory sha256 before parse; delete mismatched cache entry |
| Redirect to non-HTTPS host | MITM risk | Reject non-HTTPS final URL; no custom headers or secrets |
| Upstream URL/hash change | Stale pins | Pinned `UPSTREAM_COMMIT`; documented diff-and-swap procedure |
| Pair-map memory blowup | High RSS | Benchmark eager vs lazy; cap entries; lazy per encoding |
| Special-token semantics mismatch | Wrong encode/decode | Golden cases for allowed/disallowed/longest match |
| Unicode split differences from `regex` vs `fancy-regex` | Parity failures | Use `fancy-regex`; copy patterns verbatim; parity corpus |
| Startup cost from eager decoding | Latency regression | Lazy init per encoding; measure p95 gate |
| Config breaking change on default flip | User surprise | One release opt-in; fallback stays heuristic; docs and changelog |
| Removal of `tokenizers` breaks HF users | Compatibility break | Phase 0 decision; deprecation release if retained |

| Advisory regressions | CI deny failure | `cargo deny check` in gate; no new unknown sources |

---

## 11. Open Questions

1. Should `atlas-tokenizer` be `publish = false` like other packages? Recommended yes.
2. Should `o200k_harmony` ship by default?
3. Does any consumer need token *bytes* (not just counts) in accounting output, e.g. for debugging truncation?
4. Should Atlas honor `TIKTOKEN_CACHE_DIR` / `DATA_GYM_CACHE_DIR` exactly, or add `ATLAS_TOKENIZER_CACHE_DIR` first? Recommended: honor upstream vars, add Atlas var only if needed.
5. Phase 7 only: which default encoding wins after parity and bench results?
6. Phase 7 only: keep `bytes_per_token` for fallback tuning, or remove it with the heuristic provider?
