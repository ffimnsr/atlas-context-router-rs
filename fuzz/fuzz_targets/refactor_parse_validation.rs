#![no_main]

use arbitrary::Arbitrary;
use atlas_fuzz::SupportedPathKind;
use atlas_parser::ParserRegistry;
use atlas_refactor::validate_file_parse_for_refactor;
use libfuzzer_sys::fuzz_target;

const MAX_CONTENT_BYTES: usize = 16 * 1024;

#[derive(Arbitrary, Debug)]
enum ValidationPathKind {
    Supported(SupportedPathKind),
    Unsupported(UnsupportedPathKind),
}

#[derive(Arbitrary, Debug)]
enum UnsupportedPathKind {
    Text,
    Yaml,
    Sql,
    NoExtension,
}

#[derive(Arbitrary, Debug)]
struct RefactorValidationCase {
    path_kind: ValidationPathKind,
    content: Vec<u8>,
}

fuzz_target!(|case: RefactorValidationCase| {
    let content = bounded_content(&case.content);
    let content = String::from_utf8_lossy(&content);
    let path = case.path_kind.rel_path();

    // `SupportedPathKind` covers every grammar Atlas can parse, including
    // grammars offloaded to external parser profiles. Refactor revalidation
    // uses the default registry, which only owns in-process parsers, so support
    // must be checked against that registry instead of assumed from the kind.
    let supported = ParserRegistry::with_defaults().supports(path);
    let outcome = validate_file_parse_for_refactor(path, &content);

    match outcome {
        Some(outcome) => {
            assert!(
                supported,
                "refactor validation returned an outcome for unregistered path `{path}`"
            );
            std::hint::black_box(outcome.tree_has_error);
            std::hint::black_box(outcome.parsed_file.as_ref().map(|parsed| parsed.path.as_str()));
            for warning in &outcome.validation.warnings {
                std::hint::black_box(warning.chars().count());
            }
            for error in &outcome.validation.errors {
                std::hint::black_box(error.chars().count());
            }
        }
        None => assert!(
            !supported,
            "registered refactor validation path `{path}` should return outcome"
        ),
    }
});

fn bounded_content(content: &[u8]) -> Vec<u8> {
    content[..content.len().min(MAX_CONTENT_BYTES)].to_vec()
}

impl ValidationPathKind {
    fn rel_path(self) -> &'static str {
        match self {
            Self::Supported(path_kind) => path_kind.rel_path(),
            Self::Unsupported(path_kind) => path_kind.rel_path(),
        }
    }
}

impl UnsupportedPathKind {
    fn rel_path(self) -> &'static str {
        match self {
            Self::Text => "src/fuzz.txt",
            Self::Yaml => "src/fuzz.yaml",
            Self::Sql => "src/fuzz.sql",
            Self::NoExtension => "src/fuzz",
        }
    }
}
