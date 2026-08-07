#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
pre_commit_hook="$repo_root/.githooks/pre-commit"
pre_push_hook="$repo_root/.githooks/pre-push"

bash -n "$pre_commit_hook"
bash -n "$pre_push_hook"

grep -Fq 'cargo fmt --all --check' "$pre_commit_hook"
grep -Fq 'cargo clippy "${scope_args[@]}" --all-targets --all-features -- -D warnings' "$pre_commit_hook"
grep -Fq 'cargo +stable clippy "${scope_args[@]}" --all-targets --all-features -- -D warnings' "$pre_commit_hook"
if grep -Fq 'test-workspace-summary.sh' "$pre_commit_hook"; then
    printf 'FAIL pre-commit must not run workspace tests\n' >&2
    exit 1
fi

grep -Fq './scripts/test-workspace-summary.sh --workspace' "$pre_push_hook"

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT
cat >"$tmp_dir/cargo" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$*" >"$ATLAS_HOOK_CARGO_ARGS"
printf 'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s\n'
EOF
chmod +x "$tmp_dir/cargo"
ATLAS_HOOK_CARGO_ARGS="$tmp_dir/cargo.args" PATH="$tmp_dir:$PATH" bash "$pre_push_hook" >"$tmp_dir/pre-push.log"
grep -Fxq 'test --workspace' "$tmp_dir/cargo.args"
grep -Fq '[pre-push] test passed' "$tmp_dir/pre-push.log"

grep -Fq 'git commit -m "release: $tag_name"' "$repo_root/scripts/release.sh"
if grep -Fq 'cargo test' "$repo_root/scripts/release.sh"; then
    printf 'FAIL release script must defer tests to pre-push\n' >&2
    exit 1
fi

printf 'PASS git hook test placement\n'
