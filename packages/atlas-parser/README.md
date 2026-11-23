# atlas-parser

Tree-sitter based source parsing and parser registry for Atlas. Crate stays parser-only: no SQLite access, no engine-side graph persistence, safe for parallel execution inside build and update phases.

## Public Surface

- **`ParserRegistry`** — multi-language parser dispatch
  - Bundled: Rust, Go, Python, JavaScript, TypeScript, JSON, TOML, HTML, CSS, C, Markdown
  - Offloaded extractor profiles: PHP, C++, C#, Java, Scala, Ruby, Bash
  - Thread-safe parser caching and reuse
- **`LangParser`, `ParseContext`** — parser interfaces
  - Parse source bytes to tree-sitter AST
  - Language detection and fallback handling
- **`ExternalLangParser`, `ExternalParserConfig`, `ExternalExtractor`** — runtime external grammar support
  - Load a grammar checkout or prebuilt shared library at runtime
  - Dispatch to a built-in extractor profile, generic node-kind rules, an SCM query, or an out-of-process plugin
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

## Runtime external parsers

`ExternalLangParser::new` loads a grammar from `grammar_dir` or `lib_path` and
builds an extractor profile, a generic rule-based parser, or a query/capture
parser. `ExternalLangParser::from_language` accepts a preloaded
`tree_sitter::Language` for embedding and parity tests.

```rust,no_run
use atlas_parser::{ExternalExtractor, ExternalLangParser, ExternalParserConfig};

let config = ExternalParserConfig {
    extractor: Some(ExternalExtractor::Php),
    grammar_dir: Some("grammars/tree-sitter-php".to_owned()),
    ..Default::default()
};
let parser = ExternalLangParser::new(&config)
    .expect("external parser config should build");
```

Profile defaults fill the canonical language name, default extensions, and lib
function. Built-in extractor profiles call their module `validate_query` before
construction, so an ABI-compatible grammar with incompatible node kinds fails at
config load instead of mid-scan.

Query mode (`query_path` or inline `query`) compiles the SCM query against the
loaded grammar at construction. Invalid query text and unsupported `@atlas.*`
captures are configuration errors. Query mode reuses `qn_style`,
`owner_node_kinds`, test patterns, and the shared import/attribute/call emitters,
and rejects node-kind rule fields so exactly one extraction mode is active.

Plugin mode (`command` plus literal `args`) spawns one process per file and
speaks `atlas-parser.v1` JSON/stdio. The runner pins `working_dir`, enforces
`timeout_ms`, kills on expiry, caps stdout/stderr, and validates protocol, path,
hash, and node/edge file paths before returning a `ParsedFile`. Plugin failures
are surfaced through `LangParser::parse_checked`; `parse` keeps a best-effort
File-node fallback for callers that cannot handle per-file errors.

Offloaded grammar crates are dev-dependencies only. The default registry does
not link PHP, C++, C#, Java, Scala, Ruby, or Bash grammars; parser goldens
register the seven profiles with `ExternalLangParser::from_language` and keep the
existing golden JSON byte-identical.

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
