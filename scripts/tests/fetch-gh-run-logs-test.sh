#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
script_path="$repo_root/scripts/fetch-gh-run-logs.sh"
tmp_dir="$(mktemp -d)"
passed_cases=0
trap 'rm -rf "$tmp_dir"' EXIT

mock_bin_dir="$tmp_dir/bin"
mkdir -p "$mock_bin_dir"

cat >"$mock_bin_dir/gh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

if [[ "$1" == "repo" && "$2" == "view" ]]; then
  printf 'octo/example\n'
  exit 0
fi

if [[ "${1-}" != "api" ]]; then
  printf 'unexpected gh invocation: %s\n' "$*" >&2
  exit 1
fi
shift

while [[ $# -gt 0 && "$1" == -* ]]; do
  case "$1" in
    -H)
      shift 2
      ;;
    --jq|--json)
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done

case "$1" in
  repos/octo/example/actions/runs/42)
    cat <<'JSON'
{"status":"completed","conclusion":"failure","display_title":"ci failure"}
JSON
    ;;
  repos/octo/example/actions/runs/42/jobs?per_page=100)
    cat <<'JSON'
{"jobs":[
  {"id":101,"status":"completed","conclusion":"success","name":"Lint and Format"},
  {"id":202,"status":"completed","conclusion":"failure","name":"Test (ubuntu-latest)"},
  {"id":303,"status":"completed","conclusion":"failure","name":"Test (macos-latest)"}
]}
JSON
    ;;
  repos/octo/example/actions/jobs/101/logs)
    printf 'lint log\n'
    ;;
  repos/octo/example/actions/jobs/202/logs)
    printf 'ubuntu failed\n'
    ;;
  repos/octo/example/actions/jobs/303/logs)
    printf 'mac failed\n'
    ;;
  *)
    printf 'unexpected gh api path: %s\n' "$1" >&2
    exit 1
    ;;
esac
EOF
chmod +x "$mock_bin_dir/gh"

run_case() {
  local name="$1"
  local expected_files_csv="$2"
  shift 2

  local out_dir="$tmp_dir/$name-out"
  local output_file="$tmp_dir/$name.stdout"

  PATH="$mock_bin_dir:$PATH" bash "$script_path" --repo octo/example --out-dir "$out_dir" "$@" 42 >"$output_file"

  IFS=',' read -r -a expected_files <<<"$expected_files_csv"
  for expected in "${expected_files[@]}"; do
    [[ -f "$out_dir/$expected" ]] || {
      printf 'FAIL %s missing file %s\n' "$name" "$expected"
      exit 1
    }
  done

  passed_cases=$((passed_cases + 1))
  printf 'PASS %s\n' "$name"
}

run_case \
  failed_only \
  '202-test-ubuntu-latest.log,303-test-macos-latest.log' \
  --failed-only

if [[ -e "$tmp_dir/failed_only-out/101-lint-and-format.log" ]]; then
  printf 'FAIL failed_only downloaded success job\n'
  exit 1
fi

run_case \
  specific_job \
  '202-test-ubuntu-latest.log' \
  --job 202

if [[ -e "$tmp_dir/specific_job-out/303-test-macos-latest.log" ]]; then
  printf 'FAIL specific_job downloaded extra job\n'
  exit 1
fi

printf 'PASS all (%s cases)\n' "$passed_cases"