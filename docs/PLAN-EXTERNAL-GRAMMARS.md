# Atlas External Grammar Offload Plan

Status: proposed

Implementation tracking: [`ISSUES.md` § External Grammar Offload Patch](../ISSUES.md#external-grammar-offload-patch).
This document is the design/reference; tick the issue checklist as work lands.

Scope: move `tree-sitter-php`, `tree-sitter-cpp`, `tree-sitter-c-sharp`,
`tree-sitter-java`, `tree-sitter-scala`, `tree-sitter-ruby`, and
`tree-sitter-bash` from bundled compile-time dependencies to runtime-loaded
external grammars, while keeping the existing hand-written extractors and
byte-identical graph output.

Goal: smaller release binary, shorter C compilation time, no loss of graph
fidelity, a config path that users can actually operate, and explicit closure of
the known external-parser fidelity gaps through profiles, rules, queries, or an
out-of-process plugin.

Non-goals:

- full built-in parity for arbitrary external languages; query/plugin paths stay
  best-effort unless a shipped profile exists
- runtime Rust `dylib` parser plugin or stable Rust ABI work
- OS-level sandboxing for external grammars/plugins; configured code is trusted
- automatic grammar download at build time
- changes to graph schema, QName conventions, or existing parser output
- offloading the remaining bundled grammars in this patch

---

## 1. Current State

### 1.1 Bundled parsers in scope

| Language | Module | Grammar crate | LOC | Extraction currently emitted |
|---|---|---:|---:|---|
| PHP | `packages/atlas-parser/src/lang/php.rs` | `tree-sitter-php` | 713 | namespace, `use` imports, class/interface/trait, functions/methods, params, attributes, calls |
| C++ | `packages/atlas-parser/src/lang/cpp.rs` | `tree-sitter-cpp` | 569 | includes, namespace/types, functions/methods, template modifier, params/return, owner-aware calls |
| C# | `packages/atlas-parser/src/lang/csharp.rs` | `tree-sitter-c-sharp` | 649 | usings, namespace/types, methods, params/return, attributes, calls |
| Java | `packages/atlas-parser/src/lang/java.rs` | `tree-sitter-java` | 620 | package, imports, class/interface/enum/annotation type, methods, params/return, annotations, calls |
| Scala | `packages/atlas-parser/src/lang/scala.rs` | `tree-sitter-scala` | 679 | package, imports, object/class/trait, function/method params/return/modifiers, case-class QN, calls |
| Ruby | `packages/atlas-parser/src/lang/ruby.rs` | `tree-sitter-ruby` | 598 | `require`/`require_relative`/mixins, module/class/method/singleton method, calls |
| Bash | `packages/atlas-parser/src/lang/bash.rs` | `tree-sitter-bash` | 524 | functions, `source` imports, command/call edges |

Each parser currently constructs its grammar directly, for example:

```rust
parser.set_language(&tree_sitter_php::LANGUAGE_PHP.into())
```

`LanguagePolicy` in `packages/atlas-engine/src/lang_policy.rs` is unchanged by
this plan. Java and C++ remain `Experimental`; offloading does not change
rename/dead-code/import-cleanup feature gates.

and each `*SyntaxFacts::extract` compiles its query against the same hard-coded
grammar.

### 1.2 Existing external parser path
params, return types, modifiers, test detection, owner-aware QNames, or accurate

`packages/atlas-parser/src/external.rs` (542 LOC) already provides:

- `tree-sitter-loader` based loading from `grammar_dir` (compiled C grammar) or
  `lib_path` (prebuilt shared library)
- ABI validation against `tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=LANGUAGE_VERSION`
- generic node-kind extraction via `ExternalSymbolRule`
- best-effort call edges via `call_node_kinds` / `call_target_field`

It does not provide built-in extraction semantics: no imports, attributes,
params, return types, modifiers, test detection, owner-aware QNames, or accurate
call confidence.

### 1.3 Current integration points

- Registry: `ParserRegistry::with_defaults` registers all bundled parsers.
- Config: `ParsersConfig::external` in
  `packages/atlas-engine/src/config/parsers.rs`; path resolution and validation
  happen in `Config::load`.
- Build/update: `BuildOptions.external_parsers` and
  `UpdateOptions.external_parsers`; `register_externals` runs before the scan.
- Agent events: `packages/atlas-agent-events/src/actions.rs` and
  `graph_readiness.rs` already honor `[parsers.external]`.
- Watch: `WatchRunner::apply_batch` hard-codes `external_parsers: Vec::new()`.
- Status/doctor: `parser_registry_from_config` registers externals.
- Golden tests: `packages/atlas-parser/tests/parser_golden.rs` scans all fixtures
  and expects `ParserRegistry::with_defaults` to parse them.
- Fuzz: `fuzz/fuzz_targets/language_parsers.rs` constructs all parser structs.
- Other direct grammar use: `packages/atlas-parser/src/ast_helpers.rs` and
  `parse_runtime.rs` tests use `tree_sitter_bash`; `registry.rs` test
  `bundled_grammars()` lists all seven.

### 1.4 Missing safety net

No parser-config fingerprint is persisted. If the parser set or config changes,
`atlas update` can leave stale rows from the old parser for unchanged files. A
full `atlas build` already clears all file graphs for the repo before rebuilding.

---

## 2. Target Design

### 2.1 Config surface

Reuse `[parsers.external]`. Add an optional `extractor` field for known
languages. Keep the generic node-kind path for custom languages.

Profile form:

```toml
[[parsers.external]]
extractor = "php"
# language_name = "php"          # optional; profile default
# extensions = ["php"]           # optional; profile default
grammar_dir = "grammars/tree-sitter-php"
```

Prebuilt-library form:

```toml
[[parsers.external]]
extractor = "csharp"
lib_path = "grammars/libtree-sitter-c-sharp.so"
# lib_function = "tree_sitter_c_sharp"  # profile default
```

Generic form stays unchanged:

```toml
[[parsers.external]]
language_name = "zig"
extensions = ["zig"]
grammar_dir = "grammars/tree-sitter-zig"

[[parsers.external.symbols]]
tree_kind = "function_declaration"
node_kind = "function"
```

Profile defaults:

| Extractor | Canonical `language_name` | Default extensions | Default lib function |
|---|---|---|---|
| `php` | `php` | `php` | `tree_sitter_php` |
| `cpp` | `cpp` | `cpp`, `cc`, `cxx` | `tree_sitter_cpp` |
| `csharp` | `csharp` | `cs` | `tree_sitter_c_sharp` |
| `java` | `java` | `java` | `tree_sitter_java` |
| `scala` | `scala` | `scala`, `sc` | `tree_sitter_scala` |
| `ruby` | `ruby` | `rb` | `tree_sitter_ruby` |
| `bash` | `bash` | `sh`, `bash` | `tree_sitter_bash` |

Validation rules:

- profile/rule/query modes require exactly one of `grammar_dir` or `lib_path`
- plugin mode requires `command` and must omit `grammar_dir`, `lib_path`,
  `extractor`, `symbols`, and query fields
- plugin-only fields (`command`, `args`, `working_dir`, `protocol`,
  `timeout_ms`, `max_response_bytes`) are rejected in profile/rule/query modes
- exactly one extraction mode: profile, rule-based, query, or plugin
- `extractor` and non-empty `symbols` are mutually exclusive
- generic config requires non-empty `extensions` and `symbols`
- profile config may omit `language_name`, `extensions`, and `lib_function`
- profile `language_name`, when provided, must match the canonical name
- generic config still requires non-empty `language_name`
- plugin config requires non-empty `language_name` and `extensions`
- query mode requires exactly one of `query_path` or inline `query`
- empty strings and leading-dot extensions stay rejected
- duplicate extensions across configs stay rejected
- `call_node_kinds` / `call_target_field` are generic-only

Normalization runs in `Config::load` after `resolve_from` and before
`validate`, so downstream code sees effective defaults.

### 2.2 Types and API

Add to `atlas-parser`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalExtractor {
    Php,
    Cpp,
    #[serde(rename = "csharp")]
    CSharp,
    Java,
    Scala,
    Ruby,
    Bash,
}
```

impl ExternalExtractor {
    pub fn language_name(self) -> &'static str;
    pub fn default_extensions(self) -> &'static [&'static str];
    pub fn default_lib_function(self) -> &'static str;
}
```

`ExternalParserConfig` additions:

```rust
#[serde(default)]
pub extractor: Option<ExternalExtractor>,

#[serde(default)]
pub language_name: String,
#[serde(default)]
pub extensions: Vec<String>,
#[serde(default)]
pub symbols: Vec<ExternalSymbolRule>,
#[serde(default)]
pub imports: Vec<ExternalImportRule>,
#[serde(default)]
pub attributes: Vec<ExternalAttributeRule>,
#[serde(default)]
pub qn_style: ExternalQnStyle,
#[serde(default)]
pub owner_node_kinds: Vec<NodeKind>,
#[serde(default)]
pub test_name_patterns: Vec<String>,
#[serde(default)]
pub test_path_patterns: Vec<String>,
#[serde(default)]
pub call_receiver_field: Option<String>,
#[serde(default)]
pub query_path: Option<String>,
#[serde(default)]
pub query: Option<String>,
#[serde(default)]
pub command: Option<String>,
#[serde(default)]
pub args: Vec<String>,
#[serde(default)]
pub working_dir: Option<String>,
#[serde(default)]
pub protocol: Option<String>,
#[serde(default)]
pub timeout_ms: Option<u64>,
#[serde(default)]
pub max_response_bytes: Option<u64>,
```

`ExternalSymbolRule` additions:

```rust
#[serde(default)]
pub params_field: Option<String>,
#[serde(default)]
pub return_type_field: Option<String>,
#[serde(default)]
pub modifier_node_kinds: Vec<String>,
```

`ExternalParserConfig` methods:

```rust
pub fn apply_profile_defaults(&mut self); // fills only empty fields
pub fn validate(&self) -> anyhow::Result<()>;
```

`ParsersConfig::validate` delegates per-entry checks to
`ExternalParserConfig::validate`, then performs cross-entry duplicate-extension
checks.

Profile parser construction:

```rust
impl ExternalExtractor {
    pub(crate) fn build(
        self,
        language: tree_sitter::Language,
        extensions: Vec<String>,
    ) -> anyhow::Result<Box<dyn LangParser>>;
}
```

The wrapper enforces configured extensions while delegating parse work to the
existing extractor:

```rust
struct ExternalProfileParser {
    inner: Box<dyn LangParser>,
    language_name: Cow<'static, str>,
    extensions: Vec<String>,
}
```

`ExternalLangParser` becomes a thin wrapper around either the generic parser or
the profile parser. `LangParser` itself does not change.

Add a preloaded-language constructor for embedding and parity tests:

```rust
impl ExternalLangParser {
    pub fn from_language(
        config: &ExternalParserConfig,
        language: tree_sitter::Language,
    ) -> anyhow::Result<Self>;
}
```

`ExternalLangParser::new` becomes `load_external_language` + `from_language`.
Both constructors normalize a clone of the config before building the parser.

### 2.3 Grammar loading

Split `external.rs` into a module directory before adding profile support:

```text
packages/atlas-parser/src/external/
  mod.rs       # config types, re-exports, validation
  loader.rs    # grammar_dir/lib_path loading + ABI validation
  generic.rs   # current node-kind parser and collectors
  profile.rs   # ExternalExtractor + profile parser construction
```

`loader.rs` owns one function:

```rust
pub(crate) fn load_external_language(
    config: &ExternalParserConfig,
) -> anyhow::Result<tree_sitter::Language>;
```

It resolves the profile default lib function first, validates the grammar ABI,
and returns errors containing the language name and source path.

### 2.4 Parser-owned grammar

Each of the seven parsers gains a grammar field and constructor:

```rust
pub struct PhpParser {
    language: tree_sitter::Language,
}

impl PhpParser {
    pub fn new(language: tree_sitter::Language) -> Self;
}
```

Extraction helpers receive the loaded language:

```rust
impl PhpSyntaxFacts {
    fn extract(
        root: TsNode<'_>,
        source: &[u8],
        language: &tree_sitter::Language,
    ) -> Result<Self, String>;
}
```

Each module also exposes a construction-time query check:

```rust
pub(crate) fn validate_query(language: &tree_sitter::Language) -> Result<(), String>;
```

`ExternalExtractor::build` calls it before returning a parser. A grammar that is
ABI-compatible but missing query node kinds fails at config load/build start
instead of panicking mid-scan.

### 2.5 Registry behavior

Final state:

- `ParserRegistry::with_defaults` registers only still-bundled languages.
- `ParserRegistry::register_externals` registers generic or profile parsers.
- The seven offloaded languages are absent from the default registry.
- Profile parsers emit the canonical language name, preserving
  `Node.language`, QNames, and existing graph contracts.

### 2.6 Parser config fingerprint

Add `packages/atlas-engine/src/parser_fingerprint.rs`:

```rust
pub const PARSER_FINGERPRINT_REVISION: u32 = 1;

pub fn compute(external_parsers: &[ExternalParserConfig]) -> String;
```

Fingerprint input:

- revision constant
- default registry language identity set
- normalized `[parsers.external]` entries (normalize a clone inside `compute`)
- extraction mode: profile, rules, query, or plugin
- query path plus query-text hash
- plugin command, args, working directory, timeout, and response cap
- effective extractor/profile defaults
- grammar paths and lib function names

Build and update both receive the effective external parser list through
`BuildOptions` / `UpdateOptions`, so the fingerprint does not require a full
`Config` in the hot path.

Use SHA-256 over canonical JSON. Parser extractor behavior changes must bump the
revision constant.

Persist in the existing `metadata` table, no schema migration:

- key: `atlas_parser_fingerprint:<source_repo_id>`
- `Store::parser_fingerprint(source_repo_id)`
- `Store::set_parser_fingerprint(source_repo_id, value)`

Build/update rules:

- successful non-dry-run `build_graph` writes the current fingerprint
  (degraded builds included; failed builds do not)
- `update_graph` reads it before parsing
- missing fingerprint => require full `atlas build`
- fingerprint mismatch => return an actionable error; do not update a graph
  produced by a different parser set
- `atlas build` clears stale file graphs and writes the new fingerprint
- `status` / `doctor` report stale or missing fingerprint

This is fail-closed by design: correctness first, convenience second.

### 2.7 Watch mode

`WatchRunner` gains:

```rust
external_parsers: Vec<ExternalParserConfig>,
```

and a builder:

```rust
pub fn with_external_parsers(mut self, configs: Vec<ExternalParserConfig>) -> Self;
```

`run_watch` passes `engine_config.parsers.external.clone()`.
`apply_batch` copies the configured list into `UpdateOptions`.
Extract an `update_options_for_batch` helper so the forwarding behavior is unit
testable without starting the watcher loop.

`watch.rs` is 897 LOC today. Keep additions small; if the file crosses 1,000 LOC,
move the runner loop and tests into a submodule instead of growing the file.

### 2.8 Generic external parser limitation fixes

Profiles preserve built-in extraction for the seven known languages. Generic
external languages get a bounded, config-driven fidelity upgrade. This is not
full parity; exact semantics remain profile-only.

New config surface:

```toml
[[parsers.external]]
language_name = "zig"
extensions = ["zig"]
grammar_dir = "grammars/tree-sitter-zig"
qn_style = "flat_owner"             # nested (default) | flat_owner
owner_node_kinds = ["class", "struct"]

test_name_patterns = ["^test_"]
test_path_patterns = ["_test[.]zig$"]

[[parsers.external.symbols]]
tree_kind = "function_declaration"
node_kind = "function"
name_field = "name"
params_field = "parameters"
return_type_field = "return_type"
modifier_node_kinds = ["visibility_modifier"]

[[parsers.external.imports]]
tree_kind = "import_declaration"
name_field = "path"

[[parsers.external.attributes]]
tree_kind = "attribute"
name_field = "name"
attach_to = "next_symbol"       # scope (default) | next_symbol

# call_node_kinds = ["call_expression"]
# call_target_field = "function"
# call_receiver_field = "object"
```

Behavior:

- imports emit `NodeKind::Import` nodes plus `Imports` edges from file scope;
  `name_field` falls back to the last named child when unset
- attributes emit `NodeKind::Variable` nodes plus `Contains` edges.
  `attach_to = "scope"` attaches to the current scope; `attach_to =
  "next_symbol"` buffers sibling attributes and attaches them to the following
  symbol. Exact built-in ownership still comes from profiles.
- symbol metadata fills `params`, `return_type`, and `modifiers`;
  `modifier_node_kinds` scans direct child nodes
- `test_name_patterns` / `test_path_patterns` are regexes compiled once at
  parser construction and set `Node.is_test` on match
- `qn_style = "nested"` keeps current behavior; `qn_style = "flat_owner"` uses
  `owner_node_kinds` to emit `file::method::Owner.name`-style QNs. Exact built-in
  QN rules still come only from profiles.
- generic call edges use the shared `call_edge` helper: confidence `0.8` with
  tier `same_file` when resolved in-file, `0.3` with tier `text` otherwise.
  Unresolved calls no longer claim `1.0` / `definite`.
- `call_receiver_field` fills `extra_json.receiver_text`
- profile configs reject all generic-only fields (`symbols`, `imports`,
  `attributes`, `call_*`, `qn_style`, `owner_node_kinds`, test rules, query
  fields, and plugin fields)

Validation rules extend to non-empty `tree_kind` / `name_field` for import and
attribute rules, non-empty owner kinds when `flat_owner` is selected, and valid
regexes for test patterns.

New types:

```rust
pub struct ExternalImportRule {
    pub tree_kind: String,
    #[serde(default)]
    pub name_field: Option<String>,
}

pub struct ExternalAttributeRule {
    pub tree_kind: String,
    #[serde(default = "default_name_field")]
    pub name_field: String,
    #[serde(default)]
    pub attach_to: ExternalAttributeTarget,
}

#[serde(rename_all = "snake_case")]
pub enum ExternalAttributeTarget {
    Scope,
    NextSymbol,
}

#[serde(rename_all = "snake_case")]
pub enum ExternalQnStyle {
    Nested,
    FlatOwner,
}
```

`generic.rs` may grow past 1,000 LOC with these collectors. Split it into
`generic/{symbols,calls,imports,attributes}.rs` if needed; do not keep growing a
single oversized module.

### 2.9 SCM query/capture extraction

For custom languages whose grammar does not map cleanly to node-kind rules, add
an optional query extraction mode. This reuses the existing Atlas `@atlas.*`
capture contract documented in `packages/atlas-parser/README.md`.

Config:

```toml
[[parsers.external]]
language_name = "zig"
extensions = ["zig"]
grammar_dir = "grammars/tree-sitter-zig"
query_path = "queries/zig.scm"
# or inline: query = "(function_declaration name: (identifier) @atlas.name) @atlas.definition.function"
qn_style = "nested"
test_name_patterns = ["^test_"]
```

Extraction modes are mutually exclusive:

- profile: `extractor`
- rule-based: `symbols`
- query-based: `query_path` or inline `query`
- plugin: `command`

Query behavior:

- query text compiles against the loaded language at construction; invalid
  queries fail config load with capture/pattern diagnostics
- supported captures: definitions (`@atlas.definition.*`), `@atlas.name`,
  `@atlas.import`, `@atlas.attribute`, `@atlas.call`, `@atlas.receiver`,
  `@atlas.parameters`, `@atlas.return_type`, `@atlas.reference`
- query mode emits the same `Node`/`Edge` shapes as rule mode, including imports,
  attributes, calls, params/return, and `is_test` via test patterns
- `@atlas.reference` emits reference edges when present
- `qn_style`, `owner_node_kinds`, and test patterns still apply; QN remains
  Atlas-owned
- relative `query_path` resolves from `.atlas/`, matching grammar path rules
  (write `queries/zig.scm`, not `.atlas/queries/zig.scm`)
- `symbols`, `imports`, `attributes`, and `call_*` are rejected when query mode
  is active
- query captures are syntax semantics, not arbitrary code; exact built-in
  parity still requires profiles

Implement in `external/query.rs`. Add tests with inline Rust queries against the
vendored Rust grammar: definition/name, import, call, receiver, params/return,
attribute, and invalid-query rejection.

### 2.10 Out-of-process parser plugin protocol

For arbitrary language semantics without a Rust ABI, add a versioned JSON/stdio
plugin protocol. The plugin owns extraction completely and returns a serialized
`ParsedFile`.

Config:

```toml
[[parsers.external]]
language_name = "custom"
extensions = ["cust"]
command = "parsers/custom-parser"
args = ["--stdio"]
# working_dir defaults to repo root (parent of .atlas/)
# working_dir = ".."
timeout_ms = 30000
max_response_bytes = 16777216
# protocol defaults to atlas-parser.v1
```

Request/response:

```json
{
  "protocol": "atlas-parser.v1",
  "language_name": "custom",
  "path": "src/file.cust",
  "file_hash": "sha256...",
  "source_b64": "..."
}
```

```json
{
  "protocol": "atlas-parser.v1",
  "parsed_file": { "path": "src/file.cust", "hash": "sha256...", "nodes": [], "edges": [] }
}
```

Runner rules:

- spawn with argv vector, no shell interpolation
- relative `command` and `working_dir` resolve from `.atlas/`; absolute paths
  allowed
- `working_dir` defaults to repo root (parent of `.atlas/`)
- per-file process in v1; persistent worker mode is future work
- enforce timeout, kill on expiry, cap response bytes, cap stderr capture
- nonzero exit, invalid JSON, protocol mismatch, or path/hash mismatch is a
  parse error for that file
- plugin output is validated against `ParsedFile` and normalized path/hash;
  node/edge file paths must match the request
- plugin execution is trusted code execution; no OS sandbox is provided

Implement in `external/plugin.rs` and `external/protocol.rs`. Add protocol
serialization tests plus Unix-gated fixture executable tests for success,
timeout, malformed JSON, oversized response, and hash mismatch. Keep the Rust
dylib path explicitly out of scope.

---

## 3. Implementation Phases

Execution status lives in [`ISSUES.md` § External Grammar Offload Patch](../ISSUES.md#external-grammar-offload-patch).
This section is the design reference; keep the issue checklist aligned when scope
changes.

### Phase 0 — Baseline and fixture inventory

- Record current parser behavior with `cargo test -p atlas-parser`
- Confirm all seven fixture directories exist under
      `packages/atlas-parser/tests/fixtures/`
- Add a fixture inventory assertion for the seven offloaded languages in
      `packages/atlas-parser/tests/parser_golden.rs`
- Record release binary size and `cargo tree -p atlas-cli -e normal` output
      for the before/after comparison
- Do not change graph output in this phase

Exit criteria: parser tests green; baseline recorded.

### Phase 1 — Inject grammar into seven extractors (no behavior change)

For each of `php.rs`, `cpp.rs`, `csharp.rs`, `java.rs`, `scala.rs`, `ruby.rs`,
`bash.rs`:

- Replace the unit struct with a `language: tree_sitter::Language` field
- Add `pub fn new(language: tree_sitter::Language) -> Self`
- Use `parser.set_language(&self.language)` in `parse`
- Pass the language into `*SyntaxFacts::extract`
- Add `pub(crate) fn validate_query(language: &Language) -> Result<(), String>`
- Update module unit tests to construct the parser with the same grammar
      crate as before
- Keep `ParserRegistry::with_defaults` unchanged in this phase

Exit criteria:

- `cargo test -p atlas-parser`
- golden JSON files unchanged
- no QName, node, or edge diffs

### Phase 2 — Add profile support to the external parser

- Split `external.rs` into `external/{mod,loader,generic,profile}.rs`
- Add `ExternalExtractor` with names, extensions, and lib-function defaults
- Add `ExternalParserConfig.extractor`
- Make `language_name`, `extensions`, and `symbols` serde-defaulted
- Add `apply_profile_defaults` and `validate`
- Implement `load_external_language`
- Implement `ExternalExtractor::build`
- Implement `ExternalProfileParser` extension override wrapper
- Make `ExternalLangParser::new` dispatch generic vs profile
- Update `packages/atlas-engine/src/config/parsers.rs` to use the shared
      validation method
- Call `apply_profile_defaults` from `Config::load`
- Add unit tests for:
      - profile defaults
      - profile/generic conflict rejection
      - canonical language-name mismatch
      - C# and Java lib-function defaults
      - query validation failure with an incompatible grammar
- Keep bundled parsers registered; test profile construction directly with
      dev-dependency grammar crates

Exit criteria:

- generic external config behavior unchanged
- profile parser construction succeeds for all seven dev-dependency grammars
- invalid profile config fails before repository scan

### Phase 3 — Remove seven parsers from the default registry and normal dependencies

- Remove `BashParser`, `CSharpParser`, `PhpParser`, `CppParser`, `JavaParser`,
      `ScalaParser`, `RubyParser` imports and registrations from
      `ParserRegistry::with_defaults`
- Remove the seven grammar crates from `packages/atlas-parser/Cargo.toml`
      `[dependencies]`
- Add the seven grammar crates to `[dev-dependencies]` for parser tests
- Add the seven grammar crates to `fuzz/Cargo.toml` and construct parsers with
      explicit languages in `fuzz/fuzz_targets/language_parsers.rs`
- Update `registry.rs` tests:
      - remove the seven from `bundled_grammars`
      - assert the default registry no longer supports their extensions
      - assert profile registration restores support
- Update `parser_golden.rs` to register the seven profile parsers with
      `ExternalLangParser::from_language` and dev-dependency grammars before
      parsing fixtures
- Keep all existing `.golden.json` files unchanged
- Update `ast_helpers.rs` and `parse_runtime.rs` tests to rely on the bash
      dev-dependency
- Rewrite CLI quality-gate fixtures that assume bundled support:
      - `build_and_update_replace_compiled_language_file_graphs` uses Java, C#,
        PHP, and C++; keep pipeline coverage with still-bundled languages
      - `build_and_update_replace_markup_shell_file_graphs` uses Bash; replace or
        configure explicitly
      - `build_and_update_replace_scala_ruby_file_graphs` uses Scala and Ruby;
        move language-specific assertions to parser goldens
      - keep CLI tests focused on build/update replacement behavior, not bundled
        extraction coverage
- Add an integration test in `parser_golden.rs` that each offloaded
      fixture parses through `ExternalLangParser::from_language` and matches the
      existing golden

Exit criteria:

- fixture goldens pass byte-for-byte through profile parsers
- `cargo tree -p atlas-cli -e normal` contains none of the seven grammar crates
- `cargo build --release` succeeds
- release binary size delta recorded

### Phase 4 — Config UX, docs, and diagnostics

- Add profile examples to the commented template in
      `packages/atlas-engine/src/config/template.rs`
- Update README § External languages:
      - profile form first
      - generic form second
      - query/capture form
      - out-of-process plugin form
      - complete prebuilt example with required `symbols` only for generic
      - generic import/attribute/metadata/test/QN/call fields
      - profile-only exact QN, attribute ownership, and `extra_json` behavior
      - grammar checkout layout (`grammar_dir` needs `src/parser.c`)
      - C# lib-function spelling note
      - trusted-grammar/plugin warning
- Update the stale supported-language list in README
- Update `packages/atlas-parser/README.md` public-surface and migration notes
- Update `CHANGELOG.md` with the breaking default-language change
- Update `docs/threat-model-serve.md` or parser docs with the
      compile/load-code/plugin trust boundary
- Add an `atlas doctor` check:
      - scan tracked files
      - report offloaded extensions that have no registered parser
      - include a copyable `[[parsers.external]]` hint
- Keep `status`/`doctor` tolerant of missing grammars: warn, do not crash

Exit criteria:

- no docs claim watch mode ignores external parsers
- prebuilt example passes config validation
- doctor warns for a fixture repo containing unconfigured PHP/C++ files

### Phase 5 — Generic external parser limitation fixes

- Add `ExternalImportRule` and `ExternalAttributeRule` config types
- Add symbol-rule fields: `params_field`, `return_type_field`,
      `modifier_node_kinds`
- Add parser fields: `qn_style`, `owner_node_kinds`, `test_name_patterns`,
      `test_path_patterns`, `call_receiver_field`
- Emit import nodes and `Imports` edges from file scope
- Emit attribute nodes and `Contains` edges; implement `attach_to = scope`
      and `attach_to = next_symbol`
- Fill `params`, `return_type`, and `modifiers` from configured fields
- Set `Node.is_test` from compiled name/path regexes
- Implement `nested` and `flat_owner` QN styles
- Replace generic call-edge construction with `common::call_edge` so
      confidence and tiers are calibrated
- Populate `receiver_text` when `call_receiver_field` is configured
- Reject generic-only fields when `extractor` is set
- Extend generic tests using the vendored Rust grammar:
      - `use_declaration` import rule emits import node and edge
      - function params/return/modifiers are populated
      - attribute rule emits variable node for both `attach_to` modes
      - name/path regex sets `is_test`; invalid regex is rejected
      - `flat_owner` QN shape
      - unresolved call uses `text` tier, not `definite`
      - receiver metadata is emitted
- Keep profile output byte-identical; generic changes must not touch
      profile parsers

Exit criteria:

- generic external config supports imports, attribute ownership modes,
      symbol metadata, test regexes, configurable QN style, and receiver-aware
      calls
- unresolved generic calls never claim `1.0` / `definite`
- profile parsers still match all goldens
- generic field rejection for profile configs is tested

### Phase 6 — Watch mode and parser fingerprint

- Add `external_parsers` and `with_external_parsers` to `WatchRunner`
- Pass config from `run_watch`
- Extract and test `update_options_for_batch`
- Add `parser_fingerprint.rs` with revision and canonical hash
- Add metadata getter/setter in
      `packages/atlas-store-sqlite/src/store/build_state.rs`
- Write fingerprint after successful non-dry-run build
- Reject stale/missing fingerprint in `update_graph` with an actionable error
- Record fingerprint after successful update
- Surface stale fingerprint in `status` and `doctor`
- Add tests:
      - fingerprint stable for equal config
      - fingerprint changes for grammar path, extractor, extension, extraction
        mode, query text, or plugin command changes
      - update rejects mismatched fingerprint
      - full build clears stale rows and writes new fingerprint
      - watch passes external parsers into update options

Exit criteria:

- config change cannot leave mixed-parser graph rows
- watch updates external-language files
- full build is the documented and enforced recovery path

### Phase 7 — SCM query/capture extraction

- Add `query_path` and inline `query` config fields
- Validate exactly one of `query_path` or `query`
- Resolve relative `query_path` from `.atlas/`
- Add `ExternalExtractionMode` validation: exactly one of `extractor`,
      `symbols`, query, or `command`
- Implement `external/query.rs` using `compile_query` / `run_query`
- Map `@atlas.definition.*`, `@atlas.name`, `@atlas.import`,
      `@atlas.attribute`, `@atlas.call`, `@atlas.receiver`,
      `@atlas.parameters`, `@atlas.return_type`, and `@atlas.reference` captures
- Reuse `qn_style`, owner scopes, import/attribute emitters, and test regexes
- Reject `symbols`, `imports`, `attributes`, and `call_*` with query mode
- Add tests with inline Rust queries:
      - definition and name
      - import
      - call and receiver
      - params/return
      - attribute
      - invalid query fails construction with a clear error

Exit criteria:

- query mode produces `ParsedFile` nodes/edges without node-kind rules
- query compile failure is a config error, not a parse panic
- query mode cannot be combined with rule-based or profile fields

### Phase 8 — Out-of-process parser plugin protocol

- Add `command`, `args`, `working_dir`, `protocol`, `timeout_ms`,
      `max_response_bytes` config fields
- Add `external/protocol.rs` with versioned request/response types
- Add `external/plugin.rs` implementing `LangParser` via child process
- Allow plugin mode without `grammar_dir` / `lib_path`
- Resolve relative `command` and `working_dir` from `.atlas/`
- Spawn argv directly, no shell interpolation
- Set repo root as working directory
- Enforce timeout, kill on expiry, cap stdout/stderr sizes
- Validate response protocol version, JSON shape, path, file hash, and
      node/edge file paths
- Treat nonzero exit, malformed JSON, oversize response, timeout, and
      identity mismatch as per-file parse errors
- Reject extraction fields that conflict with plugin mode
- Add protocol serialization tests
- Add Unix-gated fixture executable tests for success, timeout, malformed
      JSON, oversized response, and hash mismatch
- Document trusted-code execution and absence of OS sandbox

Exit criteria:

- plugin can return a complete `ParsedFile` without Rust ABI coupling
- malformed or hostile plugin output cannot panic the parser process
- timeout and response-size caps are enforced
- Rust `dylib` path remains absent

### Phase 9 — Release validation

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test -p atlas-parser`
- `cargo test -p atlas-engine`
- `cargo test -p atlas-store-sqlite`
- `cargo test -p atlas-cli`
- `cargo test --workspace`
- `cargo build --release`
- Verify normal dependency graph excludes the seven grammar crates
- Manual fixture repo:
      - no config => build reports seven extensions as unsupported
      - with profile config => build parses them and matches goldens
      - with query config => query captures produce expected nodes/edges
      - with plugin config => fixture plugin produces expected `ParsedFile`
      - update modifies one offloaded file and updates its graph
      - watch modifies one offloaded file and updates its graph
      - config change => update fails closed until full build
- Record final binary-size delta in the change summary

---

## 4. Limitation Closure Matrix

Every limitation identified for the current external path is either fixed here or
explicitly declared profile-only / non-goal.

| Limitation | Fix in this plan | Coverage |
|---|---|---|
| No import/include/using/require/source edges | Profile extractors preserve all import nodes/edges for the seven languages; generic path gains `[[parsers.external.imports]]` rules emitting `NodeKind::Import` plus `Imports` edges | Phases 3 and 5 |
| Missing params/return/modifiers | Profile extractors preserve existing fields; generic symbol rules gain `params_field`, `return_type_field`, `modifier_node_kinds` | Phases 1 and 5 |
| Missing attributes/annotations | Profile extractors preserve PHP/C#/Java attributes; generic path gains `[[parsers.external.attributes]]` rules emitting `Variable` nodes and `Contains` edges, with `attach_to = scope` or `next_symbol` | Phases 3 and 5 |
| Test detection absent | Seven profiles keep current built-in behavior (no regression); generic path gains compiled name/path regexes setting `Node.is_test` | Phase 5 |
| External QNs differ from built-in QNs | Profiles use the existing extractors and preserve exact QNs; generic path gains `qn_style` with values `nested` or `flat_owner` plus `owner_node_kinds` | Phases 1, 3, and 5 |
| Call edges lack receiver and claim `1.0` / `definite` when unresolved | Profiles preserve existing call behavior; generic path gains `call_receiver_field` and uses `common::call_edge` (`0.8` same-file, `0.3` text) | Phase 5 |
| Namespace/package hierarchy semantics incomplete | Profiles preserve exact namespace/package handling; generic path can only approximate owner scope via `flat_owner` | Phases 1 and 5; generic documented best-effort |
| PHP/C#/Ruby/Bash special cases (namespace/use, attributes, require/mixins, `source`) | Profiles keep the existing extractors and all special-case logic | Phase 3 |
| C++ out-of-line methods and template modifier | Profiles keep `method_parts`, template detection, and `templated` extra metadata; generic rules cannot infer them | Phase 3 |
| Scala `case_class` QN and `extra_json` distinction | Profiles preserve it; plugin can return arbitrary `extra_json`; generic rule/query mode cannot infer it automatically | Phase 3 and Phase 8 |
| Watch mode ignores external parsers | `WatchRunner` carries config into `UpdateOptions` | Phase 6 |
| Parser config changes leave stale graph rows | Persisted parser fingerprint; update fails closed; full build required | Phase 6 |
| Grammar ABI mismatch or query incompatibility | Loader ABI validation plus construction-time query validation | Phase 2 |
| No C compiler / grammar source unavailable | `grammar_dir` compile path plus `lib_path` prebuilt path; docs and doctor hints | Phases 2 and 4 |
| User wants arbitrary language semantics | SCM `@atlas.*` query mode for custom extraction; out-of-process JSON plugin for arbitrary logic | Phases 7 and 8 |
| Out-of-process parser plugin | Versioned stdio protocol, argv spawn, timeout, size caps, path/hash validation | Phase 8 |
| Runtime Rust `dylib` parser plugin | Explicitly excluded: unstable Rust ABI, version coupling, unsafe boundary | Non-goal |

## 5. Test Matrix

| Layer | Test | Purpose |
|---|---|---|
| parser unit | existing module tests | extraction unchanged after grammar injection |
| parser integration | `parser_golden.rs` with profile registry | byte-identical `ParsedFile` vs old goldens |
| external unit | profile defaults table | canonical names/extensions/lib functions |
| external unit | generic import/attribute rules | import nodes/edges and attribute nodes/edges |
| external unit | generic metadata fields | params, return type, modifiers, test regex |
| external unit | generic QN styles | nested default and flat_owner shape |
| external unit | generic call confidence | same-file `0.8`, unresolved `0.3` / `text` |
| external unit | profile rejects generic fields | profile/generic separation |
| external unit | config conflict validation | extractor vs symbols, paths, extensions |
| external unit | query validation | clear error for incompatible grammar |
| external unit | query capture mapping | definitions, imports, attributes, calls, receivers, metadata |
| external unit | plugin protocol roundtrip | versioned request/response schema |
| external integration | Unix plugin fixture | success, timeout, malformed, oversize, hash mismatch |
| engine unit | config load/normalize | defaults applied before validation |
| engine unit | parser fingerprint | stable and change-sensitive |
| engine integration | build/update fingerprint | stale config rejected |
| store unit | metadata roundtrip | fingerprint persistence |
| watch unit | update option forwarding | external config reaches update |
| CLI test | doctor unsupported-extension warning | migration guidance |
| fuzz | `language_parsers` with explicit grammars | parser panic safety after constructor change |

No test compiles external C grammars for the seven in normal CI. Loader behavior
is covered by the existing vendored Rust grammar test. Profile extraction is
covered by dev-dependency grammar crates and unchanged goldens.

---

## 6. File Touch Map

Parser crate:

- `packages/atlas-parser/src/external.rs` -> `external/{mod,loader,generic,profile,query,plugin,protocol}.rs`
- generic limitation tests live in `external/generic.rs` unit tests using the
  existing vendored Rust grammar fixture
- query tests live in `external/query.rs` with inline Rust queries
- plugin protocol tests live in `external/protocol.rs` and `external/plugin.rs`
- Unix plugin fixture under `packages/atlas-parser/tests/fixtures/parser-plugin.sh`
- `packages/atlas-parser/src/lang/{php,cpp,csharp,java,scala,ruby,bash}.rs`
- `packages/atlas-parser/src/registry.rs`
- `packages/atlas-parser/src/lib.rs`
- `packages/atlas-parser/Cargo.toml`
- `packages/atlas-parser/tests/parser_golden.rs`
- `packages/atlas-parser/tests/fixtures/**` existing parser fixtures unchanged;
  add `tests/fixtures/parser-plugin.sh` for plugin tests
- `packages/atlas-parser/src/ast_helpers.rs` tests only
- `packages/atlas-parser/src/parse_runtime.rs` tests only

Fuzz:

- `fuzz/Cargo.toml`
- `fuzz/fuzz_targets/language_parsers.rs`

Engine:

- `packages/atlas-engine/src/config/parsers.rs`
- `packages/atlas-engine/src/config/load.rs`
- `packages/atlas-engine/src/config/template.rs`
- `packages/atlas-engine/src/parser_fingerprint.rs` (new)
- `packages/atlas-engine/src/build.rs`
- `packages/atlas-engine/src/update.rs`
- `packages/atlas-engine/src/watch.rs`
- `packages/atlas-engine/src/lib.rs`
- `packages/atlas-engine/Cargo.toml` (sha2 if not already available)
- `packages/atlas-engine/src/config/tests/**`

Store:

- `packages/atlas-store-sqlite/src/store/build_state.rs`

CLI:

- `packages/atlas-cli/src/commands/graph/watch.rs`
- `packages/atlas-cli/src/commands/maintenance.rs` (doctor check)
- `packages/atlas-cli/tests/cli_quality_gates/resolution.rs`
- CLI tests for doctor/config output

Docs:

- `README.md`
- `packages/atlas-parser/README.md`
- `CHANGELOG.md`
- `docs/threat-model-serve.md`
- `docs/architecture.md` if it still describes all grammars as bundled

---

## 7. Risks and Mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Users lose out-of-box support for seven languages | silent graph loss | doctor warning, README migration section, `skipped_unsupported` surfaced in build output |
| User grammar is ABI-compatible but query-incompatible | panic or wrong graph | construction-time query validation; clear error names grammar path and language |
| No C compiler on user machine | build/update failure | prebuilt `lib_path` path documented and tested |
| Grammar lib path loads untrusted native code | code execution | keep existing warning; document trust boundary; never auto-download |
| QName or output drift | history/context breakage | extractors unchanged; golden parity gate before removing registry entries |
| Config defaults drift from built-in behavior | graph differences | single default table in `ExternalExtractor`; tests assert canonical names/extensions/lib functions |
| Generic config cannot match all built-in semantics | weaker graph for custom languages | rule mode is best-effort; query mode for custom captures; plugin mode for arbitrary logic; profiles remain exact |
| Generic attribute ownership differs from built-ins | attributes attach incorrectly | support `attach_to = scope` and `attach_to = next_symbol`; profiles remain exact |
| `flat_owner` QN approximation differs from built-ins | custom-language QNs may not match a future built-in | default remains `nested`; document approximation; profiles are the only exact QN path |
| Query captures reference nodes absent from the chosen grammar version | query compile failure | compile queries at parser construction; surface capture/pattern error with grammar path |
| Plugin executes configured native code | arbitrary code execution | document trusted-code boundary; no auto-download; argv spawn; timeout and output caps; path/hash validation |
| Plugin protocol drift or hostile output | parse corruption or panic | version handshake; strict JSON schema; reject malformed/oversize/wrong-identity responses |
| Per-file plugin process overhead | slow builds for plugin languages | v1 per-file execution; document cost; persistent worker is future work |
| Stale graph after parser set change | mixed parser rows | parser fingerprint; fail-closed update; full build recovery |
| Watch keeps stale config after config edit | no refresh until restart | document restart; fingerprint/update check surfaces mismatch |
| Absolute grammar paths change | unnecessary rebuild | acceptable; path is part of effective parser identity |
| `watch.rs` grows past file threshold | maintainability rule violation | extract runner/tests into submodule if near 1,000 LOC |

---

## 8. Rollout and Migration

Breaking change: the seven languages are no longer bundled.

User migration:

1. Obtain a compatible grammar checkout or prebuilt shared library.
2. Put it under `.atlas/grammars/` or use an absolute path. Relative config
   values resolve from `.atlas/`, so use `grammars/tree-sitter-php`, not
   `.atlas/grammars/tree-sitter-php`.
3. Add a `[[parsers.external]]` entry with `extractor = "<language>"` and
   `grammar_dir` or `lib_path`.
4. Run `atlas build` once. Update refuses to run until the graph is rebuilt with
   the new parser set.

Repository migration:

- Add a CHANGELOG entry under `Unreleased` / breaking changes.
- Keep all seven extractors in the codebase; only grammar loading moves.
- Do not preserve a fallback to compile-time grammar crates. Optional fallback
  would defeat the offload goal and hide config errors.
- Keep workspace grammar pins for dev-dependencies and fuzz so test versions
  match the goldens.
- Generic parser additions are additive: old generic configs keep current output
  until they opt into new rules. Profile configs must never accept generic-only
  fields.
- Query and plugin modes are opt-in; no existing config selects them implicitly.
- Rust `dylib` loading is intentionally not implemented.

Rollback:

- restore previous binary
- run `atlas build` to restore bundled-parser graph output
- parser fingerprint mismatch forces the rebuild

---

## 9. Acceptance Criteria

- All seven offloaded language fixture sets pass byte-for-byte through profile parsers
- Default registry supports only still-bundled languages
- Profile config restores each offloaded language with canonical language
      name, QNames, imports, metadata, and calls
- Normal release dependency graph excludes the seven grammar crates
- Release binary size reduction recorded
- Existing generic configs without the new fields produce identical output
- Generic external config supports imports, attribute ownership modes,
      params/return/modifiers, test regexes, `qn_style`, and receiver-aware calls
- Query mode maps `@atlas.*` captures to nodes/edges and rejects invalid queries
- Plugin mode validates version/path/hash, enforces timeout and size caps, and
      handles malformed output without panic
- Profile configs reject generic-only fields
- All Section 4 limitations except the explicitly excluded Rust `dylib` path
      have a checked implementation item
- No Rust `dylib` loader or stable-ABI shim is added
- Invalid/missing grammar fails with a clear actionable error, not a panic
- `atlas update` refuses to run against a graph built with different parser
      config; full build recovers
- Watch mode honors `[parsers.external]`
- `atlas doctor` warns about unconfigured offloaded extensions
- README, parser README, CHANGELOG, and threat-model docs updated
- `cargo fmt`, `cargo clippy`, and workspace tests pass
