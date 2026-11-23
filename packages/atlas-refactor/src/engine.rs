//! `RefactorEngine` — Phase 24 implementation.
//!
//! Deterministic, graph-backed refactoring operations. All mutations go
//! through a plan-then-apply pipeline with dry-run support.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use atlas_core::{
    AtlasError, EdgeKind, ExtractFunctionCandidate, NodeKind, RefactorDryRunResult, RefactorEdit,
    RefactorEditKind, RefactorOperation, RefactorPatch, RefactorPlan, RefactorValidationResult,
    Result, SafetyBand, SimulatedRefactorImpact,
};
use atlas_parser::ParserRegistry;
use atlas_repo::stable_repo_id;
use atlas_store_sqlite::Store;
use camino::Utf8Path;
use sha2::{Digest, Sha256};
use tracing::debug;

use crate::edits::{apply_edits, check_overlaps, replace_identifier, validate_identifier};
use crate::extract::detect_candidates;
use crate::patch::unified_diff_annotated;

/// Minimal public harness for parser revalidation used by refactor apply flow.
#[derive(Debug, Clone)]
pub struct RefactorParseValidation {
    pub parsed_file: Option<atlas_core::ParsedFile>,
    pub tree_has_error: bool,
    pub validation: RefactorValidationResult,
}

/// Run the same parser revalidation path used by refactor apply without
/// requiring a full engine/store setup. Unsupported files return `None`.
pub fn validate_file_parse_for_refactor(
    file_path: &str,
    content: &str,
) -> Option<RefactorParseValidation> {
    let parser_registry = ParserRegistry::with_defaults();
    if !parser_registry.supports(file_path) {
        return None;
    }

    let mut validation = RefactorValidationResult {
        valid: true,
        errors: Vec::new(),
        warnings: Vec::new(),
        manual_review: Vec::new(),
    };

    let parsed =
        parse_file_content_with_registry(&parser_registry, file_path, content, &mut validation);

    Some(match parsed {
        Some((parsed_file, tree)) => RefactorParseValidation {
            tree_has_error: tree
                .as_ref()
                .is_some_and(|tree| tree.root_node().has_error()),
            parsed_file: Some(parsed_file),
            validation,
        },
        None => RefactorParseValidation {
            parsed_file: None,
            tree_has_error: false,
            validation,
        },
    })
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Edge query cap for per-node lookups.
const EDGE_QUERY_LIMIT: usize = 500;

/// Simple-name entrypoints suppressed from auto-removal.
const ENTRYPOINT_NAMES: &[&str] = &[
    "main",
    "new",
    "init",
    "setup",
    "configure",
    "run",
    "start",
    "handler",
    "middleware",
];

// ---------------------------------------------------------------------------
// RefactorEngine
// ---------------------------------------------------------------------------

/// Provides deterministic refactoring operations backed by the Atlas graph store.
pub struct RefactorEngine<'s> {
    store: &'s mut Store,
    /// Absolute path to the repository root (used to resolve file paths for I/O).
    repo_root: &'s Path,
    parser_registry: ParserRegistry,
}

mod helpers;
mod ops;

use self::helpers::parse_file_content_with_registry;

#[cfg(test)]
mod tests {
    use super::helpers::*;
    use super::*;

    #[test]
    fn extract_rust_use_single() {
        assert_eq!(extract_rust_use_names("use std::io::Write;"), vec!["Write"]);
    }

    #[test]
    fn extract_rust_use_multi() {
        let names = extract_rust_use_names("use std::io::{Read, Write, BufReader};");
        assert_eq!(names, vec!["Read", "Write", "BufReader"]);
    }

    #[test]
    fn extract_rust_use_alias() {
        assert_eq!(
            extract_rust_use_names("use std::collections::HashMap as Map;"),
            vec!["Map"]
        );
    }

    #[test]
    fn extract_rust_use_glob_ignored() {
        assert!(extract_rust_use_names("use std::io::*;").is_empty());
    }

    #[test]
    fn extract_python_from_import() {
        assert_eq!(
            extract_python_import_names("from os import path, getcwd"),
            vec!["path", "getcwd"]
        );
    }

    #[test]
    fn extract_python_import_simple() {
        assert_eq!(extract_python_import_names("import os"), vec!["os"]);
    }

    #[test]
    fn extract_js_named() {
        assert_eq!(
            extract_js_import_names("import { Foo, Bar } from './foo'"),
            vec!["Foo", "Bar"]
        );
    }

    #[test]
    fn extract_js_default() {
        assert_eq!(
            extract_js_import_names("import React from 'react'"),
            vec!["React"]
        );
    }

    #[test]
    fn contains_word_basic() {
        assert!(contains_word("let foo = bar;", "foo"));
        assert!(!contains_word("let foobar = 1;", "foo"));
    }

    #[test]
    fn validate_file_parse_for_refactor_skips_unsupported_path() {
        assert!(validate_file_parse_for_refactor("notes.txt", "hello").is_none());
    }

    #[test]
    fn validate_file_parse_for_refactor_skips_offloaded_language_paths() {
        for path in [
            "src/fuzz.cpp",
            "src/Main.java",
            "src/App.cs",
            "src/index.php",
            "src/Fuzz.scala",
            "src/fuzz.rb",
            "src/fuzz.sh",
        ] {
            assert!(
                validate_file_parse_for_refactor(path, "fn main() {}").is_none(),
                "offloaded language path `{path}` must be skipped by the default registry"
            );
        }
    }

    #[test]
    fn validate_file_parse_for_refactor_warns_on_empty_supported_file() {
        let outcome = validate_file_parse_for_refactor("src/lib.rs", "")
            .expect("supported path should produce validation outcome");

        assert!(outcome.parsed_file.is_none());
        assert!(!outcome.tree_has_error);
        assert_eq!(outcome.validation.errors, Vec::<String>::new());
        assert_eq!(outcome.validation.warnings.len(), 1);
        assert!(outcome.validation.warnings[0].contains("empty file"));
        assert!(
            std::str::from_utf8(outcome.validation.warnings[0].as_bytes()).is_ok(),
            "warnings must stay UTF-8 safe"
        );
    }

    #[test]
    fn validate_file_parse_for_refactor_handles_malformed_supported_source() {
        let outcome = validate_file_parse_for_refactor("src/lib.rs", "fn broken( {")
            .expect("supported path should produce validation outcome");

        assert!(
            outcome.parsed_file.is_some(),
            "tree-sitter should still yield best-effort parse output"
        );
        for warning in &outcome.validation.warnings {
            assert!(std::str::from_utf8(warning.as_bytes()).is_ok());
        }
        for error in &outcome.validation.errors {
            assert!(std::str::from_utf8(error.as_bytes()).is_ok());
        }
    }
}
