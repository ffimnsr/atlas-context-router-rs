use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::budget::BudgetReport;

use super::graph::{Edge, Node};
use super::sufficiency::EvidenceSufficiencyReport;

fn is_false(value: &bool) -> bool {
    !*value
}

mod evidence;
mod meta;
mod request;
mod result;
mod workflow;

pub use self::evidence::ContextRankingEvidence;
pub use self::evidence::ContextScoreEvidence;
pub use self::evidence::ExperienceShortcutEvidence;
pub use self::evidence::context_ranking_evidence_legend;
pub use self::meta::ContextSourceMix;
pub use self::meta::PayloadTruncationMeta;
pub use self::meta::SeedBudgetMeta;
pub use self::meta::SelectedEdge;
pub use self::meta::SelectedFile;
pub use self::meta::SelectedNode;
pub use self::meta::TokenAccountingMeta;
pub use self::meta::TraversalBudgetMeta;
pub use self::meta::TruncationMeta;
pub use self::request::ContextIntent;
pub use self::request::ContextRequest;
pub use self::request::ContextTarget;
pub use self::request::SelectionReason;
pub use self::result::ContentAsset;
pub use self::result::ContextOutcome;
pub use self::result::ContextOverview;
pub use self::result::ContextResult;
pub use self::result::ContextTelemetry;
pub use self::result::SavedContextSource;
pub use self::workflow::AmbiguityMeta;
pub use self::workflow::ContentAssetReason;
pub use self::workflow::MixedResultKind;
pub use self::workflow::NoiseReductionSummary;
pub use self::workflow::WorkflowCallChain;
pub use self::workflow::WorkflowComponent;
pub use self::workflow::WorkflowFocusNode;
pub use self::workflow::WorkflowSummary;
#[cfg(test)]
mod tests {
    use super::*;

    fn payload_meta(token_accounting: Option<TokenAccountingMeta>) -> PayloadTruncationMeta {
        PayloadTruncationMeta {
            bytes_requested: 1000,
            bytes_emitted: 500,
            tokens_estimated: 125,
            token_budget_applied: Some(100),
            omitted_node_count: 2,
            omitted_file_count: 0,
            omitted_source_count: 1,
            omitted_byte_count: 500,
            continuation_hint: None,
            source_mix: Vec::new(),
            token_accounting,
        }
    }

    #[test]
    fn payload_truncation_keeps_tokens_estimated_and_emits_token_accounting() {
        let meta = payload_meta(Some(TokenAccountingMeta {
            provider: "heuristic".to_owned(),
            encoding: None,
            model: None,
            fallback_used: true,
            fallback_reason: Some("tokenizer file missing".to_owned()),
            bytes_per_token: Some(4),
        }));
        let value = serde_json::to_value(&meta).expect("serialize");
        // Compatibility field must remain present with the counted value.
        assert_eq!(value["tokens_estimated"], serde_json::json!(125));
        assert_eq!(
            value["token_accounting"]["provider"],
            serde_json::json!("heuristic")
        );
        assert_eq!(
            value["token_accounting"]["fallback_used"],
            serde_json::json!(true)
        );
        assert_eq!(
            value["token_accounting"]["fallback_reason"],
            serde_json::json!("tokenizer file missing")
        );
        assert_eq!(
            value["token_accounting"]["bytes_per_token"],
            serde_json::json!(4)
        );
    }

    #[test]
    fn token_accounting_skips_none_fields() {
        let meta = payload_meta(Some(TokenAccountingMeta {
            provider: "tokenizers".to_owned(),
            encoding: None,
            model: Some("simple-bpe".to_owned()),
            fallback_used: false,
            fallback_reason: None,
            bytes_per_token: None,
        }));
        let value = serde_json::to_value(&meta).expect("serialize");
        let accounting = &value["token_accounting"];
        assert_eq!(accounting["model"], serde_json::json!("simple-bpe"));
        assert!(accounting.get("encoding").is_none());
        assert!(accounting.get("fallback_reason").is_none());
        assert!(accounting.get("bytes_per_token").is_none());
    }

    #[test]
    fn token_accounting_emits_tiktoken_encoding() {
        let meta = payload_meta(Some(TokenAccountingMeta {
            provider: "tiktoken".to_owned(),
            encoding: Some("o200k_base".to_owned()),
            model: Some("gpt-4o".to_owned()),
            fallback_used: false,
            fallback_reason: None,
            bytes_per_token: None,
        }));
        let value = serde_json::to_value(&meta).expect("serialize");
        let accounting = &value["token_accounting"];
        assert_eq!(accounting["provider"], serde_json::json!("tiktoken"));
        assert_eq!(accounting["encoding"], serde_json::json!("o200k_base"));
        assert_eq!(accounting["model"], serde_json::json!("gpt-4o"));
    }

    #[test]
    fn token_accounting_omitted_when_none() {
        let value = serde_json::to_value(payload_meta(None)).expect("serialize");
        assert!(value.get("token_accounting").is_none());
        assert_eq!(value["tokens_estimated"], serde_json::json!(125));
    }
}
