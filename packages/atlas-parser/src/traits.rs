use atlas_core::ParsedFile;

/// Input provided to a language parser.
pub struct ParseContext<'a> {
    /// Repository-relative path (e.g. `src/main.rs`).
    pub rel_path: &'a str,
    /// SHA-256 hex digest of the file contents.
    pub file_hash: &'a str,
    /// Raw source bytes.
    pub source: &'a [u8],
}

/// Trait implemented by each per-language parser.
pub trait LangParser: Send + Sync {
    /// Language name returned in graph nodes (e.g. `"rust"`, `"go"`).
    fn language_name(&self) -> &'static str;

    /// Returns `true` if this handler supports the given file extension.
    fn supports(&self, path: &str) -> bool;

    /// Parse the source and return a fully-populated [`ParsedFile`].
    ///
    /// On partial tree-sitter parse failure the function should still return a
    /// best-effort result (File node + any symbols found before the error node).
    fn parse(&self, ctx: &ParseContext<'_>) -> ParsedFile;
}
