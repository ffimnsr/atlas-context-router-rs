use serde::{Deserialize, Serialize};

use super::reasoning::SafetyBand;

/// A category of refactoring operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RefactorOperation {
    RenameSymbol {
        old_qname: String,
        new_name: String,
    },
    RemoveDeadCode {
        target_qname: String,
    },
    CleanImports {
        file_path: String,
    },
    ExtractFunctionCandidate {
        file_path: String,
        line_start: u32,
        line_end: u32,
    },
}

/// The kind of text transformation a single edit performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefactorEditKind {
    /// Rename an occurrence of an identifier.
    RenameOccurrence,
    /// Remove a contiguous line span (dead symbol body).
    RemoveSpan,
    /// Remove an import/use statement.
    RemoveImport,
}

/// A single deterministic text replacement applied to one file.
///
/// Line numbers are 1-based and inclusive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefactorEdit {
    pub file_path: String,
    /// 1-based start line (inclusive).
    pub line_start: u32,
    /// 1-based end line (inclusive).
    pub line_end: u32,
    pub old_text: String,
    pub new_text: String,
    pub edit_kind: RefactorEditKind,
}

/// Unified-diff patch for one file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefactorPatch {
    pub file_path: String,
    pub unified_diff: String,
}

/// All planned edits and metadata describing the full refactoring step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefactorPlan {
    pub operation: RefactorOperation,
    pub edits: Vec<RefactorEdit>,
    pub affected_files: Vec<String>,
    /// References that require human review (low-confidence, dynamic, cross-module).
    pub manual_review: Vec<String>,
    pub estimated_safety: SafetyBand,
}

/// Structured outcome of a post-apply (or simulated) validation pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefactorValidationResult {
    pub valid: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub manual_review: Vec<String>,
}

/// Full result of a dry-run or applied refactoring execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefactorDryRunResult {
    pub plan: RefactorPlan,
    pub patches: Vec<RefactorPatch>,
    pub validation: RefactorValidationResult,
    /// Number of files changed (or would be changed in dry-run).
    pub files_changed: usize,
    /// Total edits applied.
    pub edit_count: usize,
    /// `true` when no files were actually written.
    pub dry_run: bool,
}

/// A candidate block for extract-function analysis (detection only; no auto-apply).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractFunctionCandidate {
    pub file_path: String,
    /// 1-based start line of the block.
    pub line_start: u32,
    /// 1-based end line of the block.
    pub line_end: u32,
    pub proposed_inputs: Vec<String>,
    pub proposed_outputs: Vec<String>,
    /// Higher = better extraction candidate.
    pub difficulty_score: f64,
    pub score_reasons: Vec<String>,
}

/// High-level simulated impact of a planned refactoring before any files are written.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulatedRefactorImpact {
    /// Qualified names of graph nodes within the blast radius.
    pub affected_symbols: Vec<String>,
    /// Files touched by the simulated edits or graph impact.
    pub affected_files: Vec<String>,
    /// 0.0 (most risky) to 1.0 (safest).
    pub safety_score: f64,
    /// Test nodes that may need re-running.
    pub nearby_tests: Vec<String>,
    /// Unresolved concerns that block a high-confidence apply.
    pub unresolved_risks: Vec<String>,
}
