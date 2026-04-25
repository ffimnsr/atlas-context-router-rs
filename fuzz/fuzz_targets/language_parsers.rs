#![no_main]

use atlas_fuzz::{ParserCase, SupportedPathKind, hash_bytes};
use atlas_parser::lang::{
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
use atlas_parser::{LangParser, ParseContext};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|case: ParserCase| {
    let first = parse_once(case.path_kind, &case.source, None);
    if !case.reuse_old_tree {
        return;
    }

    let Some(old_tree) = first.as_ref() else {
        return;
    };

    let _ = parse_once(case.path_kind, &case.next_source, Some(old_tree));
});

fn parse_once(
    path_kind: SupportedPathKind,
    source: &[u8],
    old_tree: Option<&tree_sitter::Tree>,
) -> Option<tree_sitter::Tree> {
    let file_hash = hash_bytes(source);
    let ctx = ParseContext {
        rel_path: path_kind.rel_path(),
        file_hash: &file_hash,
        source,
        old_tree,
    };

    match path_kind {
        SupportedPathKind::Rust => RustParser.parse(&ctx),
        SupportedPathKind::Go => GoParser.parse(&ctx),
        SupportedPathKind::Python => PythonParser.parse(&ctx),
        SupportedPathKind::JavaScript => JsParser.parse(&ctx),
        SupportedPathKind::TypeScript => TsParser.parse(&ctx),
        SupportedPathKind::Json => JsonParser.parse(&ctx),
        SupportedPathKind::Toml => TomlParser.parse(&ctx),
        SupportedPathKind::Html => HtmlParser.parse(&ctx),
        SupportedPathKind::Css => CssParser.parse(&ctx),
        SupportedPathKind::Bash => BashParser.parse(&ctx),
        SupportedPathKind::Markdown => MarkdownParser.parse(&ctx),
        SupportedPathKind::Java => JavaParser.parse(&ctx),
        SupportedPathKind::CSharp => CSharpParser.parse(&ctx),
        SupportedPathKind::Php => PhpParser.parse(&ctx),
        SupportedPathKind::C => CParser.parse(&ctx),
        SupportedPathKind::Cpp => CppParser.parse(&ctx),
        SupportedPathKind::Scala => ScalaParser.parse(&ctx),
        SupportedPathKind::Ruby => RubyParser.parse(&ctx),
    }
    .1
}
