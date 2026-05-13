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

cargo bench --workspace --locked --no-run --message-format=json 2>&1 | tee "$raw_log"

grep -E '^\{.*"kind":\["bench"\].*\}$' "$raw_log" > "$json_log" || true

mapfile -t bench_bins < <(
    sed -n 's/.*"kind":\["bench"\].*"executable":"\([^"]*\)".*/\1/p' "$json_log" | sort -u
)

if [[ ${#bench_bins[@]} -eq 0 ]]; then
    echo "No benchmark executables found in cargo bench output" >&2
    exit 1
fi

for bench_bin in "${bench_bins[@]}"; do
    "$bench_bin" --noplot --save-baseline ci 2>&1 | tee -a "$raw_log"
done

if [[ -d "${target_dir}/criterion" ]]; then
    mkdir -p "$criterion_dir"
    cp -R "${target_dir}/criterion/." "$criterion_dir/"
fi

rustc -vV > "${out_dir}/rustc-version.txt"
cargo --version --verbose > "${out_dir}/cargo-version.txt"
cargo metadata --format-version=1 --no-deps > "${out_dir}/cargo-metadata.json"
git rev-parse HEAD > "${out_dir}/git-head.txt" 2>/dev/null || true
find "$criterion_dir" -name estimates.json | LC_ALL=C sort > "${out_dir}/criterion-estimates.txt" || true
