#!/usr/bin/env bash

set -euo pipefail

out_dir="${1:-target/bench-ci}"
target_dir="${CARGO_TARGET_DIR:-target}"
raw_log="${out_dir}/cargo-bench-output.log"
json_log="${out_dir}/cargo-bench-messages.jsonl"
criterion_dir="${out_dir}/criterion"

mkdir -p "$out_dir"
rm -f "$raw_log" "$json_log"
rm -rf "$criterion_dir"

export CARGO_TERM_COLOR=never

cargo bench --workspace --locked --message-format=json -- --noplot --save-baseline ci 2>&1 | tee "$raw_log"

grep -E '^\{.*\}$' "$raw_log" > "$json_log" || true

if [[ -d "${target_dir}/criterion" ]]; then
    mkdir -p "$criterion_dir"
    cp -R "${target_dir}/criterion/." "$criterion_dir/"
fi

rustc -vV > "${out_dir}/rustc-version.txt"
cargo --version --verbose > "${out_dir}/cargo-version.txt"
cargo metadata --format-version=1 --no-deps > "${out_dir}/cargo-metadata.json"
git rev-parse HEAD > "${out_dir}/git-head.txt" 2>/dev/null || true
find "$criterion_dir" -name estimates.json | LC_ALL=C sort > "${out_dir}/criterion-estimates.txt" || true
