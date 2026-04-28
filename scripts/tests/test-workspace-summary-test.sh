#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
parser_script="$repo_root/scripts/test-workspace-summary.sh"
tmp_dir="$(mktemp -d)"
passed_cases=0
trap 'rm -rf "$tmp_dir"' EXIT

run_case() {
    local name="$1"
    local log_file="$tmp_dir/$name.log"
    local expected_file="$tmp_dir/$name.expected"
    local actual_file="$tmp_dir/$name.actual"

    shift
    printf '%s\n' "$1" >"$log_file"
    printf '%s\n' "$2" >"$expected_file"

    bash "$parser_script" --parse-only "$log_file" >"$actual_file"
    diff -u "$expected_file" "$actual_file"
    passed_cases=$((passed_cases + 1))
    printf 'PASS %s\n' "$name"
}

run_command_case() {
    local name="$1"
    local expected="$2"
    local actual

    shift 2
    actual="$(bash "$parser_script" --print-command "$@")"
    if [[ "$actual" != "$expected" ]]; then
        printf 'FAIL %s\nexpected: %s\nactual:   %s\n' "$name" "$expected" "$actual"
        exit 1
    fi
    passed_cases=$((passed_cases + 1))
    printf 'PASS %s\n' "$name"
}

run_case \
    mixed-failures \
'     Running unittests src/lib.rs (target/debug/deps/atlas_core-1111111111111111)

running 2 tests
test tests::works ... ok
test tests::skip ... ignored

test result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/cli_quality_gates.rs (target/debug/deps/cli_quality_gates-2222222222222222)

running 3 tests
test cli::works ... ok
test cli::fails ... FAILED
test cli::also_fails ... FAILED

failures:
    cli::fails
    cli::also_fails

test result: FAILED. 1 passed; 2 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.01s

error: test failed, to rerun pass `-p atlas-cli --test cli_quality_gates`

   Doc-tests atlas_repo

running 1 test
test packages/atlas-repo/src/lib.rs - sample (line 4) ... FAILED

failures:
    packages/atlas-repo/src/lib.rs - sample (line 4)

test result: FAILED. 0 passed; 1 failed; 0 ignored; 1 measured; 0 filtered out; finished in 0.02s

error: test failed, to rerun pass `-p atlas-repo --doc`' \
'Merged test summary:
passed: 2
failed: 3
ignored: 1
measured: 1
filtered out: 1

Failed tests:
- cli::fails
- cli::also_fails
- packages/atlas-repo/src/lib.rs - sample (line 4)

Failed crates:
- atlas-cli
- atlas-repo'

run_case \
    all-pass \
'     Running unittests src/lib.rs (target/debug/deps/atlas_session-3333333333333333)

running 2 tests
test tests::alpha ... ok
test tests::beta ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s' \
'Merged test summary:
passed: 2
failed: 0
ignored: 0
measured: 0
filtered out: 0

Failed tests:
- none

Failed crates:
- none'

run_case \
    duplicate-failures-deduped \
'     Running unittests src/lib.rs (target/debug/deps/atlas_cli-4444444444444444)

running 2 tests
test flaky::case_one ... FAILED
test flaky::case_two ... FAILED

failures:
    flaky::case_one
    flaky::case_two
    flaky::case_one

test result: FAILED. 0 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s

error: test failed, to rerun pass `-p atlas-cli --lib`
error: test failed, to rerun pass `-p atlas-cli --lib`' \
'Merged test summary:
passed: 0
failed: 2
ignored: 0
measured: 0
filtered out: 0

Failed tests:
- flaky::case_one
- flaky::case_two

Failed crates:
- atlas-cli'

run_case \
    aggregate-many-suites \
'     Running unittests src/lib.rs (target/debug/deps/atlas_core-5555555555555555)

running 4 tests
test suite::alpha ... ok
test suite::beta ... ok
test suite::gamma ... ignored
test suite::delta ... ok

test result: ok. 3 passed; 0 failed; 1 ignored; 0 measured; 2 filtered out; finished in 0.01s

     Running tests/search.rs (target/debug/deps/search-6666666666666666)

running 3 tests
test search::smoke ... ok
test search::bench_snapshot ... FAILED
test search::tiny ... ok

failures:
    search::bench_snapshot

test result: FAILED. 2 passed; 1 failed; 0 ignored; 1 measured; 0 filtered out; finished in 0.05s

error: test failed, to rerun pass `-p atlas-search --test search`

   Doc-tests atlas_review

running 2 tests
test packages/atlas-review/src/lib.rs - render (line 9) ... ok
test packages/atlas-review/src/lib.rs - parse (line 21) ... ignored

test result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 3 filtered out; finished in 0.02s' \
'Merged test summary:
passed: 6
failed: 1
ignored: 2
measured: 1
filtered out: 5

Failed tests:
- search::bench_snapshot

Failed crates:
- atlas-search'

run_case \
    doc-test-only-failure \
'   Doc-tests atlas_parser

running 2 tests
test packages/atlas-parser/src/lib.rs - parse (line 12) ... FAILED
test packages/atlas-parser/src/lib.rs - scan (line 31) ... ok

failures:
    packages/atlas-parser/src/lib.rs - parse (line 12)

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 4 filtered out; finished in 0.04s

error: test failed, to rerun pass `-p atlas-parser --doc`' \
'Merged test summary:
passed: 1
failed: 1
ignored: 0
measured: 0
filtered out: 4

Failed tests:
- packages/atlas-parser/src/lib.rs - parse (line 12)

Failed crates:
- atlas-parser'

run_command_case \
    default-workspace-command \
    'cargo test --workspace'

run_command_case \
    scoped-package-command \
    'cargo test -p atlas-cli -p atlas-core' \
    -p atlas-cli -p atlas-core

printf 'All parser cases passed: %s\n' "$passed_cases"
