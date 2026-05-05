#![no_main]

use arbitrary::Arbitrary;
use atlas_fuzz::SupportedPathKind;
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

    match case.path_kind {
        ValidationPathKind::Supported(path_kind) => {
            let outcome = validate_file_parse_for_refactor(path_kind.rel_path(), &content)
                .expect("supported refactor validation path should return outcome");
            std::hint::black_box(outcome.tree_has_error);
            std::hint::black_box(outcome.parsed_file.as_ref().map(|parsed| parsed.path.as_str()));
            for warning in &outcome.validation.warnings {
                std::hint::black_box(warning.chars().count());
            }
            for error in &outcome.validation.errors {
                std::hint::black_box(error.chars().count());
            }
        }
        ValidationPathKind::Unsupported(path_kind) => {
            assert!(
                validate_file_parse_for_refactor(path_kind.rel_path(), &content).is_none(),
                "unsupported refactor validation path should be skipped"
            );
        }
    }
});

fn bounded_content(content: &[u8]) -> Vec<u8> {
    content[..content.len().min(MAX_CONTENT_BYTES)].to_vec()
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
