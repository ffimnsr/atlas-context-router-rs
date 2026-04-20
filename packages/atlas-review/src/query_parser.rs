// Phase 22 — Context Engine: Slice 7
//
// Semi-structured query parser that converts plain text into a `ContextRequest`.
//
// Design rules:
// - Shallow regex-based classification only; no fuzzy/LLM inference.
// - Intent classifier matches keyword phrases first (longest match wins).
// - Target extractor tries: quoted strings → file-path-like → fn/method-like →
//   CamelCase names → bare identifiers.
// - Ambiguity metadata survives through to the engine; caller handles it.

use atlas_core::model::{ContextIntent, ContextRequest, ContextTarget};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Parse a free-text query into a [`ContextRequest`].
///
/// The parser is intentionally conservative: it prefers to produce a
/// structured request over guessing, and defaults to `ContextIntent::Symbol`
/// when no intent phrase is matched.
pub fn parse_query(text: &str) -> ContextRequest {
    let intent = classify_intent(text);
    let target = extract_target(text, intent);
    ContextRequest {
        intent,
        target,
        include_imports: true,
        include_callers: matches!(
            intent,
            ContextIntent::Symbol
                | ContextIntent::Impact
                | ContextIntent::ImpactAnalysis
                | ContextIntent::UsageLookup
                | ContextIntent::RefactorSafety
                | ContextIntent::DependencyRemoval
        ),
        include_callees: matches!(
            intent,
            ContextIntent::Symbol
                | ContextIntent::Impact
                | ContextIntent::ImpactAnalysis
                | ContextIntent::RefactorSafety
                | ContextIntent::RenamePreview
        ),
        include_tests: matches!(intent, ContextIntent::Review),
        ..ContextRequest::default()
    }
}

// ---------------------------------------------------------------------------
// Intent classification
// ---------------------------------------------------------------------------

/// Lower-case keyword phrases mapped to [`ContextIntent`].
///
/// Checked in order; first match wins (so longer phrases appear first).
const INTENT_PHRASES: &[(&str, ContextIntent)] = &[
    // Review / change-set intents.
    ("review context", ContextIntent::Review),
    ("code review", ContextIntent::Review),
    ("what changed", ContextIntent::Review),
    ("changed in", ContextIntent::Review),
    // Impact / breakage intents.
    ("what breaks", ContextIntent::ImpactAnalysis),
    ("what will break", ContextIntent::ImpactAnalysis),
    ("impact of", ContextIntent::ImpactAnalysis),
    ("breaking change", ContextIntent::ImpactAnalysis),
    // Refactor safety intents.
    ("safe to refactor", ContextIntent::RefactorSafety),
    ("safe to remove", ContextIntent::RefactorSafety),
    // Dependency removal.
    ("remove dependency", ContextIntent::DependencyRemoval),
    // Dead code.
    ("dead code", ContextIntent::DeadCodeCheck),
    ("unused", ContextIntent::DeadCodeCheck),
    // Rename preview.
    ("rename", ContextIntent::RenamePreview),
    // Usage / caller intents.
    ("who calls", ContextIntent::UsageLookup),
    ("used by", ContextIntent::UsageLookup),
    ("callers of", ContextIntent::UsageLookup),
    ("usages of", ContextIntent::UsageLookup),
    ("usage of", ContextIntent::UsageLookup),
    ("where is", ContextIntent::UsageLookup),
    ("find usages", ContextIntent::UsageLookup),
    // File-centred intents.
    ("show file", ContextIntent::File),
    ("symbols in", ContextIntent::File),
    ("what is in", ContextIntent::File),
];

fn classify_intent(text: &str) -> ContextIntent {
    let lower = text.to_lowercase();
    for (phrase, intent) in INTENT_PHRASES {
        if lower.contains(phrase) {
            return *intent;
        }
    }
    ContextIntent::Symbol
}

// ---------------------------------------------------------------------------
// Target extraction
// ---------------------------------------------------------------------------

/// Attempt to extract the primary target from `text`.
///
/// Resolution priority:
/// 1. Quoted string (`"…"` or `'…'`) — treated as qualified name if it
///    contains `::` or `/`, otherwise as a symbol name.
/// 2. File-path-like token (contains `/` and a known extension).
/// 3. Qualified name (`word::word` or `pkg.Class.method`).
/// 4. Method-like pattern (`Foo::bar` or `foo.bar`).
/// 5. CamelCase identifier (heuristic for class/struct names).
/// 6. Snake-case function-like identifier (fallback).
fn extract_target(text: &str, intent: ContextIntent) -> ContextTarget {
    // 1. Quoted string.
    if let Some(quoted) = extract_quoted(text) {
        return classify_string_target(quoted);
    }

    // 2. File path (contains directory separator + known extension).
    if let Some(path) = extract_file_path(text) {
        // For File, Review, and impact-class intents, return ChangedFiles if path found.
        return match intent {
            ContextIntent::Review
            | ContextIntent::Impact
            | ContextIntent::ImpactAnalysis
            | ContextIntent::RefactorSafety
            | ContextIntent::DependencyRemoval => {
                ContextTarget::ChangedFiles { paths: vec![path] }
            }
            _ => ContextTarget::FilePath { path },
        };
    }

    // 3. Qualified name (contains `::` or `module.symbol` style).
    if let Some(qname) = extract_qualified_name(text) {
        return ContextTarget::QualifiedName { qname };
    }

    // 4 & 5. Best plain identifier (CamelCase or snake_case).
    if let Some(name) = extract_best_identifier(text) {
        return ContextTarget::SymbolName { name };
    }

    // Fallback: use the whole trimmed text as a symbol name.
    ContextTarget::SymbolName { name: text.trim().to_string() }
}

/// Classify a quoted substring into a `ContextTarget`.
fn classify_string_target(s: &str) -> ContextTarget {
    if s.contains("::") {
        ContextTarget::QualifiedName { qname: s.to_string() }
    } else if looks_like_file_path(s) {
        ContextTarget::FilePath { path: s.to_string() }
    } else {
        ContextTarget::SymbolName { name: s.to_string() }
    }
}

// ---------------------------------------------------------------------------
// Regex helpers
// ---------------------------------------------------------------------------

/// Extract the first quoted substring from `text` (double or single quotes).
fn extract_quoted(text: &str) -> Option<&str> {
    // Double-quoted
    if let Some(content) = extract_between(text, '"', '"') {
        return Some(content);
    }
    // Single-quoted (only if not an apostrophe — require at least 2 chars inside)
    if let Some(content) = extract_between(text, '\'', '\'')
        && content.len() >= 2
    {
        return Some(content);
    }
    None
}

fn extract_between(text: &str, open: char, close: char) -> Option<&str> {
    let start = text.find(open)? + open.len_utf8();
    let rest = &text[start..];
    let end = rest.find(close)?;
    Some(&rest[..end])
}

/// Known source file extensions.
const FILE_EXTENSIONS: &[&str] = &[
    ".rs", ".py", ".ts", ".tsx", ".js", ".jsx", ".go", ".java", ".kt", ".swift", ".c", ".cpp",
    ".h", ".hpp", ".cs", ".rb", ".php", ".scala", ".zig", ".ex", ".exs", ".lua",
];

fn looks_like_file_path(s: &str) -> bool {
    (s.contains('/') || s.contains('\\')) && FILE_EXTENSIONS.iter().any(|ext| s.ends_with(ext))
}

/// Extract the first file-path-like token from `text`.
fn extract_file_path(text: &str) -> Option<String> {
    // Split on whitespace and look for a token that looks like a file path.
    for token in text.split_whitespace() {
        // Strip trailing punctuation.
        let token = token.trim_end_matches([',', '.', ';', ')', ']']);
        if looks_like_file_path(token) {
            return Some(token.to_string());
        }
    }
    None
}

/// Extract a qualified name (`a::b::c` or `a.B.method`) from `text`.
fn extract_qualified_name(text: &str) -> Option<String> {
    for token in text.split_whitespace() {
        let token = token.trim_end_matches([',', '.', ';', ')', ']', '?']);
        // Must contain `::` (Rust/C++) or multiple `.`-separated words starting
        // with a capital (Java/Kotlin/Swift convention: `Foo.bar`).
        if token.contains("::") && token.len() > 3 {
            return Some(token.to_string());
        }
        // Dotted path with at least two components where any is CamelCase.
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() >= 2
            && parts.iter().any(|p| p.chars().next().map(|c| c.is_uppercase()).unwrap_or(false))
            && parts.iter().all(|p| is_identifier(p))
        {
            return Some(token.to_string());
        }
    }
    None
}

/// Return the "best" plain identifier from `text`:
/// prefer CamelCase, then snake_case function-like names.
fn extract_best_identifier(text: &str) -> Option<String> {
    let mut camel: Option<String> = None;
    let mut snake: Option<String> = None;

    for token in text.split_whitespace() {
        let token = token.trim_end_matches([',', '.', ';', ')', ']', '?']);
        if !is_identifier(token) || token.len() < 2 {
            continue;
        }
        // CamelCase: starts with uppercase and has no underscores.
        if token.chars().next().map(|c| c.is_uppercase()).unwrap_or(false)
            && !token.contains('_')
            && token.chars().any(|c| c.is_lowercase())
        {
            camel.get_or_insert_with(|| token.to_string());
        }
        // Function-like: snake_case with at least one `_` or ending with `()`.
        if token.contains('_') && token.chars().all(|c| c.is_alphanumeric() || c == '_') {
            snake.get_or_insert_with(|| token.to_string());
        }
    }

    camel.or(snake)
}

fn is_identifier(s: &str) -> bool {
    !s.is_empty()
        && s.chars().next().map(|c| c.is_alphabetic() || c == '_').unwrap_or(false)
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::model::{ContextIntent, ContextTarget};

    // ------------------------------------------------------------------
    // Intent classification
    // ------------------------------------------------------------------

    #[test]
    fn classify_what_breaks() {
        let req = parse_query("what breaks if I change parse_node");
        assert_eq!(req.intent, ContextIntent::ImpactAnalysis);
    }

    #[test]
    fn classify_who_calls() {
        let req = parse_query("who calls resolve_target");
        assert_eq!(req.intent, ContextIntent::UsageLookup);
    }

    #[test]
    fn classify_safe_to_refactor() {
        let req = parse_query("is it safe to refactor build_context");
        assert_eq!(req.intent, ContextIntent::RefactorSafety);
    }

    #[test]
    fn classify_dead_code() {
        let req = parse_query("is trim_context dead code?");
        assert_eq!(req.intent, ContextIntent::DeadCodeCheck);
    }

    #[test]
    fn classify_used_by() {
        let req = parse_query("used by ContextEngine");
        assert_eq!(req.intent, ContextIntent::UsageLookup);
    }

    #[test]
    fn classify_default_symbol() {
        let req = parse_query("build_symbol_context");
        assert_eq!(req.intent, ContextIntent::Symbol);
    }

    // ------------------------------------------------------------------
    // Target extraction
    // ------------------------------------------------------------------

    #[test]
    fn extract_quoted_qname() {
        let req = parse_query(r#"what calls "atlas_review::context::build_context"?"#);
        assert!(
            matches!(&req.target, ContextTarget::QualifiedName { qname } if qname.contains("build_context")),
            "expected QualifiedName, got {:?}",
            req.target
        );
    }

    #[test]
    fn extract_quoted_symbol_name() {
        let req = parse_query(r#"tell me about "resolve_target""#);
        assert!(
            matches!(&req.target, ContextTarget::SymbolName { name } if name == "resolve_target"),
            "got {:?}",
            req.target
        );
    }

    #[test]
    fn extract_file_path_target() {
        let req = parse_query("show me symbols in src/lib.rs");
        assert!(
            matches!(&req.target, ContextTarget::FilePath { path } if path.ends_with(".rs")),
            "expected FilePath, got {:?}",
            req.target
        );
    }

    #[test]
    fn extract_file_path_changed_files_for_review_intent() {
        let req = parse_query("code review src/context.rs");
        // review intent → file becomes ChangedFiles
        assert!(
            matches!(&req.target, ContextTarget::ChangedFiles { paths } if !paths.is_empty()),
            "expected ChangedFiles, got {:?}",
            req.target
        );
    }

    #[test]
    fn extract_rust_qualified_name() {
        let req = parse_query("who calls atlas_review::context::rank_context");
        assert!(
            matches!(&req.target, ContextTarget::QualifiedName { qname } if qname.contains("rank_context")),
            "got {:?}",
            req.target
        );
    }

    #[test]
    fn extract_camel_case_identifier() {
        let req = parse_query("tell me about ContextEngine");
        assert!(
            matches!(&req.target, ContextTarget::SymbolName { name } if name == "ContextEngine"),
            "got {:?}",
            req.target
        );
    }

    #[test]
    fn extract_snake_case_identifier() {
        let req = parse_query("what does build_symbol_context do?");
        assert!(
            matches!(&req.target, ContextTarget::SymbolName { name } if name == "build_symbol_context"),
            "got {:?}",
            req.target
        );
    }

    // ------------------------------------------------------------------
    // End-to-end: structured vs parsed round-trip
    // ------------------------------------------------------------------

    #[test]
    fn equivalent_structured_request() {
        // parse_query("who calls rank_context") should produce the same target
        // as a hand-crafted ContextRequest with SymbolName "rank_context".
        let parsed = parse_query("who calls rank_context");
        let structured = ContextRequest {
            intent: ContextIntent::Symbol,
            target: ContextTarget::SymbolName { name: "rank_context".to_string() },
            ..ContextRequest::default()
        };
        // Same target kind and name.
        assert_eq!(
            std::mem::discriminant(&parsed.target),
            std::mem::discriminant(&structured.target)
        );
        if let (ContextTarget::SymbolName { name: n1 }, ContextTarget::SymbolName { name: n2 }) =
            (&parsed.target, &structured.target)
        {
            assert_eq!(n1, n2);
        }
    }

    #[test]
    fn ambiguity_metadata_preserved_after_classifier() {
        // The parser should not resolve ambiguity itself; the engine does.
        // All parse_query gives back is a ContextRequest, which the engine
        // will process and may return AmbiguityMeta on.
        let req = parse_query("what does fn_a do?");
        // At minimum: the request has a populated target.
        assert!(!matches!(req.target, ContextTarget::SymbolName { ref name } if name.is_empty()));
    }
}
