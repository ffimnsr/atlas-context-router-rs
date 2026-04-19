use std::path::Path;

use crate::lang::{
    go::GoParser,
    javascript::{JsParser, TsParser},
    python::PythonParser,
    rust::RustParser,
};
use crate::traits::{LangParser, ParseContext};
use atlas_core::ParsedFile;

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

    /// Parse a file.  Returns `None` when no registered handler supports the
    /// file's extension — callers should skip those files.
    pub fn parse(&self, rel_path: &str, file_hash: &str, source: &[u8]) -> Option<ParsedFile> {
        let handler = self.handler_for(rel_path)?;
        let ctx = ParseContext {
            rel_path,
            file_hash,
            source,
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

    #[test]
    fn registry_supports_rust_and_go() {
        let reg = ParserRegistry::with_defaults();
        assert!(reg.supports("src/main.rs"));
        assert!(reg.supports("cmd/main.go"));
        assert!(reg.supports("index.py"));
        assert!(reg.supports("app.js"));
        assert!(reg.supports("app.ts"));
        assert!(reg.supports("app.tsx"));
        assert!(!reg.supports("config.yaml"));
    }
}
