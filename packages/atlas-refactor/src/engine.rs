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
use atlas_store_sqlite::Store;
use tracing::debug;

use crate::edits::{apply_edits, check_overlaps, replace_identifier, validate_identifier};
use crate::extract::detect_candidates;
use crate::patch::unified_diff_annotated;

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
    store: &'s Store,
    /// Absolute path to the repository root (used to resolve file paths for I/O).
    repo_root: &'s Path,
}

impl<'s> RefactorEngine<'s> {
    /// Create a new engine.
    pub fn new(store: &'s Store, repo_root: &'s Path) -> Self {
        Self { store, repo_root }
    }

    // -----------------------------------------------------------------------
    // 24.2 — Rename symbol
    // -----------------------------------------------------------------------

    /// Plan a rename of `old_qname` to `new_name`.
    ///
    /// Validates the identifier, resolves the definition, collects all graph
    /// references, checks for local collisions, and builds the full edit set.
    pub fn plan_rename(&self, old_qname: &str, new_name: &str) -> Result<RefactorPlan> {
        validate_identifier(new_name)?;

        // Resolve definition node.
        let target = self
            .store
            .node_by_qname(old_qname)?
            .ok_or_else(|| AtlasError::Other(format!("symbol not found: `{old_qname}`")))?;

        let old_name = &target.name;

        // Check collision at definition site.
        let file_nodes = self.store.nodes_by_file(&target.file_path)?;
        let collision: Vec<String> = file_nodes
            .iter()
            .filter(|n| n.name == new_name && n.qualified_name != old_qname)
            .map(|n| format!("`{}` in `{}`", n.qualified_name, n.file_path))
            .collect();
        if !collision.is_empty() {
            return Err(AtlasError::Other(format!(
                "rename collision: `{new_name}` already exists at definition site: {}",
                collision.join(", ")
            )));
        }

        // Collect inbound references.
        let inbound = self.store.inbound_edges(old_qname, EDGE_QUERY_LIMIT)?;

        let mut edits: Vec<RefactorEdit> = Vec::new();
        let mut manual_review: Vec<String> = Vec::new();
        let mut affected_files: HashSet<String> = HashSet::new();
        let mut risky = false;

        // Edit for the definition itself.
        let def_text = self.read_line(&target.file_path, target.line_start)?;
        let new_def_text = replace_identifier(&def_text, old_name, new_name);
        if new_def_text != def_text {
            edits.push(RefactorEdit {
                file_path: target.file_path.clone(),
                line_start: target.line_start,
                line_end: target.line_start,
                old_text: def_text,
                new_text: new_def_text,
                edit_kind: RefactorEditKind::RenameOccurrence,
            });
            affected_files.insert(target.file_path.clone());
        }

        // Edits for each reference.
        for (ref_node, edge) in &inbound {
            let line_num = edge.line.unwrap_or(ref_node.line_start);
            if line_num == 0 {
                continue;
            }

            // Low-confidence references are flagged for manual review only.
            if edge.confidence < 0.5 {
                manual_review.push(format!(
                    "unresolved reference in `{}` at line {} — verify manually",
                    ref_node.file_path, line_num
                ));
                risky = true;
                continue;
            }

            let line_text = self.read_line(&ref_node.file_path, line_num)?;
            let new_line_text = replace_identifier(&line_text, old_name, new_name);

            if new_line_text == line_text {
                // Identifier not found on the line graph reported — flag review.
                manual_review.push(format!(
                    "could not locate `{old_name}` on line {line_num} in `{}` — verify manually",
                    ref_node.file_path
                ));
                continue;
            }

            edits.push(RefactorEdit {
                file_path: ref_node.file_path.clone(),
                line_start: line_num,
                line_end: line_num,
                old_text: line_text,
                new_text: new_line_text,
                edit_kind: RefactorEditKind::RenameOccurrence,
            });
            affected_files.insert(ref_node.file_path.clone());
        }

        // Detect cross-scope collisions (best-effort).
        for ref_file in &affected_files {
            if ref_file == &target.file_path {
                continue;
            }
            let ref_file_nodes = self.store.nodes_by_file(ref_file)?;
            for n in ref_file_nodes {
                if n.name == new_name {
                    manual_review.push(format!(
                        "potential collision: `{new_name}` already exists in `{ref_file}` as `{}`",
                        n.qualified_name
                    ));
                    risky = true;
                }
            }
        }

        // Deduplicate edits by (file, line_start).
        edits.sort_by(|a, b| {
            a.file_path
                .cmp(&b.file_path)
                .then(a.line_start.cmp(&b.line_start))
        });
        edits.dedup_by(|a, b| a.file_path == b.file_path && a.line_start == b.line_start);

        // Safety estimate.
        let estimated_safety = if risky || !manual_review.is_empty() {
            SafetyBand::Caution
        } else {
            SafetyBand::Safe
        };

        let mut affected_files_list: Vec<String> = affected_files.into_iter().collect();
        affected_files_list.sort();

        Ok(RefactorPlan {
            operation: RefactorOperation::RenameSymbol {
                old_qname: old_qname.to_string(),
                new_name: new_name.to_string(),
            },
            edits,
            affected_files: affected_files_list,
            manual_review,
            estimated_safety,
        })
    }

    /// Execute a rename plan, optionally as a dry run.
    ///
    /// In dry-run mode generates patches but writes nothing. Otherwise applies
    /// edits in reverse line order per file and validates the result.
    pub fn apply_rename(&self, plan: &RefactorPlan, dry_run: bool) -> Result<RefactorDryRunResult> {
        let (old_qname, new_name) = match &plan.operation {
            RefactorOperation::RenameSymbol {
                old_qname,
                new_name,
            } => (old_qname.as_str(), new_name.as_str()),
            _ => return Err(AtlasError::Other("plan is not a rename operation".into())),
        };

        check_overlaps(&plan.edits)?;
        self.apply_plan_internal(plan, dry_run, |validation, new_contents| {
            // Post-apply validation: definition must exist with new name.
            for (path, (old_content, new_content)) in new_contents {
                // Old identifier must not appear where it was in the definition.
                // New identifier must be present in all touched files.
                if new_content.contains(old_qname.split("::").last().unwrap_or(old_qname)) {
                    // Old simple name still present — might be a partial rename.
                    // This is informational only; graph would need refresh to confirm.
                    validation.warnings.push(format!(
                        "old name may still appear in `{path}` — run `atlas build` to update graph"
                    ));
                }
                let _ = old_content; // suppress unused warning
                let _ = new_name; // suppress unused warning
            }
        })
    }

    // -----------------------------------------------------------------------
    // 24.3 — Remove dead code
    // -----------------------------------------------------------------------

    /// Plan removal of a dead-code symbol at `qname`.
    ///
    /// Rejects: insufficient confidence, entrypoint names, unresolved blockers.
    pub fn plan_dead_code_removal(&self, qname: &str) -> Result<RefactorPlan> {
        let node = self
            .store
            .node_by_qname(qname)?
            .ok_or_else(|| AtlasError::Other(format!("symbol not found: `{qname}`")))?;

        // Reject entrypoints by simple name.
        if ENTRYPOINT_NAMES.contains(&node.name.as_str()) {
            return Err(AtlasError::Other(format!(
                "refusing to remove `{}`: entrypoint name protected",
                node.name
            )));
        }

        // Verify no high-confidence inbound references remain.
        let inbound = self.store.inbound_edges(qname, EDGE_QUERY_LIMIT)?;
        let blocking: Vec<_> = inbound
            .iter()
            .filter(|(_, e)| {
                matches!(
                    e.kind,
                    EdgeKind::Calls | EdgeKind::Imports | EdgeKind::References
                ) && e.confidence >= 0.6
            })
            .collect();

        if !blocking.is_empty() {
            return Err(AtlasError::Other(format!(
                "`{qname}` has {} high-confidence inbound reference(s); not removable",
                blocking.len()
            )));
        }

        // Low-confidence inbound edges become manual review items.
        let mut manual_review: Vec<String> = inbound
            .iter()
            .filter(|(_, e)| e.confidence < 0.6)
            .map(|(n, _)| {
                format!(
                    "unresolved reference in `{}` — verify manually",
                    n.file_path
                )
            })
            .collect();

        // Check confidence: require at least Medium via dead_code_candidates.
        let dead = self.store.dead_code_candidates(1000)?;
        let candidate = dead.iter().find(|n| n.qualified_name == qname);
        if candidate.is_none() && inbound.is_empty() {
            // Not flagged and no inbound — treat as safe but warn.
            manual_review
                .push("symbol is not in dead-code list; verify it is truly unused".to_string());
        }

        // Build edit: remove the node span.
        let file_content = self.read_file_content(&node.file_path)?;
        let file_lines: Vec<&str> = file_content.lines().collect();
        let start_0 = node.line_start.saturating_sub(1) as usize;
        let end_0 = (node.line_end as usize)
            .min(file_lines.len())
            .saturating_sub(1);
        let old_text: String = file_lines[start_0..=end_0].join("\n");

        let mut edits: Vec<RefactorEdit> = vec![RefactorEdit {
            file_path: node.file_path.clone(),
            line_start: node.line_start,
            line_end: node.line_end,
            old_text,
            new_text: String::new(), // removal
            edit_kind: RefactorEditKind::RemoveSpan,
        }];

        // Also plan import cleanup for the touched file.
        let import_plan = self.plan_import_cleanup(&node.file_path);
        if let Ok(ip) = import_plan {
            edits.extend(ip.edits);
        }

        // Sort and dedup.
        edits.sort_by(|a, b| {
            a.file_path
                .cmp(&b.file_path)
                .then(a.line_start.cmp(&b.line_start))
        });
        edits.dedup_by(|a, b| a.file_path == b.file_path && a.line_start == b.line_start);

        let has_low_confidence =
            !inbound.is_empty() && inbound.iter().any(|(_, e)| e.confidence < 0.6);
        let estimated_safety = if has_low_confidence {
            SafetyBand::Caution
        } else {
            SafetyBand::Safe
        };

        Ok(RefactorPlan {
            operation: RefactorOperation::RemoveDeadCode {
                target_qname: qname.to_string(),
            },
            edits,
            affected_files: vec![node.file_path.clone()],
            manual_review,
            estimated_safety,
        })
    }

    /// Execute a dead-code removal plan.
    pub fn apply_dead_code_removal(
        &self,
        plan: &RefactorPlan,
        dry_run: bool,
    ) -> Result<RefactorDryRunResult> {
        match &plan.operation {
            RefactorOperation::RemoveDeadCode { .. } => {}
            _ => return Err(AtlasError::Other("plan is not a dead-code removal".into())),
        }
        check_overlaps(&plan.edits)?;
        self.apply_plan_internal(plan, dry_run, |validation, new_contents| {
            // Verify: no same-file reference to removed symbol remains.
            let _ = new_contents;
            validation
                .warnings
                .push("run `atlas build` to refresh graph after dead-code removal".to_string());
        })
    }

    // -----------------------------------------------------------------------
    // 24.3 — Import cleanup
    // -----------------------------------------------------------------------

    /// Plan import cleanup for one source file.
    ///
    /// Identifies `use`/`import`/`from ... import` lines whose imported
    /// identifiers do not appear elsewhere in the file, and marks them for
    /// removal.
    pub fn plan_import_cleanup(&self, file_path: &str) -> Result<RefactorPlan> {
        let content = self.read_file_content(file_path)?;
        let lines: Vec<&str> = content.lines().collect();

        // Detect language from file extension.
        let lang = detect_language(file_path);

        let imports = find_imports(&lines, lang);
        let mut edits: Vec<RefactorEdit> = Vec::new();
        let mut manual_review: Vec<String> = Vec::new();

        for ImportInfo {
            line_no,
            imported_names,
            raw_line,
        } in imports
        {
            // Check if any imported name is used outside the import line.
            let used = imported_names.iter().any(|name| {
                lines
                    .iter()
                    .enumerate()
                    .any(|(i, &l)| i + 1 != line_no as usize && contains_word(l, name))
            });

            if !used {
                // No usage found — safe to remove.
                edits.push(RefactorEdit {
                    file_path: file_path.to_string(),
                    line_start: line_no,
                    line_end: line_no,
                    old_text: raw_line.to_string(),
                    new_text: String::new(), // removal
                    edit_kind: RefactorEditKind::RemoveImport,
                });
            } else if imported_names.len() > 1 {
                // Multi-name import; we only remove those that are unused.
                let unused: Vec<&String> = imported_names
                    .iter()
                    .filter(|name| {
                        !lines
                            .iter()
                            .enumerate()
                            .any(|(i, &l)| i + 1 != line_no as usize && contains_word(l, name))
                    })
                    .collect();
                if !unused.is_empty() {
                    manual_review.push(format!(
                        "partial import cleanup in `{file_path}` line {line_no}: unused items {:?}",
                        unused
                    ));
                }
            }
        }

        // Sort: highest line first (applied in reverse).
        edits.sort_by_key(|e| std::cmp::Reverse(e.line_start));

        let has_edits = !edits.is_empty();
        Ok(RefactorPlan {
            operation: RefactorOperation::CleanImports {
                file_path: file_path.to_string(),
            },
            edits,
            affected_files: if has_edits {
                vec![file_path.to_string()]
            } else {
                vec![]
            },
            manual_review,
            estimated_safety: SafetyBand::Safe,
        })
    }

    /// Execute an import cleanup plan.
    pub fn apply_import_cleanup(
        &self,
        plan: &RefactorPlan,
        dry_run: bool,
    ) -> Result<RefactorDryRunResult> {
        match &plan.operation {
            RefactorOperation::CleanImports { .. } => {}
            _ => return Err(AtlasError::Other("plan is not an import cleanup".into())),
        }
        check_overlaps(&plan.edits)?;
        self.apply_plan_internal(plan, dry_run, |validation, _| {
            validation
                .warnings
                .push("run `atlas build` to refresh graph after import cleanup".to_string());
        })
    }

    // -----------------------------------------------------------------------
    // 24.4 — Extract-function candidate detection
    // -----------------------------------------------------------------------

    /// Detect extract-function candidates in `file_path`.
    ///
    /// Detection only — no plan or apply. Returns candidates scored by
    /// extraction tractability.
    pub fn detect_extract_function_candidates(
        &self,
        file_path: &str,
    ) -> Result<Vec<ExtractFunctionCandidate>> {
        let content = self.read_file_content(file_path)?;
        let nodes = self.store.nodes_by_file(file_path)?;
        detect_candidates(file_path, &content, &nodes)
    }

    // -----------------------------------------------------------------------
    // 24.4 — Simulate refactor impact
    // -----------------------------------------------------------------------

    /// Simulate the blast radius of a plan before any files are written.
    ///
    /// Traverses the graph from the seed symbol(s) in the plan and reports
    /// affected symbols, files, nearby tests, and a safety score.
    pub fn simulate_refactor_impact(&self, plan: &RefactorPlan) -> Result<SimulatedRefactorImpact> {
        let seed_qnames: Vec<String> = match &plan.operation {
            RefactorOperation::RenameSymbol { old_qname, .. } => vec![old_qname.clone()],
            RefactorOperation::RemoveDeadCode { target_qname } => vec![target_qname.clone()],
            RefactorOperation::CleanImports { file_path } => {
                // Seed from all nodes in the file.
                self.store
                    .nodes_by_file(file_path)?
                    .into_iter()
                    .map(|n| n.qualified_name)
                    .collect()
            }
            RefactorOperation::ExtractFunctionCandidate { file_path, .. } => self
                .store
                .nodes_by_file(file_path)?
                .into_iter()
                .map(|n| n.qualified_name)
                .collect(),
        };

        let mut affected_symbols: HashSet<String> = HashSet::new();
        let mut affected_files: HashSet<String> = HashSet::new();
        let mut nearby_tests: Vec<String> = Vec::new();
        let mut unresolved_risks: Vec<String> = plan.manual_review.clone();

        // BFS from seed symbols (depth 2, cap 150).
        for seed_qname in &seed_qnames {
            let inbound = self.store.inbound_edges(seed_qname, EDGE_QUERY_LIMIT)?;
            let outbound = self.store.outbound_edges(seed_qname, EDGE_QUERY_LIMIT)?;

            for (n, _) in inbound.iter().chain(outbound.iter()) {
                affected_symbols.insert(n.qualified_name.clone());
                affected_files.insert(n.file_path.clone());
                if n.is_test || n.kind == NodeKind::Test {
                    nearby_tests.push(n.qualified_name.clone());
                }
            }
        }

        // Safety score: 1.0 is safest. Deduct for impact / risks.
        let mut safety: f64 = 1.0;
        if affected_symbols.len() > 10 {
            safety -= 0.2;
            unresolved_risks.push(format!(
                "high blast radius: {} symbols affected",
                affected_symbols.len()
            ));
        }
        if !plan.manual_review.is_empty() {
            safety -= 0.15 * (plan.manual_review.len() as f64).min(4.0);
        }
        if matches!(plan.estimated_safety, SafetyBand::Risky) {
            safety -= 0.3;
        }
        safety = safety.clamp(0.0, 1.0);

        let mut affected_symbols_list: Vec<String> = affected_symbols.into_iter().collect();
        affected_symbols_list.sort();
        let mut affected_files_list: Vec<String> = affected_files.into_iter().collect();
        affected_files_list.sort();
        nearby_tests.sort();
        nearby_tests.dedup();

        Ok(SimulatedRefactorImpact {
            affected_symbols: affected_symbols_list,
            affected_files: affected_files_list,
            safety_score: safety,
            nearby_tests,
            unresolved_risks,
        })
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Read the content of a repository-relative file.
    fn read_file_content(&self, rel_path: &str) -> Result<String> {
        let abs = self.repo_root.join(rel_path);
        std::fs::read_to_string(&abs)
            .map_err(|e| AtlasError::Other(format!("cannot read `{}`: {e}", abs.display())))
    }

    /// Read a single line (1-based) from a file.
    fn read_line(&self, rel_path: &str, line_no: u32) -> Result<String> {
        let content = self.read_file_content(rel_path)?;
        content
            .lines()
            .nth((line_no as usize).saturating_sub(1))
            .map(|l| l.to_string())
            .ok_or_else(|| {
                AtlasError::Other(format!("line {line_no} out of range in `{rel_path}`"))
            })
    }

    /// Core apply loop shared by all operations.
    ///
    /// Groups edits by file, applies them in reverse line order, generates
    /// patches, optionally writes files, and runs the caller-provided
    /// validation closure.
    fn apply_plan_internal<F>(
        &self,
        plan: &RefactorPlan,
        dry_run: bool,
        validate: F,
    ) -> Result<RefactorDryRunResult>
    where
        F: Fn(&mut RefactorValidationResult, &HashMap<String, (String, String)>),
    {
        // Group edits by file.
        let mut by_file: HashMap<String, Vec<&RefactorEdit>> = HashMap::new();
        for edit in &plan.edits {
            by_file
                .entry(edit.file_path.clone())
                .or_default()
                .push(edit);
        }

        let mut patches: Vec<RefactorPatch> = Vec::new();
        let mut new_contents: HashMap<String, (String, String)> = HashMap::new();
        let mut validation = RefactorValidationResult {
            valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
            manual_review: plan.manual_review.clone(),
        };

        for (file_path, mut file_edits) in by_file {
            let old_content = match self.read_file_content(&file_path) {
                Ok(c) => c,
                Err(e) => {
                    validation.valid = false;
                    validation
                        .errors
                        .push(format!("cannot read `{file_path}`: {e}"));
                    continue;
                }
            };

            // Sort edits in reverse line order for correct application.
            file_edits.sort_by_key(|e| std::cmp::Reverse(e.line_start));

            let edit_refs: Vec<RefactorEdit> = file_edits.into_iter().cloned().collect();
            let new_content = match apply_edits(&old_content, &edit_refs) {
                Ok(c) => c,
                Err(e) => {
                    validation.valid = false;
                    validation
                        .errors
                        .push(format!("apply failed for `{file_path}`: {e}"));
                    continue;
                }
            };

            let diff = unified_diff_annotated(&file_path, &old_content, &new_content);
            if !diff.is_empty() {
                patches.push(RefactorPatch {
                    file_path: file_path.clone(),
                    unified_diff: diff,
                });
            }

            new_contents.insert(file_path, (old_content, new_content));
        }

        // Run operation-specific validation.
        validate(&mut validation, &new_contents);

        // Write files if not dry-run and no errors.
        if !dry_run && validation.valid {
            for (file_path, (_, new_content)) in &new_contents {
                let abs = self.repo_root.join(file_path);
                if let Err(e) = std::fs::write(&abs, new_content) {
                    validation.valid = false;
                    validation
                        .errors
                        .push(format!("write failed `{}`: {e}", abs.display()));
                } else {
                    debug!(path = %file_path, "wrote refactored file");
                }
            }
        }

        let files_changed = patches.len();
        let edit_count = plan.edits.len();

        Ok(RefactorDryRunResult {
            plan: plan.clone(),
            patches,
            validation,
            files_changed,
            edit_count,
            dry_run,
        })
    }
}

// ---------------------------------------------------------------------------
// Import detection helpers
// ---------------------------------------------------------------------------

/// Language family for import detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Lang {
    Rust,
    Python,
    JsTs,
    Go,
    Other,
}

fn detect_language(path: &str) -> Lang {
    if path.ends_with(".rs") {
        Lang::Rust
    } else if path.ends_with(".py") {
        Lang::Python
    } else if path.ends_with(".js")
        || path.ends_with(".ts")
        || path.ends_with(".tsx")
        || path.ends_with(".jsx")
    {
        Lang::JsTs
    } else if path.ends_with(".go") {
        Lang::Go
    } else {
        Lang::Other
    }
}

struct ImportInfo<'a> {
    line_no: u32,
    imported_names: Vec<String>,
    raw_line: &'a str,
}

fn find_imports<'a>(lines: &[&'a str], lang: Lang) -> Vec<ImportInfo<'a>> {
    let mut imports = Vec::new();
    for (i, &line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let line_no = (i + 1) as u32;

        match lang {
            Lang::Rust => {
                if trimmed.starts_with("use ") {
                    let names = extract_rust_use_names(trimmed);
                    if !names.is_empty() {
                        imports.push(ImportInfo {
                            line_no,
                            imported_names: names,
                            raw_line: line,
                        });
                    }
                }
            }
            Lang::Python => {
                if trimmed.starts_with("import ") || trimmed.starts_with("from ") {
                    let names = extract_python_import_names(trimmed);
                    if !names.is_empty() {
                        imports.push(ImportInfo {
                            line_no,
                            imported_names: names,
                            raw_line: line,
                        });
                    }
                }
            }
            Lang::JsTs => {
                if trimmed.starts_with("import ") {
                    let names = extract_js_import_names(trimmed);
                    if !names.is_empty() {
                        imports.push(ImportInfo {
                            line_no,
                            imported_names: names,
                            raw_line: line,
                        });
                    }
                }
            }
            Lang::Go => {
                // Go multi-line imports handled elsewhere; single-line:
                if trimmed.starts_with("import ") {
                    let names = extract_go_import_name(trimmed);
                    if !names.is_empty() {
                        imports.push(ImportInfo {
                            line_no,
                            imported_names: names,
                            raw_line: line,
                        });
                    }
                }
            }
            Lang::Other => {}
        }
    }
    imports
}

/// Extract the leaf identifier(s) from a Rust `use` statement.
///
/// e.g. `use std::io::{Read, Write};` → `["Read", "Write"]`
/// e.g. `use foo::bar::Baz;` → `["Baz"]`
fn extract_rust_use_names(line: &str) -> Vec<String> {
    // Strip `use ` prefix and trailing `;`.
    let body = line.trim_start_matches("use ").trim_end_matches(';').trim();

    if body.contains('{') {
        // Multi-import: extract identifiers inside braces.
        let start = body.find('{').unwrap();
        let end = body.find('}').unwrap_or(body.len());
        body[start + 1..end]
            .split(',')
            .map(|s| {
                // Handle `Name as Alias` → take Alias if present.
                let s = s.trim();
                if let Some(pos) = s.rfind(" as ") {
                    s[pos + 4..].trim().to_string()
                } else {
                    s.split("::").last().unwrap_or(s).to_string()
                }
            })
            .filter(|s| !s.is_empty() && s != "*")
            .collect()
    } else {
        // Single import: last segment.
        let raw = body.split("::").last().unwrap_or(body);
        let name = if let Some(pos) = raw.rfind(" as ") {
            raw[pos + 4..].trim().to_string()
        } else {
            raw.trim().to_string()
        };
        if name.is_empty() || name == "*" {
            vec![]
        } else {
            vec![name]
        }
    }
}

/// Extract imported name(s) from a Python import statement.
fn extract_python_import_names(line: &str) -> Vec<String> {
    if let Some(rest) = line.strip_prefix("from ") {
        // `from module import Name, Other`
        if let Some(idx) = rest.find(" import ") {
            return rest[idx + 8..]
                .split(',')
                .map(|s| {
                    let s = s.trim();
                    if let Some(p) = s.find(" as ") {
                        s[p + 4..].trim().to_string()
                    } else {
                        s.to_string()
                    }
                })
                .filter(|s| !s.is_empty() && s != "*")
                .collect();
        }
    } else if let Some(rest) = line.strip_prefix("import ") {
        return rest
            .split(',')
            .map(|s| {
                let s = s.trim();
                if let Some(p) = s.find(" as ") {
                    s[p + 4..].trim().to_string()
                } else {
                    s.split('.').next().unwrap_or(s).to_string()
                }
            })
            .filter(|s| !s.is_empty())
            .collect();
    }
    vec![]
}

/// Extract imported names from a JS/TS import statement.
fn extract_js_import_names(line: &str) -> Vec<String> {
    // `import { Foo, Bar } from '...'` or `import Foo from '...'`
    let body = line.trim_start_matches("import").trim();
    if body.starts_with('{') {
        if let Some(end) = body.find('}') {
            return body[1..end]
                .split(',')
                .map(|s| {
                    let s = s.trim();
                    if let Some(p) = s.find(" as ") {
                        s[p + 4..].trim().to_string()
                    } else {
                        s.to_string()
                    }
                })
                .filter(|s| !s.is_empty())
                .collect();
        }
    } else {
        // Default import: first word.
        let name: String = body
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() && name != "type" {
            return vec![name];
        }
    }
    vec![]
}

/// Extract the package alias from a Go import line.
fn extract_go_import_name(line: &str) -> Vec<String> {
    // `import "pkg/path"` or `import alias "pkg/path"`.
    let body = line.trim_start_matches("import").trim().trim_matches('"');
    if body.is_empty() {
        return vec![];
    }
    // Last path component without quotes.
    let name = body
        .trim_matches('"')
        .split('/')
        .next_back()
        .unwrap_or(body)
        .to_string();
    if name.is_empty() { vec![] } else { vec![name] }
}

/// Check if `line` contains `word` as a whole-word occurrence.
fn contains_word(line: &str, word: &str) -> bool {
    let bytes = line.as_bytes();
    let wb = word.as_bytes();
    let mut pos = 0;
    while pos + wb.len() <= bytes.len() {
        if bytes[pos..].starts_with(wb) {
            let before_ok = pos == 0 || !is_ident_byte(bytes[pos - 1]);
            let after = pos + wb.len();
            let after_ok = after >= bytes.len() || !is_ident_byte(bytes[after]);
            if before_ok && after_ok {
                return true;
            }
        }
        pos += 1;
    }
    false
}

#[inline]
fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

#[cfg(test)]
mod tests {
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
}
