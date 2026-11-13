#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# Sourcing is safe: release.sh only runs main when executed directly.
# shellcheck source=../release.sh
source "$repo_root/scripts/release.sh"

fail() {
  printf 'FAIL %s\n' "$*" >&2
  exit 1
}

cd "$repo_root"

manifests="$(manifest_paths)"

grep -qx 'packages/atlas-cli/Cargo.toml' <<<"$manifests" ||
  fail "workspace manifest missing from manifest_paths"

if grep -q 'tests/testdata' <<<"$manifests"; then
  printf '%s\n' "$manifests" >&2
  fail "manifest_paths includes nested testdata fixture manifests"
fi

expected_count="$(git ls-files ':(glob)packages/*/Cargo.toml' | wc -l)"
actual_count="$(wc -l <<<"$manifests" | tr -d ' ')"
[[ "$actual_count" -eq "$expected_count" ]] ||
  fail "manifest count $actual_count != $expected_count"

version="$(manifest_version "$RELEASE_MANIFEST")"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] ||
  fail "manifest_version returned '$version'"

# Regression: the fixture crate keeps its own version and must not break the
# release version-alignment gate.
fixture="packages/atlas-engine/tests/testdata/reachability_repo/Cargo.toml"
if [[ -f "$fixture" ]]; then
  fixture_version="$(manifest_version "$fixture")"
  [[ -n "$fixture_version" ]] || fail "failed to read fixture version from $fixture"
  [[ "$fixture_version" != "$version" ]] ||
    fail "fixture version should stay independent of the release version"
fi

verify_workspace_versions "$version" ||
  fail "verify_workspace_versions rejected aligned workspace manifests"

printf 'PASS release manifest paths exclude nested fixtures\n'
