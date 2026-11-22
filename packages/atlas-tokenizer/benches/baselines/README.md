# Tokenizer Bench Baselines

Recorded with:

```sh
TIKTOKEN_CACHE_DIR=/tmp/data-gym-cache \
  cargo bench -p atlas-tokenizer --bench encode -- --save-baseline atlas-baseline
```

Environment: Linux, Rust 1.100.0, `--release` bench profile. Criterion 0.5
`estimates.json` files are copied here for regression comparison. The corpus
is the fixed `TEXT` constant in `benches/encode.rs` (167 bytes).

| Encoding | Encode mean | Encode throughput | Decode mean | Decode throughput |
|---|---:|---:|---:|---:|
| `r50k_base` | 24.172 µs | 6.47 MiB/s | 1.748 µs | 32.0 Melem/s |
| `p50k_base` | 27.656 µs | 5.66 MiB/s | 2.137 µs | 26.2 Melem/s |
| `cl100k_base` | 26.044 µs | 6.01 MiB/s | 1.556 µs | 30.2 Melem/s |
| `o200k_base` | 19.642 µs | 7.96 MiB/s | 1.348 µs | 28.9 Melem/s |

Stability gate: encode throughput must not regress more than 20% against these
recorded values. A post-optimization rerun compared against this baseline
showed improvements across all four encodings (o200k encode `+7.7%`
throughput, p50k decode `+22.1%`).

Warm-cache lazy-init gate (release build, verified `o200k_base` cache):
`cargo test --release -p atlas-tokenizer --test startup -- --ignored --nocapture`
recorded p95 `135.6 ms`, below the `150 ms` limit.
