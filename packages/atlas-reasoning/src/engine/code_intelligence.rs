use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::Path;

use atlas_core::{
    ArchitectureReport, AtlasError, EdgeKind, InsightEvidence, InsightFinding, InsightLineRange,
    InsightSeverity, Node, NodeKind, PatternReport, Result,
};
use atlas_repo::CanonicalRepoPath;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::InsightsEngine;
use super::metrics::{GraphSnapshot, build_node_metrics, load_rust_complexity, module_id_for_file};

const SIMILARITY_HIGH_THRESHOLD: f64 = 0.72;
const SIMILARITY_MEDIUM_THRESHOLD: f64 = 0.55;
const SIMILARITY_LOW_THRESHOLD: f64 = 0.40;

const SIMILAR_SHINGLE_SIZE: usize = 3;
const DUPLICATE_SHINGLE_SIZE: usize = 4;
const MAX_SIMILAR_CANDIDATES_PER_SOURCE: usize = 256;
const FINGERPRINT_CACHE_VERSION: u32 = 1;
const FINGERPRINT_CACHE_FILE: &str = "insights-fingerprint-cache.v1.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsightSymbolSummary {
    pub qualified_name: String,
    pub display_name: String,
    pub file_path: String,
    pub line_start: u32,
    pub line_end: u32,
    pub language: String,
    pub node_kind: String,
    pub module_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimilarityThresholds {
    pub high: f64,
    pub medium: f64,
    pub low: f64,
}

impl Default for SimilarityThresholds {
    fn default() -> Self {
        Self {
            high: SIMILARITY_HIGH_THRESHOLD,
            medium: SIMILARITY_MEDIUM_THRESHOLD,
            low: SIMILARITY_LOW_THRESHOLD,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimilarFunctionRequest {
    pub symbol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_score: Option<f64>,
    #[serde(default)]
    pub include_same_file: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimilarFunctionMatch {
    pub source: InsightSymbolSummary,
    pub candidate: InsightSymbolSummary,
    pub score: f64,
    pub score_band: String,
    pub matched_features: Vec<String>,
    pub differing_features: Vec<String>,
    pub feature_scores: BTreeMap<String, f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimilarFunctionReportResult {
    pub source: InsightSymbolSummary,
    pub thresholds: SimilarityThresholds,
    #[serde(flatten)]
    pub report: PatternReport,
    pub matches: Vec<SimilarFunctionMatch>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimilarFunctionAnalysis {
    pub request: SimilarFunctionRequest,
    pub source: InsightSymbolSummary,
    pub thresholds: SimilarityThresholds,
    pub report: PatternReport,
    pub matches: Vec<SimilarFunctionMatch>,
}

impl SimilarFunctionAnalysis {
    pub fn report_result(&self) -> SimilarFunctionReportResult {
        SimilarFunctionReportResult {
            source: self.source.clone(),
            thresholds: self.thresholds.clone(),
            report: self.report.clone(),
            matches: self.matches.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DuplicateDetectionRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_score: Option<f64>,
    #[serde(default)]
    pub include_tests: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suppressions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SourceSpan {
    pub file_path: String,
    pub line_start: u32,
    pub line_end: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DuplicateMember {
    pub symbol: InsightSymbolSummary,
    pub source_span: SourceSpan,
    pub normalized_token_count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DuplicateGroup {
    pub group_id: String,
    pub duplicate_kind: String,
    pub confidence: f64,
    pub normalized_pattern_summary: String,
    pub duplicated_line_count: usize,
    pub duplicated_token_count: usize,
    pub member_count: usize,
    pub files: Vec<String>,
    pub members: Vec<DuplicateMember>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_extraction_target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DuplicateDetectionReportResult {
    pub thresholds: SimilarityThresholds,
    #[serde(flatten)]
    pub report: PatternReport,
    pub groups: Vec<DuplicateGroup>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DuplicateDetectionAnalysis {
    pub request: DuplicateDetectionRequest,
    pub thresholds: SimilarityThresholds,
    pub report: PatternReport,
    pub groups: Vec<DuplicateGroup>,
}

impl DuplicateDetectionAnalysis {
    pub fn report_result(&self) -> DuplicateDetectionReportResult {
        DuplicateDetectionReportResult {
            thresholds: self.thresholds.clone(),
            report: self.report.clone(),
            groups: self.groups.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InferredModule {
    pub module_id: String,
    pub display_name: String,
    pub root_paths: Vec<String>,
    pub owned_symbols: Vec<String>,
    pub node_count: usize,
    pub file_count: usize,
    pub outbound_dependencies: Vec<String>,
    pub inbound_dependencies: Vec<String>,
    pub confidence: f64,
    pub evidence: Vec<String>,
    pub explicit: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InferredModuleReportResult {
    #[serde(flatten)]
    pub report: ArchitectureReport,
    pub modules: Vec<InferredModule>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModuleInferenceAnalysis {
    pub report: ArchitectureReport,
    pub modules: Vec<InferredModule>,
}

impl ModuleInferenceAnalysis {
    pub fn report_result(&self) -> InferredModuleReportResult {
        InferredModuleReportResult {
            report: self.report.clone(),
            modules: self.modules.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentLabelMatch {
    pub label: String,
    pub confidence: f64,
    pub evidence: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentLabelAssignment {
    pub file_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qualified_name: Option<String>,
    pub labels: Vec<ComponentLabelMatch>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentLabelRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbols: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentLabelReportResult {
    #[serde(flatten)]
    pub report: PatternReport,
    pub assignments: Vec<ComponentLabelAssignment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComponentLabelAnalysis {
    pub request: ComponentLabelRequest,
    pub report: PatternReport,
    pub assignments: Vec<ComponentLabelAssignment>,
}

impl ComponentLabelAnalysis {
    pub fn report_result(&self) -> ComponentLabelReportResult {
        ComponentLabelReportResult {
            report: self.report.clone(),
            assignments: self.assignments.clone(),
        }
    }
}

#[derive(Debug, Clone)]
struct CallableFingerprint {
    summary: InsightSymbolSummary,
    arity: usize,
    name_tokens: BTreeSet<String>,
    signature_tokens: BTreeSet<String>,
    body_shingles: BTreeSet<String>,
    duplicate_shingles: BTreeSet<String>,
    duplicate_signature: String,
    duplicate_summary: String,
    neighbor_names: BTreeSet<String>,
    loc: usize,
}

#[derive(Debug, Clone)]
struct FileModuleAssignment {
    module_id: String,
    display_name: String,
    explicit: bool,
    confidence: f64,
    evidence: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedFingerprintCache {
    version: u32,
    files: BTreeMap<String, PersistedFingerprintFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedFingerprintFile {
    file_hash: String,
    callables: BTreeMap<String, PersistedCallableFingerprint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedCallableFingerprint {
    arity: usize,
    loc: usize,
    name_tokens: Vec<String>,
    signature_tokens: Vec<String>,
    body_shingles: Vec<String>,
    duplicate_shingles: Vec<String>,
    duplicate_signature: String,
    duplicate_summary: String,
}

impl PersistedFingerprintCache {
    fn empty() -> Self {
        Self {
            version: FINGERPRINT_CACHE_VERSION,
            files: BTreeMap::new(),
        }
    }
}

impl PersistedCallableFingerprint {
    fn from_runtime(fingerprint: &CallableFingerprint) -> Self {
        Self {
            arity: fingerprint.arity,
            loc: fingerprint.loc,
            name_tokens: fingerprint.name_tokens.iter().cloned().collect(),
            signature_tokens: fingerprint.signature_tokens.iter().cloned().collect(),
            body_shingles: fingerprint.body_shingles.iter().cloned().collect(),
            duplicate_shingles: fingerprint.duplicate_shingles.iter().cloned().collect(),
            duplicate_signature: fingerprint.duplicate_signature.clone(),
            duplicate_summary: fingerprint.duplicate_summary.clone(),
        }
    }

    fn to_runtime(
        &self,
        summary: InsightSymbolSummary,
        neighbor_names: BTreeSet<String>,
    ) -> CallableFingerprint {
        CallableFingerprint {
            summary,
            arity: self.arity,
            name_tokens: self.name_tokens.iter().cloned().collect(),
            signature_tokens: self.signature_tokens.iter().cloned().collect(),
            body_shingles: self.body_shingles.iter().cloned().collect(),
            duplicate_shingles: self.duplicate_shingles.iter().cloned().collect(),
            duplicate_signature: self.duplicate_signature.clone(),
            duplicate_summary: self.duplicate_summary.clone(),
            neighbor_names,
            loc: self.loc,
        }
    }
}

impl<'s> InsightsEngine<'s> {
    pub fn find_similar_functions(
        &self,
        repo_root: impl AsRef<Path>,
        request: SimilarFunctionRequest,
    ) -> Result<SimilarFunctionAnalysis> {
        let store = self.store().ok_or_else(|| {
            AtlasError::Other(
                "similar-function analysis requires a store-backed insights engine".to_owned(),
            )
        })?;
        let snapshot = self.load_graph_snapshot(store, repo_root.as_ref())?;
        let rust_complexity = load_rust_complexity(repo_root.as_ref(), &snapshot.nodes)?;
        let node_metrics = build_node_metrics(self, &snapshot, &rust_complexity);
        let module_by_qname = node_metrics
            .iter()
            .map(|metric| (metric.node.qualified_name.clone(), metric.module_id.clone()))
            .collect::<HashMap<_, _>>();
        let fingerprints = callable_fingerprints(repo_root.as_ref(), &snapshot, &module_by_qname)?;
        let thresholds = similarity_thresholds(self.config());
        let min_score = request.min_score.unwrap_or(thresholds.low);
        let limit = request.limit.unwrap_or(self.config().max_findings);

        let Some(source) = resolve_callable_source(&request.symbol, &fingerprints) else {
            let source = unresolved_symbol_summary(&request.symbol);
            let report = self.pattern_report(vec![InsightFinding {
                id: format!("similar_function:unresolved:{}", request.symbol),
                title: format!("unresolved callable {}", request.symbol),
                severity: InsightSeverity::Low,
                category: "similar_functions".to_owned(),
                message: format!(
                    "could not resolve callable symbol `{}` in current graph",
                    request.symbol
                ),
                evidence: Vec::new(),
                ranking_reason: "callable symbol did not resolve in current graph snapshot"
                    .to_owned(),
                details: Some(json!({ "symbol": request.symbol })),
                score: 0.0,
            }]);
            return Ok(SimilarFunctionAnalysis {
                request,
                source,
                thresholds,
                report,
                matches: Vec::new(),
            });
        };

        let matches = rank_similar_function_matches(
            source,
            &fingerprints,
            min_score,
            limit,
            request.include_same_file,
            &thresholds,
        );
        let findings = matches
            .iter()
            .map(similar_match_to_finding)
            .collect::<Vec<_>>();
        let report = self.pattern_report(findings);
        let retained_ids = retained_finding_ids(&report);
        let matches = matches
            .into_iter()
            .filter(|item| retained_ids.contains(&similar_match_id(item)))
            .collect::<Vec<_>>();
        let source = source.summary.clone();

        Ok(SimilarFunctionAnalysis {
            request,
            source,
            thresholds,
            report,
            matches,
        })
    }

    pub fn find_duplicates(
        &self,
        repo_root: impl AsRef<Path>,
        request: DuplicateDetectionRequest,
    ) -> Result<DuplicateDetectionAnalysis> {
        let store = self.store().ok_or_else(|| {
            AtlasError::Other(
                "duplicate detection requires a store-backed insights engine".to_owned(),
            )
        })?;
        let snapshot = self.load_graph_snapshot(store, repo_root.as_ref())?;
        let rust_complexity = load_rust_complexity(repo_root.as_ref(), &snapshot.nodes)?;
        let node_metrics = build_node_metrics(self, &snapshot, &rust_complexity);
        let module_by_qname = node_metrics
            .iter()
            .map(|metric| (metric.node.qualified_name.clone(), metric.module_id.clone()))
            .collect::<HashMap<_, _>>();
        let mut fingerprints =
            callable_fingerprints(repo_root.as_ref(), &snapshot, &module_by_qname)?;
        let file_scope = normalize_paths(request.files.as_deref())?
            .into_iter()
            .collect::<BTreeSet<_>>();
        fingerprints.retain(|item| {
            (request.include_tests || !item.summary.file_path.starts_with("tests/"))
                && (file_scope.is_empty() || file_scope.contains(&item.summary.file_path))
        });

        let thresholds = duplicate_thresholds(self.config());
        let min_score = request.min_score.unwrap_or(thresholds.low);
        let limit = request.limit.unwrap_or(self.config().max_findings);
        let suppressions = duplicate_suppressions(self.config(), &request);
        let groups = detect_duplicate_groups(&fingerprints, min_score, limit)
            .into_iter()
            .filter(|group| !duplicate_group_suppressed(group, &suppressions))
            .collect::<Vec<_>>();
        let findings = groups
            .iter()
            .map(|group| duplicate_group_to_finding(group, &thresholds))
            .collect::<Vec<_>>();
        let report = self.pattern_report(findings);
        let retained_ids = retained_finding_ids(&report);
        let groups = groups
            .into_iter()
            .filter(|group| retained_ids.contains(&duplicate_group_id(group)))
            .collect::<Vec<_>>();

        Ok(DuplicateDetectionAnalysis {
            request,
            thresholds,
            report,
            groups,
        })
    }

    pub fn infer_modules(&self, repo_root: impl AsRef<Path>) -> Result<ModuleInferenceAnalysis> {
        let store = self.store().ok_or_else(|| {
            AtlasError::Other("module inference requires a store-backed insights engine".to_owned())
        })?;
        let snapshot = self.load_graph_snapshot(store, repo_root.as_ref())?;
        let file_assignments = infer_file_modules(store, &snapshot)?;
        let modules = build_inferred_modules(&snapshot, &file_assignments);
        let findings = modules
            .iter()
            .filter_map(module_to_finding)
            .collect::<Vec<_>>();
        let report = self.architecture_report(findings);
        Ok(ModuleInferenceAnalysis { report, modules })
    }

    pub fn label_components(
        &self,
        repo_root: impl AsRef<Path>,
        request: ComponentLabelRequest,
    ) -> Result<ComponentLabelAnalysis> {
        let store = self.store().ok_or_else(|| {
            AtlasError::Other(
                "component labeling requires a store-backed insights engine".to_owned(),
            )
        })?;
        let snapshot = self.load_graph_snapshot(store, repo_root.as_ref())?;
        let explicit_files = normalize_paths(request.files.as_deref())?
            .into_iter()
            .collect::<BTreeSet<_>>();
        let explicit_symbols = request
            .symbols
            .clone()
            .unwrap_or_default()
            .into_iter()
            .collect::<BTreeSet<_>>();
        let limit = request.limit.unwrap_or(self.config().max_findings);

        let mut assignments = Vec::new();
        let mut file_paths = snapshot
            .owner_by_file
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        file_paths.extend(explicit_files.iter().cloned());
        for file_path in file_paths {
            if !explicit_files.is_empty() && !explicit_files.contains(&file_path) {
                continue;
            }
            let labels = labels_for_file(&file_path);
            if labels.is_empty() {
                continue;
            }
            assignments.push(ComponentLabelAssignment {
                file_path: file_path.clone(),
                qualified_name: None,
                labels,
            });
        }

        for node in &snapshot.nodes {
            if !explicit_symbols.is_empty() && !explicit_symbols.contains(&node.qualified_name) {
                continue;
            }
            if !explicit_files.is_empty() && !explicit_files.contains(&node.file_path) {
                continue;
            }
            let labels = labels_for_symbol(node);
            if labels.is_empty() {
                continue;
            }
            assignments.push(ComponentLabelAssignment {
                file_path: node.file_path.clone(),
                qualified_name: Some(node.qualified_name.clone()),
                labels,
            });
        }

        assignments.sort_by(|left, right| {
            left.file_path
                .cmp(&right.file_path)
                .then_with(|| left.qualified_name.cmp(&right.qualified_name))
        });
        if assignments.len() > limit {
            assignments.truncate(limit);
        }

        let findings = assignments
            .iter()
            .filter_map(component_assignment_to_finding)
            .collect::<Vec<_>>();
        let report = self.pattern_report(findings);

        Ok(ComponentLabelAnalysis {
            request,
            report,
            assignments,
        })
    }
}

fn retained_finding_ids<T>(report: &T) -> HashSet<String>
where
    T: ReportFindingView,
{
    report
        .findings()
        .iter()
        .map(|finding| finding.id.clone())
        .collect()
}

trait ReportFindingView {
    fn findings(&self) -> &[InsightFinding];
}

impl ReportFindingView for PatternReport {
    fn findings(&self) -> &[InsightFinding] {
        &self.findings
    }
}

impl ReportFindingView for ArchitectureReport {
    fn findings(&self) -> &[InsightFinding] {
        &self.findings
    }
}

fn resolve_callable_source<'a>(
    symbol: &str,
    fingerprints: &'a [CallableFingerprint],
) -> Option<&'a CallableFingerprint> {
    let normalized = super::helpers::normalize_qn_kind_tokens(symbol);
    fingerprints.iter().find(|item| {
        item.summary.qualified_name == normalized || item.summary.display_name == symbol
    })
}

fn unresolved_symbol_summary(symbol: &str) -> InsightSymbolSummary {
    InsightSymbolSummary {
        qualified_name: symbol.to_owned(),
        display_name: symbol.rsplit("::").next().unwrap_or(symbol).to_owned(),
        file_path: String::new(),
        line_start: 0,
        line_end: 0,
        language: String::new(),
        node_kind: "function".to_owned(),
        module_id: String::new(),
    }
}

fn callable_fingerprints(
    repo_root: &Path,
    snapshot: &GraphSnapshot,
    module_by_qname: &HashMap<String, String>,
) -> Result<Vec<CallableFingerprint>> {
    let node_by_qname = snapshot
        .nodes
        .iter()
        .map(|node| (node.qualified_name.clone(), node))
        .collect::<HashMap<_, _>>();
    let mut outbound = HashMap::<String, BTreeSet<String>>::new();
    for edge in &snapshot.edges {
        if !matches!(
            edge.kind,
            EdgeKind::Calls | EdgeKind::References | EdgeKind::Imports
        ) {
            continue;
        }
        let target_name = node_by_qname
            .get(&edge.target_qn)
            .map(|node| node.name.clone())
            .unwrap_or_else(|| {
                edge.target_qn
                    .rsplit("::")
                    .next()
                    .unwrap_or(&edge.target_qn)
                    .to_owned()
            });
        outbound
            .entry(edge.source_qn.clone())
            .or_default()
            .insert(target_name);
    }

    let callable_nodes_by_file = snapshot
        .nodes
        .iter()
        .filter(|node| is_callable_node(node))
        .fold(BTreeMap::<String, Vec<&Node>>::new(), |mut acc, node| {
            acc.entry(node.file_path.clone()).or_default().push(node);
            acc
        });
    let mut cache = load_persisted_fingerprint_cache(repo_root);
    let mut updated_cache_files = BTreeMap::new();
    let mut fingerprints = Vec::new();
    for (file_path, nodes) in callable_nodes_by_file {
        if let Some((cached, persisted)) = restore_cached_file_fingerprints(
            cache.files.get(&file_path),
            snapshot
                .file_hash_by_file
                .get(&file_path)
                .map(String::as_str),
            &nodes,
            module_by_qname,
            &mut outbound,
        ) {
            fingerprints.extend(cached);
            updated_cache_files.insert(file_path, persisted);
            continue;
        }

        let source = fs::read_to_string(repo_root.join(&file_path)).unwrap_or_default();
        let built = build_file_fingerprints(&source, &nodes, module_by_qname, &mut outbound);
        let file_hash = snapshot
            .file_hash_by_file
            .get(&file_path)
            .cloned()
            .unwrap_or_default();
        updated_cache_files.insert(file_path, persisted_fingerprint_file(&file_hash, &built));
        fingerprints.extend(built);
    }

    cache.version = FINGERPRINT_CACHE_VERSION;
    cache.files = updated_cache_files;
    persist_fingerprint_cache(repo_root, &cache);

    fingerprints.sort_by(|left, right| {
        left.summary
            .file_path
            .cmp(&right.summary.file_path)
            .then_with(|| left.summary.line_start.cmp(&right.summary.line_start))
            .then_with(|| {
                left.summary
                    .qualified_name
                    .cmp(&right.summary.qualified_name)
            })
    });
    Ok(fingerprints)
}

fn restore_cached_file_fingerprints(
    cached: Option<&PersistedFingerprintFile>,
    file_hash: Option<&str>,
    nodes: &[&Node],
    module_by_qname: &HashMap<String, String>,
    outbound: &mut HashMap<String, BTreeSet<String>>,
) -> Option<(Vec<CallableFingerprint>, PersistedFingerprintFile)> {
    let cached = cached?;
    let file_hash = file_hash?;
    if file_hash.is_empty() || cached.file_hash != file_hash {
        return None;
    }
    let current_qnames = nodes
        .iter()
        .map(|node| node.qualified_name.as_str())
        .collect::<BTreeSet<_>>();
    let cached_qnames = cached
        .callables
        .keys()
        .map(|qname| qname.as_str())
        .collect::<BTreeSet<_>>();
    if current_qnames != cached_qnames {
        return None;
    }

    let mut fingerprints = Vec::with_capacity(nodes.len());
    for node in nodes {
        let persisted = cached.callables.get(&node.qualified_name)?;
        let module_id = module_by_qname
            .get(&node.qualified_name)
            .cloned()
            .unwrap_or_else(|| "module:<unknown>".to_owned());
        let summary = symbol_summary(node, module_id);
        let neighbor_names = outbound.remove(&node.qualified_name).unwrap_or_default();
        fingerprints.push(persisted.to_runtime(summary, neighbor_names));
    }
    Some((fingerprints, cached.clone()))
}

fn build_file_fingerprints(
    source: &str,
    nodes: &[&Node],
    module_by_qname: &HashMap<String, String>,
    outbound: &mut HashMap<String, BTreeSet<String>>,
) -> Vec<CallableFingerprint> {
    nodes
        .iter()
        .map(|node| {
            let module_id = module_by_qname
                .get(&node.qualified_name)
                .cloned()
                .unwrap_or_else(|| "module:<unknown>".to_owned());
            let summary = symbol_summary(node, module_id);
            let name_tokens = tokenize_identifier(&node.name);
            let signature_tokens = signature_tokens(node);
            let source_excerpt = source_excerpt_from_text(source, node).unwrap_or_default();
            let body_tokens = tokenize_source(&source_excerpt);
            let body_shingles = shingles(&body_tokens, SIMILAR_SHINGLE_SIZE);
            let duplicate_tokens = normalize_duplicate_tokens(&source_excerpt);
            let duplicate_shingles = shingles(&duplicate_tokens, DUPLICATE_SHINGLE_SIZE);
            let duplicate_signature = duplicate_tokens.join(" ");
            let duplicate_summary = summarize_duplicate_pattern(&duplicate_tokens);
            let neighbor_names = outbound.remove(&node.qualified_name).unwrap_or_default();
            CallableFingerprint {
                arity: parse_arity(node.params.as_deref()),
                loc: node.line_end.saturating_sub(node.line_start) as usize + 1,
                summary,
                name_tokens,
                signature_tokens,
                body_shingles,
                duplicate_shingles,
                duplicate_signature,
                duplicate_summary,
                neighbor_names,
            }
        })
        .collect()
}

fn persisted_fingerprint_file(
    file_hash: &str,
    fingerprints: &[CallableFingerprint],
) -> PersistedFingerprintFile {
    let callables = fingerprints
        .iter()
        .map(|fingerprint| {
            (
                fingerprint.summary.qualified_name.clone(),
                PersistedCallableFingerprint::from_runtime(fingerprint),
            )
        })
        .collect::<BTreeMap<_, _>>();
    PersistedFingerprintFile {
        file_hash: file_hash.to_owned(),
        callables,
    }
}

fn load_persisted_fingerprint_cache(repo_root: &Path) -> PersistedFingerprintCache {
    let cache_path = fingerprint_cache_path(repo_root);
    let Ok(raw) = fs::read_to_string(cache_path) else {
        return PersistedFingerprintCache::empty();
    };
    let Ok(cache) = serde_json::from_str::<PersistedFingerprintCache>(&raw) else {
        return PersistedFingerprintCache::empty();
    };
    if cache.version == FINGERPRINT_CACHE_VERSION {
        cache
    } else {
        PersistedFingerprintCache::empty()
    }
}

fn persist_fingerprint_cache(repo_root: &Path, cache: &PersistedFingerprintCache) {
    let cache_path = fingerprint_cache_path(repo_root);
    let Some(parent) = cache_path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(payload) = serde_json::to_vec_pretty(cache) else {
        return;
    };
    let tmp_path = cache_path.with_extension("json.tmp");
    if fs::write(&tmp_path, payload).is_err() {
        return;
    }
    let _ = fs::rename(tmp_path, cache_path);
}

fn fingerprint_cache_path(repo_root: &Path) -> std::path::PathBuf {
    repo_root
        .join(atlas_engine::paths::ATLAS_DIR)
        .join(FINGERPRINT_CACHE_FILE)
}

fn rank_similar_function_matches(
    source: &CallableFingerprint,
    fingerprints: &[CallableFingerprint],
    min_score: f64,
    limit: usize,
    include_same_file: bool,
    thresholds: &SimilarityThresholds,
) -> Vec<SimilarFunctionMatch> {
    let mut candidates = fingerprints
        .iter()
        .filter(|candidate| candidate.summary.qualified_name != source.summary.qualified_name)
        .filter(|candidate| candidate.summary.language == source.summary.language)
        .filter(|candidate| candidate.summary.node_kind == source.summary.node_kind)
        .filter(|candidate| {
            include_same_file || candidate.summary.file_path != source.summary.file_path
        })
        .filter(|candidate| source.arity.abs_diff(candidate.arity) <= 1)
        .take(MAX_SIMILAR_CANDIDATES_PER_SOURCE)
        .filter_map(|candidate| build_similar_match(source, candidate, min_score, thresholds))
        .collect::<Vec<_>>();

    candidates.sort_by(|left, right| {
        right
            .score
            .total_cmp(&left.score)
            .then_with(|| left.candidate.file_path.cmp(&right.candidate.file_path))
            .then_with(|| left.candidate.line_start.cmp(&right.candidate.line_start))
            .then_with(|| {
                left.candidate
                    .qualified_name
                    .cmp(&right.candidate.qualified_name)
            })
    });
    if candidates.len() > limit {
        candidates.truncate(limit);
    }
    candidates
}

fn build_similar_match(
    source: &CallableFingerprint,
    candidate: &CallableFingerprint,
    min_score: f64,
    thresholds: &SimilarityThresholds,
) -> Option<SimilarFunctionMatch> {
    let name_overlap = jaccard(&source.name_tokens, &candidate.name_tokens);
    let signature_overlap = jaccard(&source.signature_tokens, &candidate.signature_tokens);
    let body_overlap = jaccard(&source.body_shingles, &candidate.body_shingles).max(jaccard(
        &source.duplicate_shingles,
        &candidate.duplicate_shingles,
    ));
    let neighbor_overlap = jaccard(&source.neighbor_names, &candidate.neighbor_names);
    let module_overlap = if source.summary.module_id == candidate.summary.module_id {
        1.0
    } else {
        0.0
    };
    let size_overlap = overlap_ratio(source.loc, candidate.loc);

    let score = (name_overlap * 0.18)
        + (signature_overlap * 0.20)
        + (body_overlap * 0.34)
        + (neighbor_overlap * 0.18)
        + (module_overlap * 0.05)
        + (size_overlap * 0.05);
    if score < min_score {
        return None;
    }
    if body_overlap < 0.15 && neighbor_overlap < 0.15 && signature_overlap < 0.30 {
        return None;
    }

    let feature_scores = BTreeMap::from([
        ("body_overlap".to_owned(), body_overlap),
        ("module_overlap".to_owned(), module_overlap),
        ("name_overlap".to_owned(), name_overlap),
        ("neighbor_overlap".to_owned(), neighbor_overlap),
        ("signature_overlap".to_owned(), signature_overlap),
        ("size_overlap".to_owned(), size_overlap),
    ]);
    let mut matched_features = Vec::new();
    let mut differing_features = Vec::new();
    for (name, value) in &feature_scores {
        if *value >= 0.5 {
            matched_features.push(name.clone());
        } else if *value <= 0.15 {
            differing_features.push(name.clone());
        }
    }

    Some(SimilarFunctionMatch {
        source: source.summary.clone(),
        candidate: candidate.summary.clone(),
        score,
        score_band: similarity_band(score, thresholds).to_owned(),
        matched_features,
        differing_features,
        feature_scores,
    })
}

fn detect_duplicate_groups(
    fingerprints: &[CallableFingerprint],
    min_score: f64,
    limit: usize,
) -> Vec<DuplicateGroup> {
    let mut exact_groups = BTreeMap::<String, Vec<&CallableFingerprint>>::new();
    for item in fingerprints {
        if item.duplicate_signature.split_whitespace().count() < 6 {
            continue;
        }
        exact_groups
            .entry(item.duplicate_signature.clone())
            .or_default()
            .push(item);
    }

    let mut groups = Vec::new();
    let mut consumed = HashSet::<String>::new();
    for members in exact_groups.values() {
        if members.len() < 2 {
            continue;
        }
        let group = build_duplicate_group("exact_normalized", 1.0, members);
        for member in members {
            consumed.insert(member.summary.qualified_name.clone());
        }
        groups.push(group);
    }

    let candidates = fingerprints
        .iter()
        .filter(|item| !consumed.contains(&item.summary.qualified_name))
        .collect::<Vec<_>>();
    let mut visited = HashSet::<String>::new();
    for (index, seed) in candidates.iter().enumerate() {
        if visited.contains(&seed.summary.qualified_name) {
            continue;
        }
        let mut cluster = vec![*seed];
        let mut cluster_score = 0.0_f64;
        visited.insert(seed.summary.qualified_name.clone());
        for other in candidates.iter().skip(index + 1) {
            if visited.contains(&other.summary.qualified_name)
                || other.summary.language != seed.summary.language
                || other.summary.node_kind != seed.summary.node_kind
            {
                continue;
            }
            if overlap_ratio(seed.loc, other.loc) < 0.55 {
                continue;
            }
            let score = jaccard(&seed.duplicate_shingles, &other.duplicate_shingles);
            if score < min_score {
                continue;
            }
            cluster.push(*other);
            cluster_score = cluster_score.max(score);
            visited.insert(other.summary.qualified_name.clone());
        }
        if cluster.len() >= 2 {
            groups.push(build_duplicate_group(
                "near_duplicate",
                cluster_score.max(min_score),
                &cluster,
            ));
        }
    }

    groups.sort_by(|left, right| {
        right
            .confidence
            .total_cmp(&left.confidence)
            .then_with(|| {
                right
                    .duplicated_token_count
                    .cmp(&left.duplicated_token_count)
            })
            .then_with(|| left.group_id.cmp(&right.group_id))
    });
    if groups.len() > limit {
        groups.truncate(limit);
    }
    groups
}

fn build_duplicate_group(
    duplicate_kind: &str,
    confidence: f64,
    members: &[&CallableFingerprint],
) -> DuplicateGroup {
    let mut sorted_members = members
        .iter()
        .map(|item| (*item).clone())
        .collect::<Vec<_>>();
    sorted_members.sort_by(|left, right| {
        left.summary
            .file_path
            .cmp(&right.summary.file_path)
            .then_with(|| left.summary.line_start.cmp(&right.summary.line_start))
            .then_with(|| {
                left.summary
                    .qualified_name
                    .cmp(&right.summary.qualified_name)
            })
    });
    let first = &sorted_members[0];
    let files = sorted_members
        .iter()
        .map(|item| item.summary.file_path.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let duplicated_line_count = sorted_members
        .iter()
        .map(|item| item.loc)
        .min()
        .unwrap_or_default();
    let duplicated_token_count = sorted_members
        .iter()
        .map(|item| item.duplicate_signature.split_whitespace().count())
        .min()
        .unwrap_or_default();
    let member_summaries = sorted_members
        .iter()
        .map(|item| DuplicateMember {
            source_span: SourceSpan {
                file_path: item.summary.file_path.clone(),
                line_start: item.summary.line_start,
                line_end: item.summary.line_end,
            },
            symbol: item.summary.clone(),
            normalized_token_count: item.duplicate_signature.split_whitespace().count(),
        })
        .collect::<Vec<_>>();
    let suggested_extraction_target = common_parent_qname(&sorted_members)
        .or_else(|| common_parent_path(&files))
        .or_else(|| files.first().cloned());
    DuplicateGroup {
        group_id: format!(
            "duplicate:{}:{}:{}:{}",
            duplicate_kind,
            first.summary.file_path,
            first.summary.line_start,
            sorted_members.len()
        ),
        duplicate_kind: duplicate_kind.to_owned(),
        confidence,
        normalized_pattern_summary: first.duplicate_summary.clone(),
        duplicated_line_count,
        duplicated_token_count,
        member_count: sorted_members.len(),
        files,
        members: member_summaries,
        suggested_extraction_target,
    }
}

fn infer_file_modules(
    store: &atlas_store_sqlite::Store,
    snapshot: &GraphSnapshot,
) -> Result<BTreeMap<String, FileModuleAssignment>> {
    let community_by_qname = community_assignments(store)?;
    let nodes_by_file =
        snapshot
            .nodes
            .iter()
            .fold(BTreeMap::<String, Vec<&Node>>::new(), |mut acc, node| {
                acc.entry(node.file_path.clone()).or_default().push(node);
                acc
            });
    let mut assignments = BTreeMap::new();
    for file_path in snapshot.owner_by_file.keys() {
        let owner_id = store.file_owner_id(file_path)?;
        let assignment = if let Some(owner_id) = owner_id {
            FileModuleAssignment {
                module_id: owner_id.clone(),
                display_name: owner_id.clone(),
                explicit: true,
                confidence: 1.0,
                evidence: vec![format!("package owner `{owner_id}`")],
            }
        } else if let Some(assignment) = infer_community_module(
            nodes_by_file
                .get(file_path)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            &community_by_qname,
        ) {
            assignment
        } else {
            infer_path_module(file_path)
        };
        assignments.insert(file_path.clone(), assignment);
    }
    Ok(assignments)
}

fn community_assignments(
    store: &atlas_store_sqlite::Store,
) -> Result<HashMap<String, (String, String)>> {
    let mut by_qname = HashMap::new();
    for community in store.list_communities()? {
        let display = community.name.clone();
        let module_id = format!("community:{}", community.id);
        for member in store.get_community_nodes(community.id)? {
            by_qname
                .entry(member.node_qualified_name)
                .or_insert_with(|| (module_id.clone(), display.clone()));
        }
    }
    Ok(by_qname)
}

fn infer_community_module(
    nodes: &[&Node],
    community_by_qname: &HashMap<String, (String, String)>,
) -> Option<FileModuleAssignment> {
    let mut counts = BTreeMap::<(String, String), usize>::new();
    for node in nodes {
        if let Some((module_id, display_name)) = community_by_qname.get(&node.qualified_name) {
            *counts
                .entry((module_id.clone(), display_name.clone()))
                .or_default() += 1;
        }
    }
    let ((module_id, display_name), count) = counts
        .into_iter()
        .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0)))?;
    if count == 0 {
        return None;
    }
    Some(FileModuleAssignment {
        module_id: module_id.clone(),
        display_name: display_name.clone(),
        explicit: false,
        confidence: 0.88,
        evidence: vec![format!(
            "graph community `{display_name}` matched {count} file symbol(s)"
        )],
    })
}

fn infer_path_module(file_path: &str) -> FileModuleAssignment {
    if let Some(rest) = file_path.strip_prefix("packages/")
        && let Some((package, _)) = rest.split_once('/')
    {
        return FileModuleAssignment {
            module_id: format!("infer:packages/{package}"),
            display_name: format!("packages/{package}"),
            explicit: false,
            confidence: 0.95,
            evidence: vec!["package directory prefix".to_owned()],
        };
    }
    if let Some(rest) = file_path.strip_prefix("src/") {
        if let Some((segment, _)) = rest.split_once('/') {
            return FileModuleAssignment {
                module_id: format!("infer:src/{segment}"),
                display_name: format!("src/{segment}"),
                explicit: false,
                confidence: 0.82,
                evidence: vec!["top-level src segment".to_owned()],
            };
        }
        return FileModuleAssignment {
            module_id: "infer:src".to_owned(),
            display_name: "src".to_owned(),
            explicit: false,
            confidence: 0.78,
            evidence: vec!["src root file".to_owned()],
        };
    }
    if file_path.starts_with("tests/") {
        return FileModuleAssignment {
            module_id: "infer:tests".to_owned(),
            display_name: "tests".to_owned(),
            explicit: false,
            confidence: 0.80,
            evidence: vec!["test directory prefix".to_owned()],
        };
    }
    if file_path.starts_with("docs/")
        || file_path.starts_with("wiki/")
        || file_path.ends_with(".md")
    {
        return FileModuleAssignment {
            module_id: "infer:docs".to_owned(),
            display_name: "docs".to_owned(),
            explicit: false,
            confidence: 0.76,
            evidence: vec!["docs path pattern".to_owned()],
        };
    }
    let display_name = file_path
        .rsplit_once('/')
        .map(|(prefix, _)| prefix.to_owned())
        .unwrap_or_else(|| "<root>".to_owned());
    FileModuleAssignment {
        module_id: format!("infer:{display_name}"),
        display_name,
        explicit: false,
        confidence: 0.62,
        evidence: vec!["parent directory fallback".to_owned()],
    }
}

fn build_inferred_modules(
    snapshot: &GraphSnapshot,
    assignments: &BTreeMap<String, FileModuleAssignment>,
) -> Vec<InferredModule> {
    let node_to_module = snapshot
        .nodes
        .iter()
        .map(|node| {
            let assignment = assignments
                .get(&node.file_path)
                .cloned()
                .unwrap_or_else(|| infer_path_module(&node.file_path));
            (node.qualified_name.clone(), assignment.module_id)
        })
        .collect::<HashMap<_, _>>();
    let mut files_by_module = BTreeMap::<String, BTreeSet<String>>::new();
    let mut qnames_by_module = BTreeMap::<String, BTreeSet<String>>::new();
    let mut evidence_by_module = BTreeMap::<String, BTreeSet<String>>::new();
    let mut confidence_by_module = BTreeMap::<String, f64>::new();
    let mut display_by_module = BTreeMap::<String, String>::new();
    let mut explicit_by_module = BTreeMap::<String, bool>::new();
    for (file_path, assignment) in assignments {
        files_by_module
            .entry(assignment.module_id.clone())
            .or_default()
            .insert(file_path.clone());
        evidence_by_module
            .entry(assignment.module_id.clone())
            .or_default()
            .extend(assignment.evidence.iter().cloned());
        confidence_by_module
            .entry(assignment.module_id.clone())
            .and_modify(|score| *score = score.max(assignment.confidence))
            .or_insert(assignment.confidence);
        display_by_module
            .entry(assignment.module_id.clone())
            .or_insert_with(|| assignment.display_name.clone());
        explicit_by_module
            .entry(assignment.module_id.clone())
            .and_modify(|value| *value |= assignment.explicit)
            .or_insert(assignment.explicit);
    }
    for node in &snapshot.nodes {
        let module_id = node_to_module
            .get(&node.qualified_name)
            .cloned()
            .unwrap_or_else(|| module_id_for_file(&node.file_path, None));
        qnames_by_module
            .entry(module_id)
            .or_default()
            .insert(node.qualified_name.clone());
    }

    let mut outbound_by_module = BTreeMap::<String, BTreeSet<String>>::new();
    let mut inbound_by_module = BTreeMap::<String, BTreeSet<String>>::new();
    for edge in &snapshot.edges {
        let Some(source_module) = node_to_module.get(&edge.source_qn) else {
            continue;
        };
        let Some(target_module) = node_to_module.get(&edge.target_qn) else {
            continue;
        };
        if source_module == target_module {
            continue;
        }
        outbound_by_module
            .entry(source_module.clone())
            .or_default()
            .insert(target_module.clone());
        inbound_by_module
            .entry(target_module.clone())
            .or_default()
            .insert(source_module.clone());
    }

    let mut modules = files_by_module
        .into_iter()
        .map(|(module_id, file_paths)| {
            let owned_symbols = qnames_by_module
                .remove(&module_id)
                .unwrap_or_default()
                .into_iter()
                .collect::<Vec<_>>();
            InferredModule {
                display_name: display_by_module
                    .get(&module_id)
                    .cloned()
                    .unwrap_or_else(|| module_id.clone()),
                root_paths: file_paths.iter().cloned().collect(),
                node_count: owned_symbols.len(),
                owned_symbols,
                file_count: file_paths.len(),
                outbound_dependencies: outbound_by_module
                    .remove(&module_id)
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
                inbound_dependencies: inbound_by_module
                    .remove(&module_id)
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
                confidence: confidence_by_module.get(&module_id).copied().unwrap_or(0.5),
                evidence: evidence_by_module
                    .remove(&module_id)
                    .unwrap_or_default()
                    .into_iter()
                    .collect(),
                explicit: explicit_by_module.get(&module_id).copied().unwrap_or(false),
                module_id,
            }
        })
        .collect::<Vec<_>>();

    modules.sort_by(|left, right| left.module_id.cmp(&right.module_id));
    modules
}

fn labels_for_file(file_path: &str) -> Vec<ComponentLabelMatch> {
    let mut labels = Vec::new();
    add_component_label(
        &mut labels,
        file_path.starts_with("packages/atlas-cli/"),
        "cli",
        1.0,
        "file path under packages/atlas-cli",
    );
    add_component_label(
        &mut labels,
        file_path.starts_with("packages/atlas-mcp/") || file_path == "MCP_TOOLS.md",
        "mcp",
        1.0,
        "file path under packages/atlas-mcp or MCP docs",
    );
    add_component_label(
        &mut labels,
        file_path.starts_with("packages/atlas-repo/"),
        "repo_scan",
        0.96,
        "file path under packages/atlas-repo",
    );
    add_component_label(
        &mut labels,
        file_path.starts_with("packages/atlas-parser/"),
        "parse",
        0.98,
        "file path under packages/atlas-parser",
    );
    add_component_label(
        &mut labels,
        file_path.starts_with("packages/atlas-store-sqlite/")
            || file_path.starts_with("packages/atlas-db-utils/"),
        "persist_graph",
        0.94,
        "store/db package path",
    );
    add_component_label(
        &mut labels,
        file_path.starts_with("packages/atlas-engine/")
            || file_path.contains("/update.rs")
            || file_path.contains("/watch.rs")
            || file_path.contains("postprocess"),
        "incremental_update",
        0.86,
        "engine/update/watch path",
    );
    add_component_label(
        &mut labels,
        file_path.starts_with("packages/atlas-search/")
            || file_path.contains("query")
            || file_path.contains("traverse"),
        "search_traverse",
        0.84,
        "search/query/traverse path",
    );
    add_component_label(
        &mut labels,
        file_path.starts_with("packages/atlas-review/")
            || file_path.contains("review")
            || file_path.ends_with("changes.rs")
            || file_path.ends_with("context_cmd.rs"),
        "review_context",
        0.88,
        "review/context path",
    );
    add_component_label(
        &mut labels,
        file_path.starts_with("packages/atlas-contextsave/")
            || file_path.starts_with("packages/atlas-contentstore/"),
        "context_memory",
        0.94,
        "content/context storage path",
    );
    add_component_label(
        &mut labels,
        file_path.starts_with("packages/atlas-session/")
            || file_path.starts_with("packages/atlas-agent-events/")
            || file_path.contains("session")
            || file_path.contains("wake_up"),
        "session_continuity",
        0.90,
        "session memory path",
    );
    add_component_label(
        &mut labels,
        file_path.starts_with("schemas/")
            || file_path.starts_with(".atlas/")
            || file_path.ends_with(".toml")
            || file_path.ends_with(".yaml")
            || file_path.ends_with(".yml")
            || file_path.ends_with(".json"),
        "config",
        0.82,
        "config/schema extension or directory",
    );
    add_component_label(
        &mut labels,
        file_path.contains("health")
            || file_path.contains("doctor")
            || file_path.contains("db_check")
            || file_path.contains("debug_graph")
            || file_path.contains("status"),
        "diagnostics",
        0.85,
        "health/doctor/status path",
    );
    add_component_label(
        &mut labels,
        file_path.starts_with("tests/")
            || file_path.contains("/tests/")
            || file_path.ends_with("_test.rs")
            || file_path.ends_with("tests.rs"),
        "tests",
        0.97,
        "test path pattern",
    );
    add_component_label(
        &mut labels,
        file_path.starts_with("docs/")
            || file_path.starts_with("wiki/")
            || file_path.ends_with(".md"),
        "docs",
        0.95,
        "docs/wiki/markdown path",
    );
    labels.sort_by(|left, right| left.label.cmp(&right.label));
    labels
}

fn labels_for_symbol(node: &Node) -> Vec<ComponentLabelMatch> {
    let mut labels = labels_for_file(&node.file_path);
    if node.name.contains("doctor") || node.name.contains("status") {
        add_component_label(
            &mut labels,
            true,
            "diagnostics",
            0.78,
            "symbol name suggests diagnostics/health surface",
        );
    }
    labels.sort_by(|left, right| left.label.cmp(&right.label));
    labels.dedup_by(|left, right| left.label == right.label);
    labels
}

fn add_component_label(
    labels: &mut Vec<ComponentLabelMatch>,
    predicate: bool,
    label: &str,
    confidence: f64,
    evidence: &str,
) {
    if !predicate {
        return;
    }
    if let Some(existing) = labels.iter_mut().find(|item| item.label == label) {
        existing.confidence = existing.confidence.max(confidence);
        if !existing.evidence.iter().any(|item| item == evidence) {
            existing.evidence.push(evidence.to_owned());
        }
        return;
    }
    labels.push(ComponentLabelMatch {
        label: label.to_owned(),
        confidence,
        evidence: vec![evidence.to_owned()],
    });
}

fn component_assignment_to_finding(
    assignment: &ComponentLabelAssignment,
) -> Option<InsightFinding> {
    let best = assignment
        .labels
        .iter()
        .min_by(|left, right| left.confidence.total_cmp(&right.confidence))?;
    if best.confidence >= 0.75 {
        return None;
    }
    Some(InsightFinding {
        id: format!(
            "component_label:{}:{}",
            assignment.file_path,
            assignment
                .qualified_name
                .clone()
                .unwrap_or_else(|| "<file>".to_owned())
        ),
        title: format!(
            "low-confidence component labels for {}",
            assignment.file_path
        ),
        severity: InsightSeverity::Low,
        category: "component_labels".to_owned(),
        message: format!(
            "{} label confidence {:.2} stays below 0.75",
            best.label, best.confidence
        ),
        evidence: vec![InsightEvidence {
            file_path: Some(assignment.file_path.clone()),
            qualified_name: assignment.qualified_name.clone(),
            node_kind: None,
            edge_kind: None,
            line_range: None,
            confidence_tier: None,
        }],
        ranking_reason: "label confidence below deterministic threshold".to_owned(),
        details: Some(json!({
            "labels": assignment.labels,
        })),
        score: (1.0 - best.confidence) * 100.0,
    })
}

fn module_to_finding(module: &InferredModule) -> Option<InsightFinding> {
    if module.confidence >= 0.75 {
        return None;
    }
    Some(InsightFinding {
        id: format!("infer_module:{}", module.module_id),
        title: format!("low-confidence inferred module {}", module.display_name),
        severity: InsightSeverity::Low,
        category: "inferred_modules".to_owned(),
        message: format!(
            "module {} inferred with confidence {:.2}",
            module.display_name, module.confidence
        ),
        evidence: module
            .root_paths
            .iter()
            .take(3)
            .map(|file_path| InsightEvidence {
                file_path: Some(file_path.clone()),
                qualified_name: None,
                node_kind: None,
                edge_kind: None,
                line_range: None,
                confidence_tier: None,
            })
            .collect(),
        ranking_reason: "heuristic-only module inference without explicit package owner".to_owned(),
        details: Some(json!({
            "module_id": module.module_id,
            "evidence": module.evidence,
            "outbound_dependencies": module.outbound_dependencies,
            "inbound_dependencies": module.inbound_dependencies,
        })),
        score: (1.0 - module.confidence) * 100.0,
    })
}

fn similar_match_id(item: &SimilarFunctionMatch) -> String {
    format!(
        "similar_function:{}:{}",
        item.source.qualified_name, item.candidate.qualified_name
    )
}

fn similar_match_to_finding(item: &SimilarFunctionMatch) -> InsightFinding {
    InsightFinding {
        id: similar_match_id(item),
        title: format!(
            "{} similar to {}",
            item.source.display_name, item.candidate.display_name
        ),
        severity: match item.score_band.as_str() {
            "high" => InsightSeverity::High,
            "medium" => InsightSeverity::Medium,
            _ => InsightSeverity::Low,
        },
        category: "similar_functions".to_owned(),
        message: format!(
            "{} and {} scored {:.2} similarity",
            item.source.qualified_name, item.candidate.qualified_name, item.score
        ),
        evidence: vec![
            symbol_evidence(&item.source),
            symbol_evidence(&item.candidate),
        ],
        ranking_reason: format!(
            "body/signature/name overlap produced {} similarity band",
            item.score_band
        ),
        details: Some(json!({
            "source": item.source,
            "candidate": item.candidate,
            "matched_features": item.matched_features,
            "differing_features": item.differing_features,
            "feature_scores": item.feature_scores,
        })),
        score: item.score * 100.0,
    }
}

fn duplicate_group_id(group: &DuplicateGroup) -> String {
    group.group_id.clone()
}

fn duplicate_group_to_finding(
    group: &DuplicateGroup,
    thresholds: &SimilarityThresholds,
) -> InsightFinding {
    InsightFinding {
        id: group.group_id.clone(),
        title: format!("{} duplicate group", group.duplicate_kind.replace('_', " ")),
        severity: duplicate_severity(group.confidence, thresholds),
        category: "duplicate_detection".to_owned(),
        message: format!(
            "{} members share {} duplicate pattern with confidence {:.2}",
            group.member_count, group.duplicate_kind, group.confidence
        ),
        evidence: group
            .members
            .iter()
            .map(|member| symbol_evidence(&member.symbol))
            .collect(),
        ranking_reason: format!(
            "confidence {:.2}, duplicated tokens {}, duplicated lines {}",
            group.confidence, group.duplicated_token_count, group.duplicated_line_count
        ),
        details: Some(json!({
            "duplicate_kind": group.duplicate_kind,
            "normalized_pattern_summary": group.normalized_pattern_summary,
            "files": group.files,
            "members": group.members,
            "suggested_extraction_target": group.suggested_extraction_target,
        })),
        score: group.confidence * 100.0,
    }
}

fn duplicate_severity(confidence: f64, thresholds: &SimilarityThresholds) -> InsightSeverity {
    if confidence >= thresholds.high {
        InsightSeverity::High
    } else if confidence >= thresholds.medium {
        InsightSeverity::Medium
    } else {
        InsightSeverity::Low
    }
}

fn similarity_thresholds(config: &atlas_engine::config::InsightsConfig) -> SimilarityThresholds {
    SimilarityThresholds {
        high: config.similarity_high_threshold,
        medium: config.similarity_medium_threshold,
        low: config.similarity_low_threshold,
    }
}

fn duplicate_thresholds(config: &atlas_engine::config::InsightsConfig) -> SimilarityThresholds {
    SimilarityThresholds {
        high: config.duplicate_high_threshold,
        medium: config.duplicate_medium_threshold,
        low: config.duplicate_low_threshold,
    }
}

fn duplicate_suppressions(
    config: &atlas_engine::config::InsightsConfig,
    request: &DuplicateDetectionRequest,
) -> BTreeSet<String> {
    config
        .duplicate_suppressions
        .iter()
        .chain(request.suppressions.iter())
        .map(|item| item.trim().to_owned())
        .filter(|item| !item.is_empty())
        .collect()
}

fn duplicate_group_suppressed(group: &DuplicateGroup, suppressions: &BTreeSet<String>) -> bool {
    suppressions.iter().any(|pattern| {
        group.group_id.contains(pattern)
            || group.normalized_pattern_summary.contains(pattern)
            || group
                .files
                .iter()
                .any(|file| file == pattern || file.starts_with(pattern))
            || group.members.iter().any(|member| {
                member.symbol.qualified_name.contains(pattern)
                    || member.symbol.file_path == *pattern
                    || member.symbol.file_path.starts_with(pattern)
            })
    })
}

fn symbol_evidence(summary: &InsightSymbolSummary) -> InsightEvidence {
    InsightEvidence {
        file_path: Some(summary.file_path.clone()),
        qualified_name: Some(summary.qualified_name.clone()),
        node_kind: Some(summary.node_kind.clone()),
        edge_kind: None,
        line_range: Some(InsightLineRange {
            start_line: summary.line_start,
            end_line: summary.line_end,
        }),
        confidence_tier: None,
    }
}

fn similarity_band(score: f64, thresholds: &SimilarityThresholds) -> &'static str {
    if score >= thresholds.high {
        "high"
    } else if score >= thresholds.medium {
        "medium"
    } else {
        "low"
    }
}

fn symbol_summary(node: &Node, module_id: String) -> InsightSymbolSummary {
    InsightSymbolSummary {
        qualified_name: node.qualified_name.clone(),
        display_name: node.name.clone(),
        file_path: node.file_path.clone(),
        line_start: node.line_start,
        line_end: node.line_end,
        language: node.language.clone(),
        node_kind: node.kind.as_str().to_owned(),
        module_id,
    }
}

fn is_callable_node(node: &Node) -> bool {
    matches!(node.kind, NodeKind::Function | NodeKind::Method)
}

fn parse_arity(params: Option<&str>) -> usize {
    let Some(params) = params.map(str::trim) else {
        return 0;
    };
    let inner = params
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(params)
        .trim();
    if inner.is_empty() {
        0
    } else {
        inner.split(',').count()
    }
}

fn signature_tokens(node: &Node) -> BTreeSet<String> {
    let mut tokens = tokenize_identifier(&node.name);
    for source in [
        node.params.as_deref(),
        node.return_type.as_deref(),
        node.modifiers.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        tokens.extend(tokenize_source(source));
    }
    tokens
}

fn source_excerpt_from_text(source: &str, node: &Node) -> Option<String> {
    let start = usize::try_from(node.line_start.saturating_sub(1)).ok()?;
    let end = usize::try_from(node.line_end).ok()?;
    let lines = source.lines().skip(start).take(end.saturating_sub(start));
    Some(lines.collect::<Vec<_>>().join("\n"))
}

fn tokenize_identifier(text: &str) -> BTreeSet<String> {
    let mut normalized = String::with_capacity(text.len() * 2);
    let mut previous_is_lower = false;
    for ch in text.chars() {
        if ch.is_ascii_uppercase() && previous_is_lower {
            normalized.push(' ');
        }
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch.to_ascii_lowercase());
        } else {
            normalized.push(' ');
        }
        previous_is_lower = ch.is_ascii_lowercase();
    }
    normalized
        .split_whitespace()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
}

fn tokenize_source(text: &str) -> Vec<String> {
    let mut normalized = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            normalized.push(ch.to_ascii_lowercase());
        } else {
            normalized.push(' ');
        }
    }
    normalized
        .split_whitespace()
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect()
}

fn normalize_duplicate_tokens(text: &str) -> Vec<String> {
    tokenize_source(text)
        .into_iter()
        .map(|token| {
            if token.chars().all(|ch| ch.is_ascii_digit()) {
                "<num>".to_owned()
            } else if is_keyword(&token) {
                token
            } else {
                "<id>".to_owned()
            }
        })
        .collect()
}

fn shingles(tokens: &[String], size: usize) -> BTreeSet<String> {
    if tokens.is_empty() {
        return BTreeSet::new();
    }
    if tokens.len() <= size {
        return [tokens.join(" ")].into_iter().collect();
    }
    let mut items = BTreeSet::new();
    for window in tokens.windows(size) {
        items.insert(window.join(" "));
    }
    items
}

fn jaccard(left: &BTreeSet<String>, right: &BTreeSet<String>) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(right).count() as f64;
    let union = left.union(right).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

fn overlap_ratio(left: usize, right: usize) -> f64 {
    if left == 0 || right == 0 {
        return 0.0;
    }
    let min = left.min(right) as f64;
    let max = left.max(right) as f64;
    min / max
}

fn summarize_duplicate_pattern(tokens: &[String]) -> String {
    tokens
        .iter()
        .take(12)
        .cloned()
        .collect::<Vec<_>>()
        .join(" ")
}

fn common_parent_qname(items: &[CallableFingerprint]) -> Option<String> {
    let mut parts = items
        .iter()
        .filter_map(|item| item.summary.qualified_name.rsplit_once("::"))
        .map(|(parent, _)| parent.to_owned());
    let first = parts.next()?;
    if parts.all(|item| item == first) {
        Some(first)
    } else {
        None
    }
}

fn common_parent_path(files: &[String]) -> Option<String> {
    let mut parts = files
        .iter()
        .filter_map(|file| file.rsplit_once('/').map(|(prefix, _)| prefix));
    let first = parts.next()?.to_owned();
    if parts.all(|item| item == first) {
        Some(first)
    } else {
        None
    }
}

fn normalize_paths(paths: Option<&[String]>) -> Result<Vec<String>> {
    paths
        .map(|paths| {
            paths
                .iter()
                .map(|path| {
                    CanonicalRepoPath::from_repo_relative(path)
                        .map(|canonical| canonical.as_str().to_owned())
                        .map_err(|error| AtlasError::Other(error.to_string()))
                })
                .collect()
        })
        .unwrap_or_else(|| Ok(Vec::new()))
}

fn is_keyword(token: &str) -> bool {
    matches!(
        token,
        "if" | "else"
            | "for"
            | "while"
            | "loop"
            | "match"
            | "return"
            | "let"
            | "const"
            | "fn"
            | "pub"
            | "impl"
            | "struct"
            | "enum"
            | "trait"
            | "class"
            | "interface"
            | "switch"
            | "case"
            | "break"
            | "continue"
            | "try"
            | "catch"
            | "throw"
            | "await"
            | "async"
            | "new"
            | "use"
            | "import"
            | "export"
            | "mod"
            | "where"
            | "in"
            | "true"
            | "false"
            | "none"
            | "some"
            | "ok"
            | "err"
    )
}
