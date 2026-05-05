#!/usr/bin/env bash
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(cd -- "$script_dir/../.." && pwd)
fixture_root="$repo_root/packages/atlas-parser/tests/fixtures"
corpus_root="$repo_root/fuzz/corpus"
dict_path="$repo_root/fuzz/regex.dict"

targets=(
  parser_handlers
  language_parsers
  tree_cache_stateful
  parser_invariants
  ast_helpers_walk
)

for target in "${targets[@]}"; do
  mkdir -p "$corpus_root/$target"
  find "$corpus_root/$target" -maxdepth 1 -type f -name 'seed_*' -delete
done

mkdir -p "$corpus_root/regex_sql_udf"
find "$corpus_root/regex_sql_udf" -maxdepth 1 -type f -name 'seed_*' -delete

emit_parser_seed() {
  local target=$1
  local kind=$2
  local label=$3
  local source_path=$4
  local output_path="$corpus_root/$target/seed_${kind}_${label}.txt"

  {
    printf 'ATLAS_PARSER_SEED\n'
    printf 'kind=%s\n' "$kind"
    printf 'reuse_old_tree=true\n'
    printf '===SOURCE===\n'
    cat "$source_path"
    printf '\n===NEXT===\n'
    cat "$source_path"
  } > "$output_path"
}

emit_source_seed() {
  local target=$1
  local kind=$2
  local label=$3
  local source_path=$4
  local output_path="$corpus_root/$target/seed_${kind}_${label}.txt"

  {
    printf 'ATLAS_SOURCE_SEED\n'
    printf 'kind=%s\n' "$kind"
    printf '===SOURCE===\n'
    cat "$source_path"
  } > "$output_path"
}

emit_tree_cache_seed() {
  local kind=$1
  local label=$2
  local source_path=$3
  local output_path="$corpus_root/tree_cache_stateful/seed_${kind}_${label}.txt"

  {
    printf 'ATLAS_TREE_CACHE_SEED\n'
    printf 'kind=%s\n' "$kind"
    printf '===SOURCE===\n'
    cat "$source_path"
  } > "$output_path"
}

while IFS= read -r fixture; do
  rel_path=${fixture#"$fixture_root/"}
  kind=${rel_path%%/*}
  file_name=${rel_path##*/}
  label=${file_name%.*}

  emit_parser_seed parser_handlers "$kind" "$label" "$fixture"
  emit_parser_seed language_parsers "$kind" "$label" "$fixture"
  emit_tree_cache_seed "$kind" "$label" "$fixture"
  emit_source_seed parser_invariants "$kind" "$label" "$fixture"
  emit_source_seed ast_helpers_walk "$kind" "$label" "$fixture"
done < <(
  find "$fixture_root" -mindepth 2 -maxdepth 2 -type f \
    \( -name 'core.*' -o -name 'bad_syntax.*' \) \
    ! -name '*.golden.*' \
    | sort
)

emit_regex_seed() {
  local name=$1
  local pattern=$2
  local value=$3
  local output_path="$corpus_root/regex_sql_udf/seed_${name}.txt"

  {
    printf 'ATLAS_REGEX_SEED\n'
    printf '===PATTERN===\n'
    printf '%s' "$pattern"
    printf '\n===VALUE===\n'
    printf '%s' "$value"
  } > "$output_path"
}

emit_regex_seed literals 'atlas' 'atlas context router'
emit_regex_seed anchors '^atlas$' 'atlas'
emit_regex_seed alternation 'rust|go|python' 'go'
emit_regex_seed character_classes '[A-Z][a-z]+_[0-9]+' 'Rust_2024'
emit_regex_seed invalid_patterns '(' 'atlas'
emit_regex_seed unicode_heavy '^\p{L}+$' 'naivé'

cat > "$dict_path" <<'EOF'
.
*
+
?
^
$
|
()
[]
{}
\\d
\\w
\\s
\\p{L}
(?i)
(?m)
(?s)
EOF
