//! Language support policy for Phase 2 features.
//!
//! Controls which advanced operations (rename, dead-code detection, import
//! cleanup) are enabled per language based on parser and graph maturity.
//!
//! Callers check [`LanguagePolicy::supports`] before running a feature and
//! degrade gracefully when a language is not yet mature enough.

use serde::{Deserialize, Serialize};

/// Feature gate identifiers for Phase 2 operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    /// Rename a symbol across all reference sites.
    Rename,
    /// Dead-code scanning based on inbound edge confidence.
    DeadCodeScan,
    /// Remove unused import / use declarations.
    ImportCleanup,
}

impl Feature {
    pub fn as_str(self) -> &'static str {
        match self {
            Feature::Rename => "rename",
            Feature::DeadCodeScan => "dead_code_scan",
            Feature::ImportCleanup => "import_cleanup",
        }
    }
}

impl std::fmt::Display for Feature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Maturity level of a language's parser and symbol/reference mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Maturity {
    /// Parser exists but symbol/reference mapping is incomplete.
    Experimental,
    /// Symbol mapping is usable; reference resolution is partial.
    Beta,
    /// Symbol and reference mapping are reliable for automated transforms.
    Stable,
}

/// Policy entry for a single language.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LangEntry {
    pub language: String,
    pub maturity: Maturity,
}

/// Policy table: which operations are allowed per language.
#[derive(Debug, Clone)]
pub struct LanguagePolicy {
    entries: Vec<LangEntry>,
}

impl Default for LanguagePolicy {
    fn default() -> Self {
        // Established maturity levels based on atlas-parser coverage.
        Self {
            entries: vec![
                LangEntry {
                    language: "rust".into(),
                    maturity: Maturity::Stable,
                },
                LangEntry {
                    language: "python".into(),
                    maturity: Maturity::Stable,
                },
                LangEntry {
                    language: "typescript".into(),
                    maturity: Maturity::Beta,
                },
                LangEntry {
                    language: "javascript".into(),
                    maturity: Maturity::Beta,
                },
                LangEntry {
                    language: "go".into(),
                    maturity: Maturity::Beta,
                },
                LangEntry {
                    language: "java".into(),
                    maturity: Maturity::Experimental,
                },
                LangEntry {
                    language: "c".into(),
                    maturity: Maturity::Experimental,
                },
                LangEntry {
                    language: "cpp".into(),
                    maturity: Maturity::Experimental,
                },
            ],
        }
    }
}

impl LanguagePolicy {
    /// Return the maturity level of `language` (case-insensitive).
    ///
    /// Unknown languages are treated as [`Maturity::Experimental`].
    pub fn maturity(&self, language: &str) -> Maturity {
        let lang_lower = language.to_lowercase();
        self.entries
            .iter()
            .find(|e| e.language == lang_lower)
            .map(|e| e.maturity)
            .unwrap_or(Maturity::Experimental)
    }

    /// Returns `true` when `feature` is enabled for `language`.
    ///
    /// | Feature            | Required maturity |
    /// |--------------------|-------------------|
    /// | `Rename`           | `Beta` or higher  |
    /// | `DeadCodeScan`     | `Beta` or higher  |
    /// | `ImportCleanup`    | `Stable`          |
    pub fn supports(&self, language: &str, feature: Feature) -> bool {
        let mat = self.maturity(language);
        match feature {
            Feature::Rename | Feature::DeadCodeScan => mat >= Maturity::Beta,
            Feature::ImportCleanup => mat >= Maturity::Stable,
        }
    }

    /// Human-readable reason why a feature is disabled for a language.
    pub fn degradation_reason(&self, language: &str, feature: Feature) -> Option<String> {
        if self.supports(language, feature) {
            return None;
        }
        let mat = self.maturity(language);
        Some(format!(
            "`{feature}` is not enabled for `{language}` (maturity: {mat:?}); \
             symbol/reference mapping is not yet reliable enough for automated transforms"
        ))
    }

    /// All entries in the policy table.
    pub fn entries(&self) -> &[LangEntry] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_supports_all_features() {
        let policy = LanguagePolicy::default();
        assert!(policy.supports("rust", Feature::Rename));
        assert!(policy.supports("rust", Feature::DeadCodeScan));
        assert!(policy.supports("rust", Feature::ImportCleanup));
    }

    #[test]
    fn typescript_beta_features() {
        let policy = LanguagePolicy::default();
        assert!(policy.supports("typescript", Feature::Rename));
        assert!(policy.supports("typescript", Feature::DeadCodeScan));
        assert!(!policy.supports("typescript", Feature::ImportCleanup));
    }

    #[test]
    fn java_experimental_no_features() {
        let policy = LanguagePolicy::default();
        assert!(!policy.supports("java", Feature::Rename));
        assert!(!policy.supports("java", Feature::DeadCodeScan));
        assert!(!policy.supports("java", Feature::ImportCleanup));
    }

    #[test]
    fn unknown_language_is_experimental() {
        let policy = LanguagePolicy::default();
        assert_eq!(policy.maturity("cobol"), Maturity::Experimental);
        assert!(!policy.supports("cobol", Feature::Rename));
    }

    #[test]
    fn degradation_reason_is_none_when_supported() {
        let policy = LanguagePolicy::default();
        assert!(policy.degradation_reason("rust", Feature::Rename).is_none());
    }

    #[test]
    fn degradation_reason_is_some_when_disabled() {
        let policy = LanguagePolicy::default();
        assert!(policy.degradation_reason("java", Feature::Rename).is_some());
    }
}
