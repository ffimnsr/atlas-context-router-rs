use std::collections::BTreeMap;

use atlas_parser::{ExternalExtractor, ParserRegistry};
use camino::Utf8PathBuf;

pub(crate) struct OffloadedLanguageCheck {
    pub(crate) ok: bool,
    pub(crate) detail: String,
    pub(crate) issue_code: Option<&'static str>,
}

pub(crate) fn check_tracked_files(
    files: &[Utf8PathBuf],
    registry: &ParserRegistry,
) -> OffloadedLanguageCheck {
    let offloaded = offloaded_extensions();
    let mut missing: BTreeMap<&'static str, usize> = BTreeMap::new();

    for file in files {
        let Some(extension) = file.extension() else {
            continue;
        };
        let Some(extension) = offloaded
            .iter()
            .copied()
            .find(|candidate| *candidate == extension)
        else {
            continue;
        };
        if registry.supports(file.as_str()) {
            continue;
        }
        *missing.entry(extension).or_default() += 1;
    }

    if missing.is_empty() {
        return OffloadedLanguageCheck {
            ok: true,
            detail: "all tracked offloaded-language files have registered parsers".to_owned(),
            issue_code: None,
        };
    }

    let detail = missing
        .iter()
        .map(|(extension, count)| format!(".{extension} ({count})"))
        .collect::<Vec<_>>()
        .join(", ");
    OffloadedLanguageCheck {
        ok: false,
        detail: format!(
            "tracked offloaded-language files have no registered parser: {detail}; \
             add [parsers.external] extractor profiles"
        ),
        issue_code: Some("unregistered_offloaded_language"),
    }
}

fn offloaded_extensions() -> Vec<&'static str> {
    [
        ExternalExtractor::Php,
        ExternalExtractor::Cpp,
        ExternalExtractor::CSharp,
        ExternalExtractor::Java,
        ExternalExtractor::Scala,
        ExternalExtractor::Ruby,
        ExternalExtractor::Bash,
    ]
    .into_iter()
    .flat_map(|extractor| extractor.default_extensions().iter().copied())
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_offloaded_extensions_without_registered_parsers() {
        let files = vec![
            Utf8PathBuf::from("src/app.php"),
            Utf8PathBuf::from("src/native.cpp"),
            Utf8PathBuf::from("src/main.rs"),
        ];
        let check = check_tracked_files(&files, &ParserRegistry::with_defaults());

        assert!(!check.ok);
        assert_eq!(check.issue_code, Some("unregistered_offloaded_language"));
        assert!(check.detail.contains(".php (1)"), "{}", check.detail);
        assert!(check.detail.contains(".cpp (1)"), "{}", check.detail);
    }

    #[test]
    fn passes_when_only_bundled_extensions_are_tracked() {
        let files = vec![
            Utf8PathBuf::from("src/main.rs"),
            Utf8PathBuf::from("src/app.py"),
            Utf8PathBuf::from("src/app.go"),
        ];
        let check = check_tracked_files(&files, &ParserRegistry::with_defaults());

        assert!(check.ok);
        assert_eq!(check.issue_code, None);
    }
}
