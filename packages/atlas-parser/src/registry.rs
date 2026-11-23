use std::borrow::Cow;
use std::path::Path;

use anyhow::{Context, Result};

use crate::external::{ExternalLangParser, ExternalParserConfig};
use crate::lang::{
    c::CParser,
    css::CssParser,
    go::GoParser,
    html::HtmlParser,
    javascript::{JsParser, TsParser},
    json::JsonParser,
    markdown::MarkdownParser,
    python::PythonParser,
    rust::RustParser,
    toml::TomlParser,
};
use crate::traits::{LangParser, ParseContext};
use atlas_core::ParsedFile;
use atlas_repo::DEFAULT_MAX_FILE_BYTES;

/// Central registry that resolves a [`LangParser`] for a given file path.
pub struct ParserRegistry {
    handlers: Vec<Box<dyn LangParser>>,
}

impl ParserRegistry {
    /// Create a registry pre-populated with every built-in language handler.
    pub fn with_defaults() -> Self {
        let mut r = Self {
            handlers: Vec::new(),
        };
        r.register(Box::new(RustParser));
        r.register(Box::new(GoParser));
        r.register(Box::new(PythonParser));
        r.register(Box::new(JsParser));
        r.register(Box::new(TsParser));
        r.register(Box::new(JsonParser));
        r.register(Box::new(TomlParser));
        r.register(Box::new(HtmlParser));
        r.register(Box::new(CssParser));
        r.register(Box::new(MarkdownParser));
        r.register(Box::new(CParser));
        r
    }

    /// Register a custom language handler.
    pub fn register(&mut self, handler: Box<dyn LangParser>) {
        self.handlers.push(handler);
    }

    /// Returns the names of all registered languages.
    pub fn supported_languages(&self) -> Vec<Cow<'static, str>> {
        self.handlers.iter().map(|h| h.language_name()).collect()
    }

    /// Register external config-driven parsers on top of the built-ins.
    ///
    /// External grammars are loaded eagerly so misconfiguration surfaces at
    /// registry construction (build/update start) instead of mid-scan.
    /// Built-in handlers keep precedence for overlapping extensions because
    /// they are registered first.
    pub fn register_externals(&mut self, configs: &[ExternalParserConfig]) -> Result<()> {
        for config in configs {
            let parser = ExternalLangParser::new(config).with_context(|| {
                format!("cannot build external parser '{}'", config.language_name)
            })?;
            self.register(Box::new(parser));
        }
        Ok(())
    }

    /// Parse a file, optionally supplying a previous tree-sitter tree to
    /// enable incremental re-parsing.  Returns `None` when no registered
    /// handler supports the file's extension — callers should skip those
    /// files. Parser failures are logged and treated as no output; use
    /// [`Self::parse_checked`] when the caller must count parse errors.
    ///
    /// The second element of the returned tuple is the new tree-sitter tree,
    /// which can be cached and passed back as `old_tree` on the next parse.
    pub fn parse(
        &self,
        rel_path: &str,
        file_hash: &str,
        source: &[u8],
        old_tree: Option<&tree_sitter::Tree>,
    ) -> Option<(ParsedFile, Option<tree_sitter::Tree>)> {
        match self.parse_checked(rel_path, file_hash, source, old_tree) {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!(path = rel_path, error = %error, "parser failed");
                None
            }
        }
    }

    /// Parse a file, surfacing parser failures to the caller.
    ///
    /// `Ok(None)` means no handler supports the path or the file exceeds the
    /// parser byte cap. `Err` means a handler ran and failed (for example an
    /// out-of-process plugin timeout or malformed response).
    pub fn parse_checked(
        &self,
        rel_path: &str,
        file_hash: &str,
        source: &[u8],
        old_tree: Option<&tree_sitter::Tree>,
    ) -> std::result::Result<Option<(ParsedFile, Option<tree_sitter::Tree>)>, String> {
        if source.len() > DEFAULT_MAX_FILE_BYTES as usize {
            tracing::warn!(
                path = rel_path,
                size_bytes = source.len(),
                max_bytes = DEFAULT_MAX_FILE_BYTES,
                "skipping parse: file exceeds parser byte cap"
            );
            return Ok(None);
        }
        let Some(handler) = self.handler_for(rel_path) else {
            return Ok(None);
        };
        // Record before handing off to the language parser: this counts real
        // parse work and whether tree-sitter receives a cached old tree.
        atlas_metrics::record_parse_attempt(old_tree.is_some());
        let ctx = ParseContext {
            rel_path,
            file_hash,
            source,
            old_tree,
        };
        handler.parse_checked(&ctx).map(Some)
    }

    /// Returns `true` if a handler is registered for `path`.
    pub fn supports(&self, path: &str) -> bool {
        self.handler_for(path).is_some()
    }

    fn handler_for(&self, path: &str) -> Option<&dyn LangParser> {
        // Prefer longest-suffix match so `.test.ts` could beat `.ts` later.
        let file_name = Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path);
        self.handlers
            .iter()
            .find(|h| h.supports(file_name))
            .map(|h| h.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external::ExternalExtractor;
    use atlas_core::NodeKind;
    use std::collections::HashMap;

    fn grammar_crates() -> Vec<(&'static str, tree_sitter::Language)> {
        vec![
            ("bash", tree_sitter_bash::LANGUAGE.into()),
            ("c", tree_sitter_c::LANGUAGE.into()),
            ("cpp", tree_sitter_cpp::LANGUAGE.into()),
            ("csharp", tree_sitter_c_sharp::LANGUAGE.into()),
            ("css", tree_sitter_css::LANGUAGE.into()),
            ("go", tree_sitter_go::LANGUAGE.into()),
            ("html", tree_sitter_html::LANGUAGE.into()),
            ("java", tree_sitter_java::LANGUAGE.into()),
            ("javascript", tree_sitter_javascript::LANGUAGE.into()),
            ("json", tree_sitter_json::LANGUAGE.into()),
            ("markdown", tree_sitter_md::LANGUAGE.into()),
            ("php", tree_sitter_php::LANGUAGE_PHP.into()),
            ("python", tree_sitter_python::LANGUAGE.into()),
            ("ruby", tree_sitter_ruby::LANGUAGE.into()),
            ("rust", tree_sitter_rust::LANGUAGE.into()),
            ("scala", tree_sitter_scala::LANGUAGE.into()),
            (
                "typescript",
                tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            ),
            ("tsx", tree_sitter_typescript::LANGUAGE_TSX.into()),
        ]
    }

    #[test]
    fn registry_supports_rust_and_go() {
        let reg = ParserRegistry::with_defaults();
        assert!(reg.supports("src/main.rs"));
        assert!(reg.supports("cmd/main.go"));
        assert!(reg.supports("index.py"));
        assert!(reg.supports("app.js"));
        assert!(reg.supports("app.ts"));
        assert!(reg.supports("app.tsx"));
        assert!(reg.supports("config.json"));
        assert!(reg.supports("Cargo.toml"));
        assert!(reg.supports("index.html"));
        assert!(reg.supports("styles.css"));
        assert!(reg.supports("README.md"));
        assert!(reg.supports("src/native.c"));
        assert!(!reg.supports("script.sh"));
        assert!(!reg.supports("src/Main.java"));
        assert!(!reg.supports("src/App.cs"));
        assert!(!reg.supports("src/index.php"));
        assert!(!reg.supports("src/native.cpp"));
        assert!(!reg.supports("src/Main.scala"));
        assert!(!reg.supports("lib/app.rb"));
        assert!(!reg.supports("config.yaml"));
    }

    #[test]
    fn default_registry_rejects_offloaded_languages() {
        let reg = ParserRegistry::with_defaults();
        for path in [
            "scripts/deploy.sh",
            "scripts/deploy.bash",
            "src/Main.java",
            "src/App.cs",
            "src/index.php",
            "src/native.cpp",
            "src/Main.scala",
            "lib/app.rb",
        ] {
            assert!(!reg.supports(path), "default registry must reject {path}");
        }
    }

    fn offloaded_languages() -> Vec<(ExternalExtractor, tree_sitter::Language)> {
        vec![
            (ExternalExtractor::Bash, tree_sitter_bash::LANGUAGE.into()),
            (
                ExternalExtractor::CSharp,
                tree_sitter_c_sharp::LANGUAGE.into(),
            ),
            (ExternalExtractor::Cpp, tree_sitter_cpp::LANGUAGE.into()),
            (ExternalExtractor::Java, tree_sitter_java::LANGUAGE.into()),
            (ExternalExtractor::Php, tree_sitter_php::LANGUAGE_PHP.into()),
            (ExternalExtractor::Ruby, tree_sitter_ruby::LANGUAGE.into()),
            (ExternalExtractor::Scala, tree_sitter_scala::LANGUAGE.into()),
        ]
    }

    #[test]
    fn profile_registration_restores_offloaded_languages() {
        let mut reg = ParserRegistry::with_defaults();
        for (extractor, language) in offloaded_languages() {
            let config = ExternalParserConfig {
                extractor: Some(extractor),
                grammar_dir: Some("unused-for-preloaded-language".to_owned()),
                ..Default::default()
            };
            reg.register(Box::new(
                ExternalLangParser::from_language(&config, language)
                    .unwrap_or_else(|err| panic!("cannot build {extractor:?} profile: {err}")),
            ));
        }

        for path in [
            "scripts/deploy.sh",
            "scripts/deploy.bash",
            "src/Main.java",
            "src/App.cs",
            "src/index.php",
            "src/native.cpp",
            "src/Main.scala",
            "lib/app.rb",
        ] {
            assert!(
                reg.supports(path),
                "profile registration must restore {path}"
            );
        }
    }

    #[test]
    fn grammar_crates_match_runtime_abi() {
        for (name, language) in grammar_crates() {
            let abi = language.abi_version();
            assert!(
                (tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=tree_sitter::LANGUAGE_VERSION)
                    .contains(&abi),
                "{name} grammar ABI {abi} outside supported range {}..={}",
                tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION,
                tree_sitter::LANGUAGE_VERSION
            );

            let mut parser = tree_sitter::Parser::new();
            parser
                .set_language(&language)
                .unwrap_or_else(|err| panic!("failed to load {name} grammar: {err}"));
        }
    }

    fn assert_parsed_file_invariants(parsed: &ParsedFile, rel_path: &str, source: &[u8]) {
        assert_eq!(parsed.path, rel_path);

        if let Some(size) = parsed.size {
            assert_eq!(size, source.len() as i64);
        }

        let file_node_count = parsed
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::File)
            .count();
        assert_eq!(file_node_count, 1, "{rel_path} should emit one file node");

        for node in &parsed.nodes {
            assert_eq!(node.file_path, rel_path, "{rel_path} node path drifted");
            assert!(
                !node.qualified_name.is_empty(),
                "{rel_path} emitted empty qualified_name"
            );
            assert!(
                node.line_start >= 1,
                "{rel_path} line_start must be 1-based"
            );
            assert!(
                node.line_end >= node.line_start,
                "{rel_path} line_end must not precede line_start"
            );
        }

        for edge in &parsed.edges {
            assert!(
                !edge.source_qn.is_empty(),
                "{rel_path} emitted empty edge.source_qn"
            );
            assert!(
                !edge.target_qn.is_empty(),
                "{rel_path} emitted empty edge.target_qn"
            );
        }

        // Duplicate qualified names remain advisory until validity is defined
        // per language/model. Keep detector live without asserting here.
        let mut counts = HashMap::new();
        for node in &parsed.nodes {
            *counts.entry(node.qualified_name.as_str()).or_insert(0usize) += 1;
        }
        std::hint::black_box(
            counts
                .into_iter()
                .filter(|(_, count)| *count > 1)
                .map(|(qualified_name, count)| (qualified_name.to_owned(), count))
                .collect::<Vec<_>>(),
        );
    }

    #[test]
    fn built_in_handlers_preserve_parsed_file_invariants() {
        let reg = ParserRegistry::with_defaults();
        let samples: [(&str, &[u8]); 11] = [
            ("src/main.rs", b"fn main() {}\n"),
            ("src/main.go", b"package main\nfunc main() {}\n"),
            ("src/main.py", b"def main():\n    return 1\n"),
            ("src/main.js", b"function main() { return 1; }\n"),
            (
                "src/main.ts",
                b"export function main(value: number): number { return value; }\n",
            ),
            ("src/main.json", b"{\"enabled\":true}\n"),
            ("src/main.toml", b"enabled = true\n"),
            (
                "src/main.html",
                b"<html><body><main>ok</main></body></html>\n",
            ),
            ("src/main.css", b"body { color: red; }\n"),
            ("src/main.md", b"# Heading\nbody\n"),
            ("src/main.c", b"int main(void) { return 0; }\n"),
        ];

        for (rel_path, source) in samples {
            let file_hash = format!("hash-{rel_path}");
            let (parsed, _tree) = reg
                .parse(rel_path, &file_hash, source, None)
                .unwrap_or_else(|| panic!("registry should support {rel_path}"));
            assert_parsed_file_invariants(&parsed, rel_path, source);
        }
    }

    #[test]
    fn parse_records_tree_reuse_metrics() {
        let reg = ParserRegistry::with_defaults();
        let before = atlas_metrics::snapshot();

        let (_parsed, tree) = reg
            .parse("src/main.rs", "hash1", b"fn main() {}", None)
            .expect("fresh parse");
        let tree = tree.expect("rust parser must return a tree");
        let (_parsed2, _tree2) = reg
            .parse("src/main.rs", "hash2", b"fn main() { }", Some(&tree))
            .expect("incremental parse");

        let after = atlas_metrics::snapshot();
        assert!(
            after.parser_parses_total >= before.parser_parses_total + 2,
            "both parse calls must be counted"
        );
        assert!(
            after.parser_tree_reuses_total > before.parser_tree_reuses_total,
            "incremental parse with old tree must count as a reuse"
        );
        assert!(after.parser_cache_reuse_ratio > 0.0);
    }

    #[test]
    fn registry_skips_files_over_parse_byte_cap() {
        let reg = ParserRegistry::with_defaults();
        let oversized = vec![b'a'; DEFAULT_MAX_FILE_BYTES as usize + 1];

        assert!(reg.parse("src/main.rs", "hash", &oversized, None).is_none());
    }
}
