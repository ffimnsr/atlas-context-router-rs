#!/usr/bin/env bash

set -euo pipefail

readonly DEFAULT_CONTAINERFILE="Containerfile"
readonly RUST_SOURCE_IMAGE="cgr.dev/chainguard/rust:latest-dev"
readonly GIT_SOURCE_IMAGE="cgr.dev/chainguard/git:latest-glibc"

usage() {
  cat <<'EOF'
Usage: scripts/update-containerfile-images.sh [options]

Resolve latest digests for pinned Containerfile base images and update ARG lines in place.

Options:
  --file <path>  Containerfile path to update. Default: Containerfile
  --dry-run      Print resolved image references without modifying files.
  -h, --help     Show this help message.

Resolution order:
  1. skopeo inspect
  2. podman pull + image inspect
  3. docker pull + image inspect
EOF
}

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

need_file() {
  [[ -f "$1" ]] || die "file not found: $1"
}

resolve_with_skopeo() {
  local image="$1"
  local digest

  digest="$(skopeo inspect --format '{{.Digest}}' "docker://${image}" 2>/dev/null)" || return 1
  [[ -n "$digest" ]] || return 1
  printf '%s@%s\n' "${image%%:*}" "$digest"
}

resolve_with_podman() {
  local image="$1"
  local ref

  podman pull "$image" >/dev/null
  ref="$(podman image inspect "$image" --format '{{index .RepoDigests 0}}' 2>/dev/null)" || return 1
  [[ -n "$ref" ]] || return 1
  printf '%s\n' "$ref"
}

resolve_with_docker() {
  local image="$1"
  local ref

  docker pull "$image" >/dev/null
  ref="$(docker image inspect "$image" --format '{{index .RepoDigests 0}}' 2>/dev/null)" || return 1
  [[ -n "$ref" ]] || return 1
  printf '%s\n' "$ref"
}

resolve_repo_digest() {
  local image="$1"

  if command -v skopeo >/dev/null 2>&1; then
    resolve_with_skopeo "$image" && return 0
  fi

  if command -v podman >/dev/null 2>&1; then
    resolve_with_podman "$image" && return 0
  fi

  if command -v docker >/dev/null 2>&1; then
    resolve_with_docker "$image" && return 0
  fi

  die "cannot resolve image digest for $image; need skopeo, podman, or docker"
}

rewrite_containerfile() {
  local file="$1"
  local rust_ref="$2"
  local git_ref="$3"
  local tmp

  tmp="$(mktemp)"

  awk -v rust_ref="$rust_ref" -v git_ref="$git_ref" '
    BEGIN { rust_seen = 0; git_seen = 0 }
    /^ARG RUST_IMAGE=/ {
      print "ARG RUST_IMAGE=" rust_ref
      rust_seen = 1
      next
    }
    /^ARG GIT_RUNTIME_IMAGE=/ {
      print "ARG GIT_RUNTIME_IMAGE=" git_ref
      git_seen = 1
      next
    }
    { print }
    END {
      if (!rust_seen || !git_seen) {
        exit 1
      }
    }
  ' "$file" >"$tmp" || {
    rm -f "$tmp"
    die "failed to update ARG lines in $file"
  }

  mv "$tmp" "$file"
}

main() {
  local containerfile="$DEFAULT_CONTAINERFILE"
  local dry_run=0

  while (($# > 0)); do
    case "$1" in
      --file)
        [[ $# -ge 2 ]] || die "--file requires a path"
        containerfile="$2"
        shift 2
        ;;
      --dry-run)
        dry_run=1
        shift
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      *)
        die "unknown option: $1"
        ;;
    esac
  done

  need_file "$containerfile"

  local rust_ref
  local git_ref
  rust_ref="$(resolve_repo_digest "$RUST_SOURCE_IMAGE")"
  git_ref="$(resolve_repo_digest "$GIT_SOURCE_IMAGE")"

  printf 'Resolved RUST_IMAGE=%s\n' "$rust_ref"
  printf 'Resolved GIT_RUNTIME_IMAGE=%s\n' "$git_ref"

  if (( dry_run )); then
    exit 0
  fi

  rewrite_containerfile "$containerfile" "$rust_ref" "$git_ref"
  printf 'Updated %s\n' "$containerfile"
}

main "$@"