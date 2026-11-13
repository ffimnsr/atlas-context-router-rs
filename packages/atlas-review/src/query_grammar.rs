//! Shared query-grammar parsing for `query_graph`, `explain_query`, and the
//! CLI `query` command.
//!
//! One parser serves every surface so intent classification, normalization, and
//! natural-language rejection stay byte-identical across MCP and CLI.

use atlas_core::model::{ContextIntent, ContextRequest, ContextTarget};

use crate::query_parser;

/// Parsed result for a supported query-grammar input.
#[derive(Clone, Debug)]
pub struct QueryGrammar {
    /// Grammar kind: `who_calls`, `what_breaks`, `tests_for`,
    /// `exact_qualified_name`, `plain_identifier`, `structured_query`, or
    /// `unsupported_description`.
    pub kind: &'static str,
    /// `false` when the input is an unstructured natural-language description.
    pub accepted: bool,
    /// Text exactly as supplied.
    pub source_text: String,
    /// Lookup text after stripping intent prefixes.
    pub normalized_text: String,
    /// Parsed context request carried through to the execution layer.
    pub parsed_request: ContextRequest,
}

/// Canonical examples documented for the supported query grammar.
pub fn query_grammar_examples() -> &'static [&'static str] {
    &[
        "compute",
        "src/service.rs::fn::compute",
        "who calls compute",
        "what breaks if I change compute",
        "tests for compute",
    ]
}

fn strip_case_insensitive_prefix<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    if text.len() < prefix.len() {
        return None;
    }
    let head = text.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) {
        Some(text[prefix.len()..].trim())
    } else {
        None
    }
}

/// Supported intent prefixes. Each maps to a concrete execution path.
const SUPPORTED_INTENT_PREFIXES: &[&str] = &[
    "tests for ",
    "who calls ",
    "what breaks if i change ",
    "what breaks ",
];

/// Strip a supported intent prefix and return the remaining target text.
pub fn strip_supported_intent_prefix(text: &str) -> Option<&str> {
    SUPPORTED_INTENT_PREFIXES
        .iter()
        .find_map(|prefix| strip_case_insensitive_prefix(text, prefix))
}

fn target_to_lookup_text(target: &ContextTarget) -> Option<String> {
    match target {
        ContextTarget::QualifiedName { qname } => Some(qname.clone()),
        ContextTarget::SymbolName { name } => Some(name.clone()),
        ContextTarget::FilePath { path } => Some(path.clone()),
        ContextTarget::ChangedFiles { paths } => paths.first().cloned(),
        ContextTarget::ChangedSymbols { qnames } => qnames.first().cloned(),
        ContextTarget::EdgeQuerySeed { source_qname, .. } => Some(source_qname.clone()),
    }
}

fn looks_code_like_query(trimmed: &str) -> bool {
    trimmed.contains("::")
        || trimmed.contains('/')
        || trimmed.contains('_')
        || trimmed.contains('(')
        || trimmed.contains('.')
        || trimmed.chars().any(|ch| ch.is_ascii_uppercase())
}

fn looks_plain_identifier(trimmed: &str) -> bool {
    !trimmed.is_empty()
        && !trimmed.contains(char::is_whitespace)
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | ':' | '/' | '.'))
}

/// Return `true` when `query` is a natural-language description that carries no
/// concrete identifier.
///
/// Supported intent phrases are accepted only when the remainder is a concrete
/// identifier. Other intent phrases (for example `where is ...`) are rejected so
/// the caller can return actionable grammar guidance instead of an empty result.
pub fn query_looks_like_unstructured_description(query: &str) -> bool {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return false;
    }
    if let Some(rest) = strip_supported_intent_prefix(trimmed) {
        return !looks_plain_identifier(rest);
    }
    if !trimmed.contains(char::is_whitespace) {
        return false;
    }
    if looks_code_like_query(trimmed) {
        return false;
    }

    let parsed = query_parser::parse_query(trimmed);
    if parsed.intent == ContextIntent::Symbol
        && matches!(parsed.target, ContextTarget::SymbolName { ref name } if name == trimmed)
    {
        return true;
    }

    let words = trimmed
        .split_whitespace()
        .map(|word| word.trim_matches(|ch: char| !ch.is_alphanumeric()))
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    let natural_language_cues = [
        "please", "show", "find", "explain", "review", "check", "tell", "help", "look", "what",
        "why", "how", "where",
    ];
    words.len() >= 4
        || words.iter().any(|word| {
            natural_language_cues
                .iter()
                .any(|cue| word.eq_ignore_ascii_case(cue))
        })
}

/// Parse `query` into a [`QueryGrammar`] classification.
pub fn parse_query_grammar(query: &str) -> QueryGrammar {
    let trimmed = query.trim();

    if let Some(rest) = strip_case_insensitive_prefix(trimmed, "tests for ") {
        let parsed_target = query_parser::parse_query(rest);
        let normalized_text =
            target_to_lookup_text(&parsed_target.target).unwrap_or_else(|| rest.trim().to_owned());
        let mut parsed_request = parsed_target;
        parsed_request.intent = ContextIntent::UsageLookup;
        parsed_request.include_callers = true;
        parsed_request.include_callees = false;
        parsed_request.include_tests = true;
        return QueryGrammar {
            kind: "tests_for",
            accepted: !normalized_text.is_empty()
                && !query_looks_like_unstructured_description(rest),
            source_text: trimmed.to_owned(),
            normalized_text,
            parsed_request,
        };
    }

    let parsed_request = query_parser::parse_query(trimmed);
    let lower = trimmed.to_ascii_lowercase();
    let kind = if lower.starts_with("who calls ") {
        "who_calls"
    } else if lower.starts_with("what breaks") {
        "what_breaks"
    } else if matches!(&parsed_request.target, ContextTarget::QualifiedName { qname } if qname == trimmed)
    {
        "exact_qualified_name"
    } else if matches!(&parsed_request.target, ContextTarget::SymbolName { name } if name == trimmed)
        && looks_plain_identifier(trimmed)
    {
        "plain_identifier"
    } else if query_looks_like_unstructured_description(trimmed) {
        "unsupported_description"
    } else {
        "structured_query"
    };
    let normalized_text = match kind {
        "who_calls" | "what_breaks" => {
            target_to_lookup_text(&parsed_request.target).unwrap_or_else(|| trimmed.to_owned())
        }
        _ => trimmed.to_owned(),
    };

    QueryGrammar {
        kind,
        accepted: kind != "unsupported_description",
        source_text: trimmed.to_owned(),
        normalized_text,
        parsed_request,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_intent_phrases_are_accepted() {
        assert!(!query_looks_like_unstructured_description(
            "who calls handle_request"
        ));
        assert!(!query_looks_like_unstructured_description(
            "what breaks if I change compute"
        ));
        assert!(!query_looks_like_unstructured_description(
            "tests for compute"
        ));
    }

    #[test]
    fn intent_phrase_without_concrete_target_is_rejected() {
        assert!(query_looks_like_unstructured_description(
            "who calls the place that handles error codes"
        ));
    }

    #[test]
    fn unrecognized_natural_language_intent_is_rejected() {
        assert!(query_looks_like_unstructured_description(
            "where is the place that handles error codes?"
        ));
    }

    #[test]
    fn plain_descriptions_are_rejected_but_code_like_queries_are_not() {
        assert!(query_looks_like_unstructured_description(
            "please show me authentication flow"
        ));
        assert!(!query_looks_like_unstructured_description(
            "src/lib.rs::fn::handle_request"
        ));
        assert!(!query_looks_like_unstructured_description("handle_request"));
    }

    #[test]
    fn grammar_classifies_every_supported_shape() {
        assert_eq!(parse_query_grammar("who calls compute").kind, "who_calls");
        assert_eq!(
            parse_query_grammar("who calls compute").normalized_text,
            "compute"
        );
        assert_eq!(
            parse_query_grammar("what breaks if I change compute").kind,
            "what_breaks"
        );
        assert_eq!(parse_query_grammar("tests for compute").kind, "tests_for");
        assert_eq!(
            parse_query_grammar("src/service.rs::fn::compute").kind,
            "exact_qualified_name"
        );
        assert_eq!(parse_query_grammar("compute").kind, "plain_identifier");
        assert!(!parse_query_grammar("please show me authentication flow").accepted);
    }
}
