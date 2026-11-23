use super::visit::parse_source;
use super::*;

impl LangParser for JsParser {
    fn language_name(&self) -> std::borrow::Cow<'static, str> {
        "javascript".into()
    }

    fn supports(&self, path: &str) -> bool {
        path.ends_with(".js")
            || path.ends_with(".jsx")
            || path.ends_with(".mjs")
            || path.ends_with(".cjs")
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> (ParsedFile, Option<tree_sitter::Tree>) {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .expect("tree-sitter-javascript grammar failed to load");
        parse_source(
            &mut parser,
            ctx,
            "javascript",
            JsTsQueryConfig {
                language_name: "javascript",
                language: tree_sitter_javascript::LANGUAGE.into(),
                query_text: JAVASCRIPT_QUERY,
            },
        )
    }
}

impl LangParser for TsParser {
    fn language_name(&self) -> std::borrow::Cow<'static, str> {
        "typescript".into()
    }

    fn supports(&self, path: &str) -> bool {
        // Skip declaration files (.d.ts) — they carry no runtime symbols.
        if path.ends_with(".d.ts") {
            return false;
        }
        path.ends_with(".ts") || path.ends_with(".tsx")
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> (ParsedFile, Option<tree_sitter::Tree>) {
        let mut parser = tree_sitter::Parser::new();
        let lang = if ctx.rel_path.ends_with(".tsx") {
            tree_sitter_typescript::LANGUAGE_TSX.into()
        } else {
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
        };
        parser
            .set_language(&lang)
            .expect("tree-sitter-typescript grammar failed to load");
        parse_source(
            &mut parser,
            ctx,
            "typescript",
            JsTsQueryConfig {
                language_name: "typescript",
                language: lang,
                query_text: TYPESCRIPT_QUERY,
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Core parsing (shared between JS and TS)
// ---------------------------------------------------------------------------
