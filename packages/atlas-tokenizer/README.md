# atlas-tokenizer

Standalone, OpenAI `tiktoken`-compatible byte-pair tokenizer for Atlas context
accounting. The crate has no `atlas-*` dependencies and ports the upstream
tokenizer behavior at a pinned commit:

- Upstream: `https://github.com/openai/tiktoken`
- Pinned commit: `3591ff175d6a80efbe4fcc7f0e219ddd4b8c52f1`
- Upstream sources:
  `tiktoken_ext/openai_public.py` (encoding URLs, hashes, patterns, specials)
  and `tiktoken/load.py` (cache layout).

## Encodings

| Name | Feature | Default |
|---|---|---|
| `r50k_base` | `encoding-r50k` | yes |
| `p50k_base` | `encoding-p50k` | yes |
| `p50k_edit` | `encoding-p50k` | yes |
| `cl100k_base` | `encoding-cl100k` | yes |
| `o200k_base` | `encoding-o200k` | yes |
| `o200k_harmony` | `encoding-o200k-harmony` | no |

`gpt2` is out of scope because upstream builds it from `vocab.bpe` and
`encoder.json`, not a `.tiktoken` rank file.

## Vocab assets

Vocabulary assets are **not** committed to this repository. They are fetched
over HTTPS from the pinned upstream URLs, sha256-verified, and cached locally,
matching official `tiktoken` behavior. Cache directory precedence:

1. `TIKTOKEN_CACHE_DIR`
2. `DATA_GYM_CACHE_DIR`
3. `<std::env::temp_dir()>/data-gym-cache`

The cache key is `sha1(url)` in hex, so a Python `tiktoken` cache is reusable.
Cache entries are re-verified with sha256 on every load. A mismatched entry is
deleted and re-downloaded. Warm-cache operation is fully offline. Use
`Encoding::from_rank_file` (`file-assets` feature) for air-gapped setups, or
configure Atlas with `context.tokenizer.tokenizer_file`.

## Features

- `download` (default): HTTPS asset download and cache population.
- `file-assets` (default): load custom `.tiktoken` rank files.
- `batch` (off): optional rayon batch encoding helpers.
- `encoding-*` (see table): compile-time encoding availability.

With `download` disabled, named encodings return an error; only rank files and
byte inputs work. Build without default features to check this path:

```sh
cargo check -p atlas-tokenizer --no-default-features
```

## Licensing

Ported code keeps upstream MIT attribution in `LICENSE-TIKTOKEN`. This crate
is distributed under `MIT OR Apache-2.0`. Vocabulary files remain
OpenAI-hosted and are fetched unmodified.
