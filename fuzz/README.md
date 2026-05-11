# Atlas fuzz targets

Targets:

- `parser_handlers`: fuzz `ParserRegistry` dispatch and incremental reparse flow
- `language_parsers`: fuzz direct built-in language parser handlers
- `parser_invariants`: fuzz `ParserRegistry` and assert `ParsedFile` invariants
- `ast_helpers_walk`: fuzz `ast_helpers` across all built-in parser trees
- `refactor_parse_validation`: fuzz refactor parser revalidation on supported and unsupported paths
- `regex_sql_udf`: fuzz SQLite `atlas_regexp()` path used by `atlas query --regex`

Examples:

```bash
cargo fuzz run parser_handlers
cargo fuzz run language_parsers
cargo fuzz run parser_invariants
cargo fuzz run ast_helpers_walk
cargo fuzz run refactor_parse_validation
cargo fuzz run regex_sql_udf
```

Setup:

```bash
rustup toolchain install nightly
cargo install cargo-fuzz
cd fuzz
cargo +nightly fuzz build parser_handlers
```

`cargo install cargo-fuzz --locked` is intentionally avoided because older locked
transitive `rustix` versions fail on current nightly toolchains.

Committed seed corpus:

- parser-centric targets consume fixture-derived seed files under `fuzz/corpus/`
- `regex_sql_udf` ships a small hand-curated regex corpus and optional `regex.dict`
- arbitrary-byte fallback still works, so generated corpora can mix seed files and libFuzzer-found inputs

Refresh seed corpus from parser fixtures:

```bash
./fuzz/scripts/refresh_seed_corpus.sh
```

Run with committed seeds and optional regex dictionary:

```bash
cd fuzz
cargo +nightly fuzz run parser_handlers
cargo +nightly fuzz run language_parsers
cargo +nightly fuzz run tree_cache_stateful
cargo +nightly fuzz run parser_invariants
cargo +nightly fuzz run ast_helpers_walk
cargo +nightly fuzz run regex_sql_udf -- -dict=regex.dict
```
