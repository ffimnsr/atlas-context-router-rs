//! Config-driven external language support backed by `tree-sitter-loader`.
//!
//! Users register extra languages in `.atlas/config.toml` under
//! `[parsers.external]` by pointing at a tree-sitter grammar checkout (compiled
//! on first use and cached) or a prebuilt grammar shared library. Known
//! languages can use an extractor profile; other languages describe which
//! tree-sitter node kinds map to graph symbols and calls. Built-in handlers
//! always win for overlapping extensions; external parsers are appended after
//! them in the registry.
//!
//! Relative `grammar_dir` / `lib_path` / `grammar_lib_dir` paths are resolved
//! against the atlas dir (`.atlas/`) by `Config::load` before the registry is
//! built.

mod emit;
mod generic;
mod loader;
mod plugin;
mod profile;
mod protocol;
mod query;

use std::borrow::Cow;

use anyhow::{Result, ensure};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::traits::{LangParser, ParseContext};

use self::generic::GenericExternalParser;
use self::loader::load_external_language;
use self::plugin::PluginExternalParser;
use self::protocol::{DEFAULT_MAX_RESPONSE_BYTES, DEFAULT_TIMEOUT_MS, PLUGIN_PROTOCOL};
use self::query::QueryExternalParser;

/// Rule mapping one tree-sitter node kind to a graph symbol node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalSymbolRule {
    /// Tree-sitter node kind treated as a symbol (e.g. `"function_item"`).
    pub tree_kind: String,
    /// Graph node kind emitted for this tree kind.
    pub node_kind: atlas_core::NodeKind,
    /// Tree-sitter field holding the symbol name (default `"name"`).
    #[serde(default = "default_name_field")]
    pub name_field: String,
    /// Tree-sitter field holding the parameter list text.
    #[serde(default)]
    pub params_field: Option<String>,
    /// Tree-sitter field holding the return type text.
    #[serde(default)]
    pub return_type_field: Option<String>,
    /// Direct child node kinds whose text contributes to `modifiers`.
    #[serde(default)]
    pub modifier_node_kinds: Vec<String>,
}

/// Rule mapping one tree-sitter node kind to an import node.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExternalImportRule {
    /// Tree-sitter node kind treated as an import.
    pub tree_kind: String,
    /// Field holding the imported name; falls back to the last named child.
    #[serde(default)]
    pub name_field: Option<String>,
}

/// Rule mapping one tree-sitter node kind to an attribute node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalAttributeRule {
    /// Tree-sitter node kind treated as an attribute.
    pub tree_kind: String,
    /// Field holding the attribute name (default `"name"`).
    #[serde(default = "default_name_field")]
    pub name_field: String,
    /// Attachment target for the attribute node.
    #[serde(default)]
    pub attach_to: ExternalAttributeTarget,
}

/// Attachment target for a generic external attribute rule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalAttributeTarget {
    /// Attach the attribute to the current traversal scope.
    #[default]
    Scope,
    /// Buffer the attribute and attach it to the following sibling symbol.
    NextSymbol,
}

/// Qualified-name style for generic external symbols.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalQnStyle {
    /// Nest symbols under the nearest enclosing symbol.
    #[default]
    Nested,
    /// Emit file-scoped `file::kind::Owner.name` qualified names.
    FlatOwner,
}

fn default_name_field() -> String {
    "name".to_owned()
}

/// How an external parser derives graph symbols.
///
/// Modes are mutually exclusive; [`ExternalParserConfig::validate`] enforces
/// that exactly one mode is configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalExtractionMode {
    /// Built-in extractor profile (canonical built-in parity).
    Profile,
    /// Node-kind symbol/import/attribute rules.
    Rules,
    /// Tree-sitter SCM query with `@atlas.*` captures.
    Query,
    /// Out-of-process plugin speaking the JSON/stdio protocol.
    Plugin,
}

impl ExternalExtractionMode {
    /// Stable mode label used in diagnostics and the parser fingerprint.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Profile => "profile",
            Self::Rules => "rules",
            Self::Query => "query",
            Self::Plugin => "plugin",
        }
    }
}

/// Built-in extractor profiles for languages offloaded from bundled handlers.
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

impl ExternalExtractor {
    /// Canonical language name stored on graph nodes.
    pub fn language_name(self) -> &'static str {
        match self {
            Self::Php => "php",
            Self::Cpp => "cpp",
            Self::CSharp => "csharp",
            Self::Java => "java",
            Self::Scala => "scala",
            Self::Ruby => "ruby",
            Self::Bash => "bash",
        }
    }

    /// Default file extensions registered for this profile.
    pub fn default_extensions(self) -> &'static [&'static str] {
        match self {
            Self::Php => &["php"],
            Self::Cpp => &["cpp", "cc", "cxx"],
            Self::CSharp => &["cs"],
            Self::Java => &["java"],
            Self::Scala => &["scala", "sc"],
            Self::Ruby => &["rb"],
            Self::Bash => &["sh", "bash"],
        }
    }

    /// Default exported symbol for a prebuilt grammar library.
    pub fn default_lib_function(self) -> &'static str {
        match self {
            Self::Php => "tree_sitter_php",
            Self::Cpp => "tree_sitter_cpp",
            Self::CSharp => "tree_sitter_c_sharp",
            Self::Java => "tree_sitter_java",
            Self::Scala => "tree_sitter_scala",
            Self::Ruby => "tree_sitter_ruby",
            Self::Bash => "tree_sitter_bash",
        }
    }
}

/// One external language registration (serde-compatible with
/// `[parsers.external]` in `.atlas/config.toml`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExternalParserConfig {
    /// Built-in extractor profile, when this entry offloads a known language.
    #[serde(default)]
    pub extractor: Option<ExternalExtractor>,
    /// Language name stored on graph nodes (e.g. `"zig"`).
    #[serde(default)]
    pub language_name: String,
    /// File extensions registered for this parser (no leading dot).
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Directory containing the grammar checkout (must hold `src/parser.c`
    /// and `src/grammar.json`). The grammar is compiled with a C compiler on
    /// first use and cached in `grammar_lib_dir`.
    #[serde(default)]
    pub grammar_dir: Option<String>,
    /// Path to a prebuilt grammar shared library (e.g.
    /// `/opt/grammars/libtree-sitter-zig.so`). Mutually exclusive with
    /// `grammar_dir`.
    #[serde(default)]
    pub lib_path: Option<String>,
    /// Exported symbol inside `lib_path`. Defaults to
    /// `tree_sitter_<language_name>` (hyphens replaced with underscores) for
    /// generic parsers and to the profile default for extractor entries.
    #[serde(default)]
    pub lib_function: Option<String>,
    /// Directory where compiled grammars are cached. Defaults to the
    /// tree-sitter user cache (overridable with `TREE_SITTER_LIBDIR`).
    #[serde(default)]
    pub grammar_lib_dir: Option<String>,
    /// Path to a tree-sitter SCM query file. Relative paths resolve from
    /// `.atlas/`, matching grammar path rules. Mutually exclusive with `query`.
    #[serde(default)]
    pub query_path: Option<String>,
    /// Inline tree-sitter SCM query using `@atlas.*` captures. Mutually
    /// exclusive with `query_path`.
    #[serde(default)]
    pub query: Option<String>,
    /// Plugin executable for out-of-process extraction. Relative paths resolve
    /// from `.atlas/` at config load.
    #[serde(default)]
    pub command: Option<String>,
    /// Arguments passed to `command` as a literal argv vector (no shell).
    #[serde(default)]
    pub args: Vec<String>,
    /// Plugin working directory. Defaults to the repo root (parent of `.atlas/`).
    #[serde(default)]
    pub working_dir: Option<String>,
    /// Plugin protocol identifier. Defaults to `atlas-parser.v1`.
    #[serde(default)]
    pub protocol: Option<String>,
    /// Per-file plugin timeout in milliseconds. Defaults to 30000.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Maximum accepted plugin response size in bytes. Defaults to 16 MiB.
    #[serde(default)]
    pub max_response_bytes: Option<u64>,
    /// Symbol rules: which tree node kinds become graph symbols. Required for
    /// generic parsers and rejected for extractor profiles.
    #[serde(default)]
    pub symbols: Vec<ExternalSymbolRule>,
    /// Tree node kinds treated as call sites. Edges connect the enclosing
    /// symbol to a same-file symbol whose name matches the callee text.
    #[serde(default)]
    pub call_node_kinds: Vec<String>,
    /// Field of a call node that names the callee (e.g. `"function"`).
    /// Defaults to the last named child.
    #[serde(default)]
    pub call_target_field: Option<String>,
    /// Field of a call node holding the receiver (e.g. `"object"`).
    #[serde(default)]
    pub call_receiver_field: Option<String>,
    /// Import rules for generic external parsers.
    #[serde(default)]
    pub imports: Vec<ExternalImportRule>,
    /// Attribute rules for generic external parsers.
    #[serde(default)]
    pub attributes: Vec<ExternalAttributeRule>,
    /// Qualified-name style for generic external symbols.
    #[serde(default)]
    pub qn_style: ExternalQnStyle,
    /// Node kinds that act as owners for `flat_owner` qualified names.
    #[serde(default)]
    pub owner_node_kinds: Vec<atlas_core::NodeKind>,
    /// Regexes matched against symbol names to mark tests.
    #[serde(default)]
    pub test_name_patterns: Vec<String>,
    /// Regexes matched against repo-relative paths to mark tests.
    #[serde(default)]
    pub test_path_patterns: Vec<String>,
}

impl ExternalParserConfig {
    /// Fill profile and plugin defaults without overwriting explicit values.
    pub fn apply_defaults(&mut self) {
        if let Some(extractor) = self.extractor {
            if self.language_name.trim().is_empty() {
                self.language_name = extractor.language_name().to_owned();
            }
            if self.extensions.is_empty() {
                self.extensions = extractor
                    .default_extensions()
                    .iter()
                    .map(|extension| (*extension).to_owned())
                    .collect();
            }
            if self
                .lib_function
                .as_deref()
                .is_none_or(|function| function.trim().is_empty())
            {
                self.lib_function = Some(extractor.default_lib_function().to_owned());
            }
        }

        if self.extraction_mode() == Some(ExternalExtractionMode::Plugin) {
            if self
                .protocol
                .as_deref()
                .is_none_or(|protocol| protocol.trim().is_empty())
            {
                self.protocol = Some(PLUGIN_PROTOCOL.to_owned());
            }
            if self.timeout_ms.is_none() {
                self.timeout_ms = Some(DEFAULT_TIMEOUT_MS);
            }
            if self.max_response_bytes.is_none() {
                self.max_response_bytes = Some(DEFAULT_MAX_RESPONSE_BYTES);
            }
        }
    }

    /// Extraction mode selected by the configured fields, or `None` when no
    /// mode is configured.
    pub fn extraction_mode(&self) -> Option<ExternalExtractionMode> {
        if self.extractor.is_some() {
            Some(ExternalExtractionMode::Profile)
        } else if self.command.is_some() {
            Some(ExternalExtractionMode::Plugin)
        } else if !self.symbols.is_empty() {
            Some(ExternalExtractionMode::Rules)
        } else if self.query_path.is_some() || self.query.is_some() {
            Some(ExternalExtractionMode::Query)
        } else {
            None
        }
    }

    /// Validate one external parser entry after profile defaults are applied.
    pub fn validate(&self) -> Result<()> {
        let label = if self.language_name.is_empty() {
            self.extractor
                .map(ExternalExtractor::language_name)
                .unwrap_or("external")
        } else {
            self.language_name.as_str()
        };

        let mode = self.extraction_mode();
        if mode == Some(ExternalExtractionMode::Plugin) {
            ensure!(
                self.grammar_dir.is_none() && self.lib_path.is_none(),
                "parsers.external plugin '{label}' must not declare grammar_dir or lib_path"
            );
        } else {
            ensure!(
                self.grammar_dir.is_some() != self.lib_path.is_some(),
                "parsers.external '{label}' must configure exactly one of grammar_dir or lib_path"
            );
        }
        ensure!(
            !(self.query_path.is_some() && self.query.is_some()),
            "parsers.external '{label}' must configure exactly one of query_path or query"
        );
        if let Some(query) = &self.query {
            ensure!(
                !query.trim().is_empty(),
                "parsers.external '{label}' has an empty inline query"
            );
        }
        if let Some(path) = &self.query_path {
            ensure!(
                !path.trim().is_empty(),
                "parsers.external '{label}' has an empty query_path"
            );
        }

        match mode {
            Some(ExternalExtractionMode::Profile) => {
                let extractor = self.extractor.expect("profile mode requires an extractor");
                ensure!(
                    self.language_name == extractor.language_name(),
                    "parsers.external extractor '{}' requires canonical language_name '{}'",
                    extractor.language_name(),
                    extractor.language_name()
                );
                ensure!(
                    !self.extensions.is_empty(),
                    "parsers.external profile '{label}' must declare at least one extension"
                );
                ensure!(
                    self.symbols.is_empty(),
                    "parsers.external profile '{label}' must not declare symbol rules"
                );
                ensure!(
                    self.query_path.is_none() && self.query.is_none(),
                    "parsers.external profile '{label}' must not declare query fields"
                );
                ensure!(
                    self.command.is_none()
                        && self.args.is_empty()
                        && self.working_dir.is_none()
                        && self.protocol.is_none()
                        && self.timeout_ms.is_none()
                        && self.max_response_bytes.is_none(),
                    "parsers.external profile '{label}' must not declare plugin fields"
                );
                ensure!(
                    self.call_node_kinds.is_empty()
                        && self.call_target_field.is_none()
                        && self.call_receiver_field.is_none()
                        && self.imports.is_empty()
                        && self.attributes.is_empty()
                        && self.qn_style == ExternalQnStyle::Nested
                        && self.owner_node_kinds.is_empty()
                        && self.test_name_patterns.is_empty()
                        && self.test_path_patterns.is_empty(),
                    "parsers.external profile '{label}' must not declare generic extraction fields"
                );
            }
            Some(ExternalExtractionMode::Rules) => {
                ensure!(
                    self.query_path.is_none() && self.query.is_none(),
                    "parsers.external '{label}' rule mode must not declare query_path or query"
                );
                ensure!(
                    self.command.is_none()
                        && self.args.is_empty()
                        && self.working_dir.is_none()
                        && self.protocol.is_none()
                        && self.timeout_ms.is_none()
                        && self.max_response_bytes.is_none(),
                    "parsers.external '{label}' rule mode must not declare plugin fields"
                );
                ensure!(
                    !self.language_name.is_empty(),
                    "parsers.external entry must declare a language_name"
                );
                ensure!(
                    !self.extensions.is_empty(),
                    "parsers.external '{label}' must declare at least one extension"
                );
            }
            Some(ExternalExtractionMode::Query) => {
                ensure!(
                    !self.language_name.is_empty(),
                    "parsers.external entry must declare a language_name"
                );
                ensure!(
                    !self.extensions.is_empty(),
                    "parsers.external '{label}' must declare at least one extension"
                );
                ensure!(
                    self.symbols.is_empty()
                        && self.imports.is_empty()
                        && self.attributes.is_empty()
                        && self.call_node_kinds.is_empty()
                        && self.call_target_field.is_none()
                        && self.call_receiver_field.is_none(),
                    "parsers.external query mode '{label}' must not declare symbol, import, attribute, or call rules"
                );
                ensure!(
                    self.command.is_none()
                        && self.args.is_empty()
                        && self.working_dir.is_none()
                        && self.protocol.is_none()
                        && self.timeout_ms.is_none()
                        && self.max_response_bytes.is_none(),
                    "parsers.external query mode '{label}' must not declare plugin fields"
                );
            }
            Some(ExternalExtractionMode::Plugin) => {
                ensure!(
                    !self.language_name.is_empty(),
                    "parsers.external entry must declare a language_name"
                );
                ensure!(
                    !self.extensions.is_empty(),
                    "parsers.external plugin '{label}' must declare at least one extension"
                );
                ensure!(
                    self.command
                        .as_deref()
                        .is_some_and(|command| !command.trim().is_empty()),
                    "parsers.external plugin '{label}' needs a non-empty command"
                );
                ensure!(
                    self.query_path.is_none()
                        && self.query.is_none()
                        && self.symbols.is_empty()
                        && self.imports.is_empty()
                        && self.attributes.is_empty()
                        && self.call_node_kinds.is_empty()
                        && self.call_target_field.is_none()
                        && self.call_receiver_field.is_none()
                        && self.qn_style == ExternalQnStyle::Nested
                        && self.owner_node_kinds.is_empty()
                        && self.test_name_patterns.is_empty()
                        && self.test_path_patterns.is_empty(),
                    "parsers.external plugin '{label}' must not declare extraction fields"
                );
                let protocol = self.protocol.as_deref().unwrap_or(PLUGIN_PROTOCOL);
                ensure!(
                    protocol == PLUGIN_PROTOCOL,
                    "parsers.external plugin '{label}' uses unsupported protocol '{protocol}' (expected '{PLUGIN_PROTOCOL}')"
                );
                ensure!(
                    self.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS) > 0,
                    "parsers.external plugin '{label}' timeout_ms must be greater than zero"
                );
                ensure!(
                    self.max_response_bytes
                        .unwrap_or(DEFAULT_MAX_RESPONSE_BYTES)
                        > 0,
                    "parsers.external plugin '{label}' max_response_bytes must be greater than zero"
                );
            }
            None => anyhow::bail!(
                "parsers.external '{label}' must declare an extractor, symbol rules, query, or command"
            ),
        }

        if let Some(function) = &self.lib_function {
            ensure!(
                !function.is_empty(),
                "parsers.external '{label}' has an empty lib_function"
            );
        }

        for rule in &self.symbols {
            ensure!(
                !rule.tree_kind.is_empty() && !rule.name_field.is_empty(),
                "parsers.external '{label}' symbol rules need non-empty tree_kind and name_field"
            );
            if let Some(field) = &rule.params_field {
                ensure!(
                    !field.is_empty(),
                    "parsers.external '{label}' symbol rule has an empty params_field"
                );
            }
            if let Some(field) = &rule.return_type_field {
                ensure!(
                    !field.is_empty(),
                    "parsers.external '{label}' symbol rule has an empty return_type_field"
                );
            }
            for kind in &rule.modifier_node_kinds {
                ensure!(
                    !kind.is_empty(),
                    "parsers.external '{label}' symbol rule has an empty modifier node kind"
                );
            }
        }

        for rule in &self.imports {
            ensure!(
                !rule.tree_kind.is_empty(),
                "parsers.external '{label}' import rules need a non-empty tree_kind"
            );
            if let Some(field) = &rule.name_field {
                ensure!(
                    !field.is_empty(),
                    "parsers.external '{label}' import rule has an empty name_field"
                );
            }
        }

        for rule in &self.attributes {
            ensure!(
                !rule.tree_kind.is_empty() && !rule.name_field.is_empty(),
                "parsers.external '{label}' attribute rules need non-empty tree_kind and name_field"
            );
        }

        if self.qn_style == ExternalQnStyle::FlatOwner {
            ensure!(
                !self.owner_node_kinds.is_empty(),
                "parsers.external '{label}' with flat_owner qn_style needs owner_node_kinds"
            );
        }
        if let Some(field) = &self.call_receiver_field {
            ensure!(
                !field.is_empty(),
                "parsers.external '{label}' has an empty call_receiver_field"
            );
        }
        for kind in &self.call_node_kinds {
            ensure!(
                !kind.is_empty(),
                "parsers.external '{label}' has an empty call node kind"
            );
        }
        for pattern in self
            .test_name_patterns
            .iter()
            .chain(self.test_path_patterns.iter())
        {
            Regex::new(pattern).map_err(|error| {
                anyhow::anyhow!(
                    "parsers.external '{label}' has invalid test regex '{pattern}': {error}"
                )
            })?;
        }

        for extension in &self.extensions {
            ensure!(
                !extension.is_empty() && !extension.starts_with('.'),
                "parsers.external '{label}' has invalid extension '{extension}' (no leading dot)"
            );
        }

        Ok(())
    }
}

/// A runtime-loaded, config-driven language parser.
///
/// Wraps an extractor profile, a generic node-kind parser, a query parser, or an
/// out-of-process plugin. The wrapper enforces the configured extension set and
/// canonical language name.
pub struct ExternalLangParser {
    inner: Box<dyn LangParser>,
    language_name: Cow<'static, str>,
    extensions: Vec<String>,
}

impl ExternalLangParser {
    /// Build the configured parser, loading a grammar unless plugin mode is used.
    pub fn new(config: &ExternalParserConfig) -> Result<Self> {
        let mut config = config.clone();
        config.apply_defaults();
        config.validate()?;
        if config.extraction_mode() == Some(ExternalExtractionMode::Plugin) {
            return Self::assemble(&config, None);
        }
        let language = load_external_language(&config)?;
        Self::assemble(&config, Some(language))
    }

    /// Build a grammar-backed parser from a preloaded language, for embedding
    /// and parity tests. Plugin mode does not require a language; use
    /// [`Self::new`] for plugin configs.
    pub fn from_language(
        config: &ExternalParserConfig,
        language: tree_sitter::Language,
    ) -> Result<Self> {
        let mut config = config.clone();
        config.apply_defaults();
        config.validate()?;
        if config.extraction_mode() == Some(ExternalExtractionMode::Plugin) {
            anyhow::bail!(
                "external plugin '{}' does not use a tree-sitter grammar",
                config.language_name
            );
        }
        Self::assemble(&config, Some(language))
    }

    fn assemble(
        config: &ExternalParserConfig,
        language: Option<tree_sitter::Language>,
    ) -> Result<Self> {
        let language_name = Cow::Owned(config.language_name.clone());
        let extensions = config.extensions.clone();
        let inner: Box<dyn LangParser> = match config.extraction_mode() {
            Some(ExternalExtractionMode::Profile) => {
                let extractor = config
                    .extractor
                    .expect("validated profile mode requires an extractor");
                extractor.build(
                    language.expect("profile mode requires a loaded grammar"),
                    extensions.clone(),
                )?
            }
            Some(ExternalExtractionMode::Query) => Box::new(QueryExternalParser::new(
                config,
                language.expect("query mode requires a loaded grammar"),
            )?),
            Some(ExternalExtractionMode::Plugin) => Box::new(PluginExternalParser::new(config)?),
            Some(ExternalExtractionMode::Rules) | None => Box::new(GenericExternalParser::new(
                config,
                language.expect("rule mode requires a loaded grammar"),
            )),
        };

        Ok(Self {
            inner,
            language_name,
            extensions,
        })
    }
}

impl LangParser for ExternalLangParser {
    fn language_name(&self) -> Cow<'static, str> {
        self.language_name.clone()
    }

    fn supports(&self, path: &str) -> bool {
        supports_extension(path, &self.extensions)
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> (atlas_core::ParsedFile, Option<tree_sitter::Tree>) {
        self.inner.parse(ctx)
    }

    fn parse_checked(
        &self,
        ctx: &ParseContext<'_>,
    ) -> std::result::Result<(atlas_core::ParsedFile, Option<tree_sitter::Tree>), String> {
        self.inner.parse_checked(ctx)
    }
}

pub(crate) fn supports_extension(path: &str, extensions: &[String]) -> bool {
    let Some((_, extension)) = path.rsplit_once('.') else {
        return false;
    };
    extensions.iter().any(|candidate| candidate == extension)
}

#[cfg(test)]
mod tests;
