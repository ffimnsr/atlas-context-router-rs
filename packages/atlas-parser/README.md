# atlas-parser

Tree-sitter based source parsing and parser registry for Atlas. This crate has no SQLite access and is not part of database-sharing risk, making it safe for parallel execution within engine build/update phases.

## Public Surface

- **`ParserRegistry`** — multi-language parser dispatch
  - Supports 20+ languages (Rust, Go, Python, TypeScript, Java, C, C++, Bash, JSON, Markdown, etc.)
  - Thread-safe parser caching and reuse

- **`LangParser`, `ParseContext`** — parser interfaces
  - Parse source bytes to tree-sitter AST
  - Language detection and fallback handling

- **`TreeCache`** — AST caching for incremental parsing
  - Efficient tree reuse between parses
  - Memory-bounded cache management

- **`ast_helpers`** — syntax tree utilities
  - Node extraction and navigation helpers
  - Language-agnostic traversal patterns

Parser-only responsibility ensures it stays outside engine's SQLite/Rayon boundary.
