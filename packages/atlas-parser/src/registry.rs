use std::path::Path;

use crate::lang::{
    bash::BashParser,
    c::CParser,
    cpp::CppParser,
    csharp::CSharpParser,
    css::CssParser,
    go::GoParser,
    html::HtmlParser,
    java::JavaParser,
    javascript::{JsParser, TsParser},
    json::JsonParser,
    markdown::MarkdownParser,
    php::PhpParser,
    python::PythonParser,
    ruby::RubyParser,
    rust::RustParser,
    scala::ScalaParser,
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
        r.register(Box::new(BashParser));
        r.register(Box::new(MarkdownParser));
        r.register(Box::new(JavaParser));
        r.register(Box::new(CSharpParser));
        r.register(Box::new(PhpParser));
        r.register(Box::new(CParser));
        r.register(Box::new(CppParser));
        r.register(Box::new(ScalaParser));
        r.register(Box::new(RubyParser));
        r
    }

    /// Register a custom language handler.
    pub fn register(&mut self, handler: Box<dyn LangParser>) {
        self.handlers.push(handler);
    }

    /// Returns the names of all registered languages.
    pub fn supported_languages(&self) -> Vec<&'static str> {
        self.handlers.iter().map(|h| h.language_name()).collect()
    }

    /// Parse a file, optionally supplying a previous tree-sitter tree to
    /// enable incremental re-parsing.  Returns `None` when no registered
    /// handler supports the file's extension — callers should skip those
    /// files.
    ///
    /// The second element of the returned tuple is the new tree-sitter tree,
    /// which the caller may cache and pass back as `old_tree` on the next
    /// parse of the same file.
    pub fn parse(
        &self,
        rel_path: &str,
        file_hash: &str,
        source: &[u8],
        old_tree: Option<&tree_sitter::Tree>,
    ) -> Option<(ParsedFile, Option<tree_sitter::Tree>)> {
        if source.len() > DEFAULT_MAX_FILE_BYTES as usize {
            tracing::warn!(
                path = rel_path,
                size_bytes = source.len(),
                max_bytes = DEFAULT_MAX_FILE_BYTES,
                "skipping parse: file exceeds parser byte cap"
            );
            return None;
        }
        let handler = self.handler_for(rel_path)?;
        let ctx = ParseContext {
            rel_path,
            file_hash,
            source,
            old_tree,
        };
        Some(handler.parse(&ctx))
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

    fn bundled_grammars() -> Vec<(&'static str, tree_sitter::Language)> {
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
        assert!(reg.supports("script.sh"));
        assert!(reg.supports("README.md"));
        assert!(reg.supports("src/Main.java"));
        assert!(reg.supports("src/App.cs"));
        assert!(reg.supports("src/index.php"));
        assert!(reg.supports("src/native.c"));
        assert!(reg.supports("src/native.cpp"));
        assert!(reg.supports("src/Main.scala"));
        assert!(reg.supports("lib/app.rb"));
        assert!(!reg.supports("config.yaml"));
    }

    #[test]
    fn bundled_grammars_match_runtime_abi() {
        for (name, language) in bundled_grammars() {
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

    #[test]
    fn registry_skips_files_over_parse_byte_cap() {
        let reg = ParserRegistry::with_defaults();
        let oversized = vec![b'a'; DEFAULT_MAX_FILE_BYTES as usize + 1];

        assert!(reg.parse("src/main.rs", "hash", &oversized, None).is_none());
    }
}
