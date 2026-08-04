use super::*;

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
