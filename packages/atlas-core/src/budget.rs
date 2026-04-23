use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetNamespace {
    BuildUpdate,
    GraphTraversal,
    QueryCandidatesAndSeeds,
    ReviewContextExtraction,
    ContentSavedContextLookup,
    McpCliPayloadSerialization,
}

impl BudgetNamespace {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::BuildUpdate => "build_update",
            Self::GraphTraversal => "graph_traversal",
            Self::QueryCandidatesAndSeeds => "query_candidates_and_seeds",
            Self::ReviewContextExtraction => "review_context_extraction",
            Self::ContentSavedContextLookup => "content_saved_context_lookup",
            Self::McpCliPayloadSerialization => "mcp_cli_payload_serialization",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetHitBehavior {
    Clamp,
    Partial,
    FailClosed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetStatus {
    WithinBudget,
    OverrideClamped,
    PartialResult,
    Blocked,
}

impl BudgetStatus {
    fn severity(self) -> u8 {
        match self {
            Self::WithinBudget => 0,
            Self::OverrideClamped => 1,
            Self::PartialResult => 2,
            Self::Blocked => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetStage {
    Build,
    Update,
    Query,
    Impact,
    ReviewContext,
    MinimalContext,
    SavedContextRetrieval,
    McpResponseSerialization,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetOutcomeClass {
    HardError,
    SoftPartial,
    DegradedState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetStageRule {
    pub stage: BudgetStage,
    pub budget_name: &'static str,
    pub outcome: BudgetOutcomeClass,
    pub safe_to_answer: bool,
}

pub fn budget_stage_rules() -> &'static [BudgetStageRule] {
    const RULES: &[BudgetStageRule] = &[
        BudgetStageRule {
            stage: BudgetStage::Build,
            budget_name: "build_update.max_files_per_run",
            outcome: BudgetOutcomeClass::DegradedState,
            safe_to_answer: false,
        },
        BudgetStageRule {
            stage: BudgetStage::Build,
            budget_name: "build_update.max_total_bytes_per_run",
            outcome: BudgetOutcomeClass::DegradedState,
            safe_to_answer: false,
        },
        BudgetStageRule {
            stage: BudgetStage::Build,
            budget_name: "build_update.max_file_bytes",
            outcome: BudgetOutcomeClass::DegradedState,
            safe_to_answer: false,
        },
        BudgetStageRule {
            stage: BudgetStage::Build,
            budget_name: "build_update.max_parse_failures",
            outcome: BudgetOutcomeClass::HardError,
            safe_to_answer: false,
        },
        BudgetStageRule {
            stage: BudgetStage::Build,
            budget_name: "build_update.max_parse_failure_ratio",
            outcome: BudgetOutcomeClass::HardError,
            safe_to_answer: false,
        },
        BudgetStageRule {
            stage: BudgetStage::Build,
            budget_name: "build_update.max_wall_time_ms",
            outcome: BudgetOutcomeClass::DegradedState,
            safe_to_answer: false,
        },
        BudgetStageRule {
            stage: BudgetStage::Update,
            budget_name: "build_update.max_files_per_run",
            outcome: BudgetOutcomeClass::DegradedState,
            safe_to_answer: false,
        },
        BudgetStageRule {
            stage: BudgetStage::Update,
            budget_name: "build_update.max_total_bytes_per_run",
            outcome: BudgetOutcomeClass::DegradedState,
            safe_to_answer: false,
        },
        BudgetStageRule {
            stage: BudgetStage::Update,
            budget_name: "build_update.max_file_bytes",
            outcome: BudgetOutcomeClass::DegradedState,
            safe_to_answer: false,
        },
        BudgetStageRule {
            stage: BudgetStage::Update,
            budget_name: "build_update.max_parse_failures",
            outcome: BudgetOutcomeClass::HardError,
            safe_to_answer: false,
        },
        BudgetStageRule {
            stage: BudgetStage::Update,
            budget_name: "build_update.max_parse_failure_ratio",
            outcome: BudgetOutcomeClass::HardError,
            safe_to_answer: false,
        },
        BudgetStageRule {
            stage: BudgetStage::Update,
            budget_name: "build_update.max_wall_time_ms",
            outcome: BudgetOutcomeClass::DegradedState,
            safe_to_answer: false,
        },
        BudgetStageRule {
            stage: BudgetStage::Query,
            budget_name: "query_candidates_and_seeds.max_candidates",
            outcome: BudgetOutcomeClass::SoftPartial,
            safe_to_answer: true,
        },
        BudgetStageRule {
            stage: BudgetStage::Query,
            budget_name: "query_candidates_and_seeds.max_query_wall_time_ms",
            outcome: BudgetOutcomeClass::SoftPartial,
            safe_to_answer: true,
        },
        BudgetStageRule {
            stage: BudgetStage::Impact,
            budget_name: "graph_traversal.max_seed_nodes",
            outcome: BudgetOutcomeClass::SoftPartial,
            safe_to_answer: true,
        },
        BudgetStageRule {
            stage: BudgetStage::Impact,
            budget_name: "query_candidates_and_seeds.symbol_resolution",
            outcome: BudgetOutcomeClass::HardError,
            safe_to_answer: false,
        },
        BudgetStageRule {
            stage: BudgetStage::Impact,
            budget_name: "graph_traversal.max_depth",
            outcome: BudgetOutcomeClass::SoftPartial,
            safe_to_answer: true,
        },
        BudgetStageRule {
            stage: BudgetStage::Impact,
            budget_name: "graph_traversal.max_nodes",
            outcome: BudgetOutcomeClass::SoftPartial,
            safe_to_answer: true,
        },
        BudgetStageRule {
            stage: BudgetStage::Impact,
            budget_name: "graph_traversal.max_edges",
            outcome: BudgetOutcomeClass::SoftPartial,
            safe_to_answer: true,
        },
        BudgetStageRule {
            stage: BudgetStage::ReviewContext,
            budget_name: "graph_traversal.max_seed_files",
            outcome: BudgetOutcomeClass::SoftPartial,
            safe_to_answer: true,
        },
        BudgetStageRule {
            stage: BudgetStage::ReviewContext,
            budget_name: "review_context_extraction.max_nodes",
            outcome: BudgetOutcomeClass::SoftPartial,
            safe_to_answer: true,
        },
        BudgetStageRule {
            stage: BudgetStage::ReviewContext,
            budget_name: "review_context_extraction.max_edges",
            outcome: BudgetOutcomeClass::SoftPartial,
            safe_to_answer: true,
        },
        BudgetStageRule {
            stage: BudgetStage::ReviewContext,
            budget_name: "review_context_extraction.max_files",
            outcome: BudgetOutcomeClass::SoftPartial,
            safe_to_answer: true,
        },
        BudgetStageRule {
            stage: BudgetStage::ReviewContext,
            budget_name: "mcp_cli_payload_serialization.max_context_payload_bytes",
            outcome: BudgetOutcomeClass::SoftPartial,
            safe_to_answer: true,
        },
        BudgetStageRule {
            stage: BudgetStage::ReviewContext,
            budget_name: "mcp_cli_payload_serialization.max_context_tokens_estimate",
            outcome: BudgetOutcomeClass::SoftPartial,
            safe_to_answer: true,
        },
        BudgetStageRule {
            stage: BudgetStage::MinimalContext,
            budget_name: "graph_traversal.max_depth",
            outcome: BudgetOutcomeClass::SoftPartial,
            safe_to_answer: true,
        },
        BudgetStageRule {
            stage: BudgetStage::MinimalContext,
            budget_name: "graph_traversal.max_nodes",
            outcome: BudgetOutcomeClass::SoftPartial,
            safe_to_answer: true,
        },
        BudgetStageRule {
            stage: BudgetStage::MinimalContext,
            budget_name: "graph_traversal.max_edges",
            outcome: BudgetOutcomeClass::SoftPartial,
            safe_to_answer: true,
        },
        BudgetStageRule {
            stage: BudgetStage::SavedContextRetrieval,
            budget_name: "content_saved_context_lookup.max_sources",
            outcome: BudgetOutcomeClass::SoftPartial,
            safe_to_answer: true,
        },
        BudgetStageRule {
            stage: BudgetStage::McpResponseSerialization,
            budget_name: "mcp_cli_payload_serialization.max_mcp_response_bytes",
            outcome: BudgetOutcomeClass::SoftPartial,
            safe_to_answer: true,
        },
    ];

    RULES
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetLimitRule {
    pub default_limit: usize,
    pub max_limit: usize,
    pub hit_behavior: BudgetHitBehavior,
    pub safe_to_answer_on_hit: bool,
}

impl BudgetLimitRule {
    pub const fn new(
        default_limit: usize,
        max_limit: usize,
        hit_behavior: BudgetHitBehavior,
        safe_to_answer_on_hit: bool,
    ) -> Self {
        Self {
            default_limit,
            max_limit,
            hit_behavior,
            safe_to_answer_on_hit,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetReport {
    pub budget_status: BudgetStatus,
    pub budget_hit: bool,
    pub budget_name: String,
    pub budget_limit: usize,
    pub budget_observed: usize,
    pub partial: bool,
    pub safe_to_answer: bool,
}

impl BudgetReport {
    pub fn within_budget(name: impl Into<String>, limit: usize, observed: usize) -> Self {
        Self {
            budget_status: BudgetStatus::WithinBudget,
            budget_hit: false,
            budget_name: name.into(),
            budget_limit: limit,
            budget_observed: observed,
            partial: false,
            safe_to_answer: true,
        }
    }

    pub fn not_applicable() -> Self {
        Self::within_budget("none", 0, 0)
    }

    pub fn override_clamped(name: impl Into<String>, limit: usize, observed: usize) -> Self {
        Self {
            budget_status: BudgetStatus::OverrideClamped,
            budget_hit: true,
            budget_name: name.into(),
            budget_limit: limit,
            budget_observed: observed,
            partial: false,
            safe_to_answer: true,
        }
    }

    pub fn partial_result(
        name: impl Into<String>,
        limit: usize,
        observed: usize,
        safe_to_answer: bool,
    ) -> Self {
        Self {
            budget_status: BudgetStatus::PartialResult,
            budget_hit: true,
            budget_name: name.into(),
            budget_limit: limit,
            budget_observed: observed,
            partial: true,
            safe_to_answer,
        }
    }

    pub fn blocked(name: impl Into<String>, limit: usize, observed: usize) -> Self {
        Self {
            budget_status: BudgetStatus::Blocked,
            budget_hit: true,
            budget_name: name.into(),
            budget_limit: limit,
            budget_observed: observed,
            partial: false,
            safe_to_answer: false,
        }
    }

    pub fn is_worse_than(&self, other: &Self) -> bool {
        self.budget_status.severity() > other.budget_status.severity()
            || (self.budget_status.severity() == other.budget_status.severity()
                && self.budget_observed > other.budget_observed)
    }

    pub fn merge(self, other: Self) -> Self {
        if other.is_worse_than(&self) {
            other
        } else {
            self
        }
    }
}

impl Default for BudgetReport {
    fn default() -> Self {
        Self::not_applicable()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BuildUpdateBudgetCounters {
    pub files_discovered: usize,
    pub files_accepted: usize,
    pub files_skipped_by_byte_budget: usize,
    pub bytes_accepted: u64,
    pub bytes_skipped: u64,
    pub parse_failures: usize,
    pub budget_stop_reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildUpdateBudgetPolicy {
    pub files_per_run: BudgetLimitRule,
    pub total_bytes_per_run: BudgetLimitRule,
    pub file_bytes: BudgetLimitRule,
    pub parse_failures: BudgetLimitRule,
    pub parse_failure_ratio_bps: BudgetLimitRule,
    pub wall_time_ms: BudgetLimitRule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphTraversalBudgetPolicy {
    pub seed_nodes: BudgetLimitRule,
    pub seed_files: BudgetLimitRule,
    pub depth: BudgetLimitRule,
    pub nodes: BudgetLimitRule,
    pub edges: BudgetLimitRule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryBudgetPolicy {
    pub candidates: BudgetLimitRule,
    pub seeds: BudgetLimitRule,
    pub wall_time_ms: BudgetLimitRule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewContextBudgetPolicy {
    pub nodes: BudgetLimitRule,
    pub edges: BudgetLimitRule,
    pub files: BudgetLimitRule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentLookupBudgetPolicy {
    pub sources: BudgetLimitRule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PayloadSerializationBudgetPolicy {
    pub nodes: BudgetLimitRule,
    pub edges: BudgetLimitRule,
    pub bytes: BudgetLimitRule,
    pub review_source_bytes: BudgetLimitRule,
    pub context_payload_bytes: BudgetLimitRule,
    pub context_tokens_estimate: BudgetLimitRule,
    pub file_excerpt_bytes: BudgetLimitRule,
    pub saved_context_bytes: BudgetLimitRule,
    pub mcp_response_bytes: BudgetLimitRule,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetPolicy {
    pub build_update: BuildUpdateBudgetPolicy,
    pub graph_traversal: GraphTraversalBudgetPolicy,
    pub query_candidates_and_seeds: QueryBudgetPolicy,
    pub review_context_extraction: ReviewContextBudgetPolicy,
    pub content_saved_context_lookup: ContentLookupBudgetPolicy,
    pub mcp_cli_payload_serialization: PayloadSerializationBudgetPolicy,
}

impl Default for BudgetPolicy {
    fn default() -> Self {
        Self {
            build_update: BuildUpdateBudgetPolicy {
                files_per_run: BudgetLimitRule::new(
                    10_000,
                    50_000,
                    BudgetHitBehavior::Partial,
                    false,
                ),
                total_bytes_per_run: BudgetLimitRule::new(
                    64 * 1024 * 1024,
                    512 * 1024 * 1024,
                    BudgetHitBehavior::Partial,
                    false,
                ),
                file_bytes: BudgetLimitRule::new(
                    10 * 1024 * 1024,
                    64 * 1024 * 1024,
                    BudgetHitBehavior::Partial,
                    false,
                ),
                parse_failures: BudgetLimitRule::new(
                    100,
                    10_000,
                    BudgetHitBehavior::FailClosed,
                    false,
                ),
                parse_failure_ratio_bps: BudgetLimitRule::new(
                    2_500,
                    10_000,
                    BudgetHitBehavior::FailClosed,
                    false,
                ),
                wall_time_ms: BudgetLimitRule::new(
                    30_000,
                    300_000,
                    BudgetHitBehavior::Partial,
                    false,
                ),
            },
            graph_traversal: GraphTraversalBudgetPolicy {
                seed_nodes: BudgetLimitRule::new(20, 100, BudgetHitBehavior::Clamp, true),
                seed_files: BudgetLimitRule::new(20, 100, BudgetHitBehavior::Clamp, true),
                depth: BudgetLimitRule::new(3, 10, BudgetHitBehavior::Clamp, true),
                nodes: BudgetLimitRule::new(200, 1_000, BudgetHitBehavior::Clamp, true),
                edges: BudgetLimitRule::new(500, 2_000, BudgetHitBehavior::Clamp, true),
            },
            query_candidates_and_seeds: QueryBudgetPolicy {
                candidates: BudgetLimitRule::new(20, 200, BudgetHitBehavior::Clamp, true),
                seeds: BudgetLimitRule::new(20, 100, BudgetHitBehavior::Partial, true),
                wall_time_ms: BudgetLimitRule::new(5_000, 30_000, BudgetHitBehavior::Partial, true),
            },
            review_context_extraction: ReviewContextBudgetPolicy {
                nodes: BudgetLimitRule::new(50, 200, BudgetHitBehavior::Clamp, true),
                edges: BudgetLimitRule::new(100, 400, BudgetHitBehavior::Clamp, true),
                files: BudgetLimitRule::new(20, 100, BudgetHitBehavior::Clamp, true),
            },
            content_saved_context_lookup: ContentLookupBudgetPolicy {
                sources: BudgetLimitRule::new(5, 25, BudgetHitBehavior::Clamp, true),
            },
            mcp_cli_payload_serialization: PayloadSerializationBudgetPolicy {
                nodes: BudgetLimitRule::new(100, 200, BudgetHitBehavior::Clamp, true),
                edges: BudgetLimitRule::new(100, 200, BudgetHitBehavior::Clamp, true),
                bytes: BudgetLimitRule::new(64 * 1024, 256 * 1024, BudgetHitBehavior::Clamp, true),
                review_source_bytes: BudgetLimitRule::new(
                    12 * 1024,
                    128 * 1024,
                    BudgetHitBehavior::Partial,
                    true,
                ),
                context_payload_bytes: BudgetLimitRule::new(
                    32 * 1024,
                    256 * 1024,
                    BudgetHitBehavior::Partial,
                    true,
                ),
                context_tokens_estimate: BudgetLimitRule::new(
                    8_000,
                    64_000,
                    BudgetHitBehavior::Partial,
                    true,
                ),
                file_excerpt_bytes: BudgetLimitRule::new(
                    4 * 1024,
                    64 * 1024,
                    BudgetHitBehavior::Partial,
                    true,
                ),
                saved_context_bytes: BudgetLimitRule::new(
                    2 * 1024,
                    32 * 1024,
                    BudgetHitBehavior::Partial,
                    true,
                ),
                mcp_response_bytes: BudgetLimitRule::new(
                    48 * 1024,
                    256 * 1024,
                    BudgetHitBehavior::Partial,
                    true,
                ),
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct BudgetManager {
    reports: Vec<BudgetReport>,
}

impl BudgetManager {
    pub fn new() -> Self {
        Self {
            reports: Vec::new(),
        }
    }

    pub fn resolve_limit(
        &mut self,
        rule: BudgetLimitRule,
        name: impl Into<String>,
        requested: Option<usize>,
    ) -> usize {
        let name = name.into();
        let observed = requested.unwrap_or(rule.default_limit);
        if observed <= rule.max_limit {
            return observed;
        }

        let report = match rule.hit_behavior {
            BudgetHitBehavior::Clamp => {
                BudgetReport::override_clamped(name, rule.max_limit, observed)
            }
            BudgetHitBehavior::Partial => BudgetReport::partial_result(
                name,
                rule.max_limit,
                observed,
                rule.safe_to_answer_on_hit,
            ),
            BudgetHitBehavior::FailClosed => BudgetReport::blocked(name, rule.max_limit, observed),
        };
        self.reports.push(report);
        rule.max_limit
    }

    pub fn record_report(&mut self, report: BudgetReport) {
        if report.budget_hit {
            self.reports.push(report);
        }
    }

    pub fn record_usage(
        &mut self,
        rule: BudgetLimitRule,
        name: impl Into<String>,
        limit: usize,
        observed: usize,
        partial: bool,
    ) {
        if observed <= limit && !partial {
            return;
        }

        let name = name.into();
        let report = match rule.hit_behavior {
            BudgetHitBehavior::FailClosed => BudgetReport::blocked(name, limit, observed),
            BudgetHitBehavior::Clamp | BudgetHitBehavior::Partial => {
                BudgetReport::partial_result(name, limit, observed, rule.safe_to_answer_on_hit)
            }
        };
        self.reports.push(report);
    }

    pub fn summary(
        &self,
        default_name: impl Into<String>,
        limit: usize,
        observed: usize,
    ) -> BudgetReport {
        self.reports
            .iter()
            .cloned()
            .max_by(|left, right| {
                left.budget_status
                    .severity()
                    .cmp(&right.budget_status.severity())
                    .then_with(|| left.budget_observed.cmp(&right.budget_observed))
            })
            .unwrap_or_else(|| BudgetReport::within_budget(default_name, limit, observed))
    }
}

impl Default for BudgetManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_limit_clamps_requested_override() {
        let mut manager = BudgetManager::new();
        let resolved = manager.resolve_limit(
            BudgetLimitRule::new(50, 200, BudgetHitBehavior::Clamp, true),
            "review_context_extraction.max_nodes",
            Some(500),
        );
        assert_eq!(resolved, 200);

        let summary = manager.summary("review_context_extraction.max_nodes", resolved, resolved);
        assert_eq!(summary.budget_status, BudgetStatus::OverrideClamped);
        assert!(summary.budget_hit);
        assert_eq!(summary.budget_limit, 200);
        assert_eq!(summary.budget_observed, 500);
    }

    #[test]
    fn record_usage_marks_partial_result() {
        let mut manager = BudgetManager::new();
        manager.record_usage(
            BudgetLimitRule::new(50, 200, BudgetHitBehavior::Partial, true),
            "review_context_extraction.max_nodes",
            50,
            75,
            true,
        );

        let summary = manager.summary("review_context_extraction.max_nodes", 50, 50);
        assert_eq!(summary.budget_status, BudgetStatus::PartialResult);
        assert!(summary.partial);
        assert!(summary.safe_to_answer);
    }

    #[test]
    fn record_report_merges_with_worst_status() {
        let mut manager = BudgetManager::new();
        manager.record_report(BudgetReport::partial_result(
            "review_context_extraction.max_nodes",
            50,
            75,
            true,
        ));
        manager.record_report(BudgetReport::blocked(
            "query_candidates_and_seeds.symbol_resolution",
            1,
            2,
        ));

        let summary = manager.summary("none", 0, 0);
        assert_eq!(summary.budget_status, BudgetStatus::Blocked);
        assert_eq!(
            summary.budget_name,
            "query_candidates_and_seeds.symbol_resolution"
        );
        assert!(!summary.safe_to_answer);
    }

    #[test]
    fn budget_stage_rules_cover_b4_stages() {
        let stages: std::collections::BTreeSet<_> =
            budget_stage_rules().iter().map(|rule| rule.stage).collect();
        assert!(stages.contains(&BudgetStage::Build));
        assert!(stages.contains(&BudgetStage::Update));
        assert!(stages.contains(&BudgetStage::Query));
        assert!(stages.contains(&BudgetStage::Impact));
        assert!(stages.contains(&BudgetStage::ReviewContext));
        assert!(stages.contains(&BudgetStage::MinimalContext));
        assert!(stages.contains(&BudgetStage::SavedContextRetrieval));
        assert!(stages.contains(&BudgetStage::McpResponseSerialization));
        assert!(
            budget_stage_rules()
                .iter()
                .any(|rule| matches!(rule.outcome, BudgetOutcomeClass::HardError)
                    && !rule.safe_to_answer)
        );
    }
}
