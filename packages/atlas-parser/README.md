# atlas-parser

Tree-sitter based source parsing and parser registry for Atlas. Crate stays parser-only: no SQLite access, no engine-side graph persistence, safe for parallel execution inside build and update phases.

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
- **`query_helpers`** — shared tree-sitter query harness
  - Static per-language query compilation via `include_str!`
  - Deterministic capture ordering and source-order match traversal
  - Exact capture lookup with optional/required helpers

Parser-only responsibility keeps crate outside engine SQLite/Rayon boundary.

## Shared Query-Backed Parser Contract

Patch SQ1 defines shared query-backed extraction contract for remaining tree-sitter language parsers.

### Query file location

- Atlas query files live under `packages/atlas-parser/queries/<language>.scm`.
- Query text is loaded from Rust via one static string per language, typically `const QUERY: &str = include_str!("../../../queries/<language>.scm");`.
- Helix `runtime/queries/*/tags.scm` and `runtime/queries/*/locals.scm` may be used as grammar references only. Atlas `.scm` files must be authored for Atlas captures and must not be copied verbatim without license handling.

### Capture namespace and meaning

- Capture names use `@atlas.*` namespace.
- Queries capture syntax facts only: matching declarations, names, receiver nodes, parameter nodes, import syntax, call syntax, and reference syntax.
- Language parser Rust code still owns Atlas graph semantics:
  - `Node` kind selection
  - `Edge` kind selection
  - qualified-name construction
  - parent-scope resolution
  - confidence tiers
  - source metadata
  - language-specific heuristics and fallback behavior
- Query captures do **not** replace semantic resolution by themselves.

### Common capture conventions

Shared conventions for migration work:

- `@atlas.definition.function`
- `@atlas.definition.method`
- `@atlas.definition.class`
- `@atlas.definition.module`
- `@atlas.definition.struct`
- `@atlas.definition.enum`
- `@atlas.definition.interface`
- `@atlas.definition.trait`
- `@atlas.definition.constant`
- `@atlas.definition.variable`
- `@atlas.import`
- `@atlas.call`
- `@atlas.reference`
- `@atlas.name`
- `@atlas.parameters`
- `@atlas.return_type`
- `@atlas.receiver`

Language-specific helper captures may exist when needed for syntax extraction, but shared migrations should prefer common names first.

### Migration boundary

- Parser public APIs remain unchanged.
- Database schemas remain unchanged.
- Graph output contracts remain unchanged.
- Each language parser maps shared query captures into `Node`, `Edge`, and `ParsedFile` without changing external parser behavior.

### Documented manual exceptions after SQ6

Some parsers stay manual because query migration would not reduce syntax work without risking current semantics:

- `src/lang/json.rs` — parent-driven object/key path and array index construction still requires recursive ownership over traversal order, including best-effort malformed trees.
- `src/lang/toml.rs` — parser intentionally uses `toml::Value` plus custom line indexing for stable table/key-path semantics instead of tree-sitter syntax matching.
- `src/lang/markdown.rs` — heading hierarchy, duplicate slug handling, fenced-code ownership, regex link extraction, and current malformed shorter-input safeguards remain tied to manual ordered walking.
