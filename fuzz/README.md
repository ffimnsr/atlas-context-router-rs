# Atlas fuzz targets

Targets:

- `parser_handlers`: fuzz `ParserRegistry` dispatch and incremental reparse flow
- `language_parsers`: fuzz direct built-in language parser handlers
- `regex_sql_udf`: fuzz SQLite `atlas_regexp()` path used by `atlas query --regex`

Examples:

```bash
cargo fuzz run parser_handlers
cargo fuzz run language_parsers
cargo fuzz run regex_sql_udf
```
