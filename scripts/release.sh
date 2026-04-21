#!/usr/bin/env bash

set -euo pipefail

readonly RELEASE_MANIFEST="packages/atlas-cli/Cargo.toml"
readonly REMOTE_NAME="origin"

# Topological publish order: dependencies before dependents.
readonly PUBLISH_ORDER=(
  atlas-core
  atlas-contentstore
  atlas-impact
  atlas-parser
  atlas-repo
  atlas-session
  atlas-store-sqlite
  atlas-contextsave
  atlas-reasoning
  atlas-refactor
  atlas-search
  atlas-review
  atlas-engine
  atlas-adapters
  atlas-mcp
  atlas-cli
)

usage() {
  cat <<'EOF'
Usage: scripts/release.sh [options] [<version>]

Bump Atlas workspace crate versions, refresh Cargo.lock, run release quality gates,
create release commit, create v-prefixed git tag, and optionally push commit/tag.

Release version source of truth: packages/atlas-cli/Cargo.toml
Updated manifests: packages/*/Cargo.toml

Options:
  --major      Increment major version and reset minor/patch to zero.
  --minor      Increment minor version and reset patch to zero.
  --patch      Increment patch version.
  --skip-push  Skip pushing release commit and tag to origin.
  --publish    Publish all workspace crates to crates.io in dependency order.
  -h, --help   Show this help message.

Examples:
  scripts/release.sh --patch
  scripts/release.sh --minor --skip-push
  scripts/release.sh 0.2.0
EOF
}

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || die "required command not found: $1"
}

manifest_paths() {
  git ls-files 'packages/*/Cargo.toml'
}

manifest_version() {
  local manifest="$1"

  awk '
    BEGIN { in_package = 0 }
    /^\[package\]$/ { in_package = 1; next }
    /^\[/ && $0 != "[package]" && in_package { in_package = 0 }
    in_package && /^version = "/ {
      gsub(/^version = "/, "", $0)
      gsub(/"$/, "", $0)
      print
      exit
    }
  ' "$manifest"
}

ensure_clean_worktree() {
  git diff --quiet --exit-code || die "working tree has unstaged changes"
  git diff --cached --quiet --exit-code || die "index has staged but uncommitted changes"
}

current_version() {
  manifest_version "$RELEASE_MANIFEST"
}

verify_workspace_versions() {
  local expected="$1"
  local mismatch=0
  local manifest
  local version

  while IFS= read -r manifest; do
    version="$(manifest_version "$manifest")"
    [[ -n "$version" ]] || die "failed to read package version from $manifest"

    if [[ "$version" != "$expected" ]]; then
      printf 'error: %s has version %s, expected %s\n' "$manifest" "$version" "$expected" >&2
      mismatch=1
    fi
  done < <(manifest_paths)

  (( mismatch == 0 )) || die "workspace package versions differ; align them before release"
}

increment_version() {
  local current="$1"
  local bump_kind="$2"
  local major minor patch

  IFS='.' read -r major minor patch <<<"$current"

  case "$bump_kind" in
    major)
      ((major += 1))
      minor=0
      patch=0
      ;;
    minor)
      ((minor += 1))
      patch=0
      ;;
    patch)
      ((patch += 1))
      ;;
    *)
      die "unsupported bump kind: $bump_kind"
      ;;
  esac

  printf '%s.%s.%s\n' "$major" "$minor" "$patch"
}

update_manifest_version() {
  local manifest="$1"
  local version="$2"
  local tmp

  tmp="$(mktemp)"

  awk -v version="$version" '
    BEGIN { in_package = 0; replaced = 0 }
    /^\[package\]$/ { in_package = 1 }
    /^\[/ && $0 != "[package]" && in_package { in_package = 0 }
    in_package && /^version = "/ && !replaced {
      print "version = \"" version "\""
      replaced = 1
      next
    }
    /path = "\.\.\/atlas-/ {
      if (/version =/) {
        sub(/version = "[^"]*"/, "version = \"" version "\"")
      } else {
        sub(/\}$/, ", version = \"" version "\"}")
      }
    }
    { print }
    END {
      if (!replaced) {
        exit 1
      }
    }
  ' "$manifest" >"$tmp" || {
    rm -f "$tmp"
    die "failed to update version in $manifest"
  }

  mv "$tmp" "$manifest"
}

update_workspace_versions() {
  local version="$1"
  local manifest

  while IFS= read -r manifest; do
    update_manifest_version "$manifest" "$version"
  done < <(manifest_paths)
}

ensure_remote_exists() {
  git remote get-url "$REMOTE_NAME" >/dev/null 2>&1 || die "git remote '$REMOTE_NAME' is not configured"
}

publish_workspace() {
  local pkg

  for pkg in "${PUBLISH_ORDER[@]}"; do
    printf 'Publishing %s...\n' "$pkg"
    cargo publish -p "$pkg"
  done
}

ensure_tag_absent() {
  local tag_name="$1"

  git rev-parse --verify "refs/tags/$tag_name" >/dev/null 2>&1 &&
    die "tag '$tag_name' already exists locally"

  if git remote get-url "$REMOTE_NAME" >/dev/null 2>&1; then
    git ls-remote --exit-code --tags "$REMOTE_NAME" "refs/tags/$tag_name" >/dev/null 2>&1 &&
      die "tag '$tag_name' already exists on '$REMOTE_NAME'" || true
  fi
}

main() {
  local run_push=1
  local run_publish=0
  local version=""
  local bump_kind=""

  while (($# > 0)); do
    case "$1" in
      --major|--minor|--patch)
        [[ -z "$bump_kind" ]] || die "only one of --major, --minor, or --patch may be used"
        bump_kind="${1#--}"
        shift
        ;;
      --skip-push)
        run_push=0
        shift
        ;;
      --publish)
        run_publish=1
        shift
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      -*)
        die "unknown option: $1"
        ;;
      *)
        [[ -z "$version" ]] || die "version may only be provided once"
        version="$1"
        shift
        ;;
    esac
  done

  if [[ -z "$version" && -z "$bump_kind" ]]; then
    usage
    exit 1
  fi

  [[ -z "$version" || -z "$bump_kind" ]] || die "pass either an explicit version or one bump flag"

  need_cmd awk
  need_cmd cargo
  need_cmd git
  need_cmd mktemp

  local repo_root
  repo_root="$(git rev-parse --show-toplevel 2>/dev/null)" || die "must be run inside a git repository"
  cd "$repo_root"

  ensure_clean_worktree

  local old_version
  old_version="$(current_version)"
  [[ -n "$old_version" ]] || die "failed to read current release version"
  [[ "$old_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "current version must match x.y.z"

  verify_workspace_versions "$old_version"

  if [[ -n "$bump_kind" ]]; then
    version="$(increment_version "$old_version" "$bump_kind")"
  fi

  [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "version must match x.y.z"
  [[ "$old_version" != "$version" ]] || die "version is already $version"

  local tag_name="v$version"

  if (( run_push )); then
    ensure_remote_exists
  fi

  ensure_tag_absent "$tag_name"

  update_workspace_versions "$version"

  cargo check --workspace --all-targets --quiet
  cargo fmt --all --check
  cargo clippy --workspace --all-targets --all-features -- -D warnings
  cargo test --workspace
  cargo test -p atlas-cli --test cli_quality_gates sqlite_fts5_smoke_round_trip -- --exact

  git add Cargo.lock packages/*/Cargo.toml
  git commit -m "release: $tag_name"

  git tag -a "$tag_name" -m "release: $tag_name"

  if (( run_push )); then
    git push "$REMOTE_NAME" HEAD
    git push "$REMOTE_NAME" "$tag_name"
  fi

  if (( run_publish )); then
    publish_workspace
  fi

  printf 'Released %s -> %s (%s)\n' "$old_version" "$version" "$tag_name"
}

main "$@"
