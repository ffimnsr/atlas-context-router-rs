//! Deterministic token counting for Atlas context payloads.
//!
//! This crate is the only crate that depends on external tokenizer
//! machinery (`tokenizers`). Callers ask [`TokenCounter`] to count tokens
//! and receive [`TokenCount`] plus [`TokenCountMethod`] metadata so they
//! can surface how a count was produced.

use std::path::Path;
use std::sync::Arc;

use anyhow::Context;

/// How a [`TokenCount`] was produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenCountMethod {
    /// Counted with a local tokenizer model.
    Tokenizer {
        /// Provider name (e.g. model family or source label).
        provider: String,
        /// Optional model identifier.
        model: Option<String>,
    },
    /// Counted with the byte heuristic `bytes.div_ceil(bytes_per_token)`.
    HeuristicBytes { bytes_per_token: usize },
}

/// Result of counting tokens for one text payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenCount {
    /// Number of tokens.
    pub tokens: usize,
    /// Method used to produce the count.
    pub method: TokenCountMethod,
    /// Why the heuristic was used, when tokenizer loading/counting failed.
    /// `None` when the count was produced without a fallback.
    pub fallback_reason: Option<String>,
}

/// A token counter backed by a local tokenizer file or a byte heuristic.
#[derive(Debug, Clone)]
pub enum TokenCounter {
    /// Tokenizer-backed counting using a local tokenizer JSON file.
    Tokenizer {
        tokenizer: Arc<tokenizers::Tokenizer>,
        provider: String,
        model: Option<String>,
    },
    /// Heuristic counting: `text.len().div_ceil(bytes_per_token)`.
    Heuristic { bytes_per_token: usize },
}

/// Newtype so tokenizers' boxed error (`Box<dyn Error + Send + Sync>`) can
/// convert into `anyhow::Error` while preserving the source chain.
#[derive(Debug)]
struct TokenizersError(Box<dyn std::error::Error + Send + Sync>);

impl std::fmt::Display for TokenizersError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for TokenizersError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.0)
    }
}

impl TokenCounter {
    /// Build a heuristic counter using `text.len().div_ceil(bytes_per_token)`.
    ///
    /// Rejects `bytes_per_token == 0` to keep the count deterministic and
    /// the division meaningful.
    pub fn heuristic(bytes_per_token: usize) -> anyhow::Result<Self> {
        if bytes_per_token == 0 {
            anyhow::bail!("bytes_per_token must be greater than zero");
        }
        Ok(Self::Heuristic { bytes_per_token })
    }

    /// Load a tokenizer from a local JSON file.
    ///
    /// The path is attached to errors so failures are actionable, without
    /// leaking payload content. Model discovery is never network-backed.
    pub fn from_file(
        path: impl AsRef<Path>,
        provider: impl Into<String>,
        model: Option<String>,
    ) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let tokenizer = tokenizers::Tokenizer::from_file(path)
            .map_err(TokenizersError)
            .with_context(|| format!("failed to load tokenizer from {}", path.display()))?;
        Ok(Self::Tokenizer {
            tokenizer: Arc::new(tokenizer),
            provider: provider.into(),
            model,
        })
    }

    /// Count tokens in `text`, exactly as passed; text is never normalized
    /// before counting.
    pub fn count_text(&self, text: &str) -> anyhow::Result<TokenCount> {
        match self {
            Self::Tokenizer {
                tokenizer,
                provider,
                model,
            } => {
                let encoding = tokenizer
                    .encode(text, false)
                    .map_err(TokenizersError)
                    .with_context(|| format!("tokenizer encode failed (provider: {provider})"))?;
                Ok(TokenCount {
                    tokens: encoding.len(),
                    method: TokenCountMethod::Tokenizer {
                        provider: provider.clone(),
                        model: model.clone(),
                    },
                    fallback_reason: None,
                })
            }
            Self::Heuristic { bytes_per_token } => Ok(TokenCount {
                tokens: text.len().div_ceil(*bytes_per_token),
                method: TokenCountMethod::HeuristicBytes {
                    bytes_per_token: *bytes_per_token,
                },
                fallback_reason: None,
            }),
        }
    }

    /// Count tokens in UTF-8 JSON bytes.
    ///
    /// Errors carry no payload content, only that the input was not valid
    /// UTF-8.
    pub fn count_json_bytes(&self, bytes: &[u8]) -> anyhow::Result<TokenCount> {
        let text =
            std::str::from_utf8(bytes).context("cannot count tokens: input is not valid UTF-8")?;
        self.count_text(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heuristic_counts_empty_string_as_zero() {
        let counter = TokenCounter::heuristic(4).unwrap();
        let count = counter.count_text("").unwrap();
        assert_eq!(count.tokens, 0);
    }

    #[test]
    fn heuristic_counts_one_byte_as_one() {
        let counter = TokenCounter::heuristic(4).unwrap();
        let count = counter.count_text("a").unwrap();
        assert_eq!(count.tokens, 1);
    }

    #[test]
    fn heuristic_counts_four_bytes_as_one() {
        let counter = TokenCounter::heuristic(4).unwrap();
        let count = counter.count_text("abcd").unwrap();
        assert_eq!(count.tokens, 1);
    }

    #[test]
    fn heuristic_counts_five_bytes_as_two() {
        let counter = TokenCounter::heuristic(4).unwrap();
        let count = counter.count_text("abcde").unwrap();
        assert_eq!(count.tokens, 2);
    }

    #[test]
    fn heuristic_reports_method_metadata() {
        let counter = TokenCounter::heuristic(4).unwrap();
        let count = counter.count_text("abcdefgh").unwrap();
        assert_eq!(
            count.method,
            TokenCountMethod::HeuristicBytes { bytes_per_token: 4 }
        );
        assert!(count.fallback_reason.is_none());
    }

    #[test]
    fn heuristic_rejects_zero_bytes_per_token() {
        assert!(TokenCounter::heuristic(0).is_err());
    }

    #[test]
    fn count_json_bytes_counts_valid_utf8() {
        let counter = TokenCounter::heuristic(4).unwrap();
        let count = counter.count_json_bytes(b"{\"k\":\"v\"}").unwrap();
        assert_eq!(count.tokens, 3);
    }

    #[test]
    fn count_json_bytes_rejects_invalid_utf8_without_content() {
        let counter = TokenCounter::heuristic(4).unwrap();
        let err = counter
            .count_json_bytes(&[0xff, 0xfe])
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("UTF-8"),
            "error should explain the failure: {err}"
        );
    }
}
