use atlas_core::ParsedFile;

/// Input provided to a language parser.
pub struct ParseContext<'a> {
    /// Repository-relative path (e.g. `src/main.rs`).
    pub rel_path: &'a str,
    /// SHA-256 hex digest of the file contents.
    pub file_hash: &'a str,
    /// Raw source bytes.
    pub source: &'a [u8],
    /// Previous tree-sitter parse tree for the same file.  When provided, the
    /// underlying parser passes it to `tree_sitter::Parser::parse` so unchanged
    /// subtrees are reused rather than re-parsed from scratch.  `None` forces a
    /// full parse.
    pub old_tree: Option<&'a tree_sitter::Tree>,
}

/// Trait implemented by each per-language parser.
pub trait LangParser: Send + Sync {
    /// Language name returned in graph nodes (e.g. `"rust"`, `"go"`).
    fn language_name(&self) -> &'static str;

    /// Returns `true` if this handler supports the given file extension.
    fn supports(&self, path: &str) -> bool;

    /// Parse the source and return a fully-populated [`ParsedFile`] together
    /// with the resulting tree-sitter [`tree_sitter::Tree`].
    ///
    /// The returned tree may be cached by the caller and passed back via
    /// [`ParseContext::old_tree`] on the next parse of the same file, enabling
    /// incremental re-parsing.
    ///
    /// On partial tree-sitter parse failure the function should still return a
    /// best-effort result (File node + any symbols found before the error node).
    fn parse(&self, ctx: &ParseContext<'_>) -> (ParsedFile, Option<tree_sitter::Tree>);
}
