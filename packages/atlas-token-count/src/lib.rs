//! Deterministic token counting for Atlas context payloads.
//!
//! This crate wraps OpenAI-compatible `tiktoken` encoding machinery behind
//! one deterministic [`TokenCounter`] API. Callers count tokens and receive
//! [`TokenCount`] plus [`TokenCountMethod`] metadata so they can surface how
//! a count was produced. The byte heuristic remains available as the
//! automatic fallback when a configured encoding cannot load.

use std::path::Path;
use std::sync::Arc;

use anyhow::Context;
use atlas_tokenizer::{Encoding, EncodingName};

/// How a [`TokenCount`] was produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenCountMethod {
    /// Counted with an OpenAI-compatible `tiktoken` encoding.
    Tiktoken {
        /// Encoding name, e.g. `o200k_base`.
        encoding: String,
        /// Optional model identifier preserved for accounting.
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

/// A token counter backed by a `tiktoken` encoding or a byte heuristic.
#[derive(Debug, Clone)]
pub enum TokenCounter {
    /// OpenAI-compatible byte-pair counting.
    Tiktoken {
        encoding: Arc<Encoding>,
        model: Option<String>,
    },
    /// Heuristic counting: `text.len().div_ceil(bytes_per_token)`.
    Heuristic { bytes_per_token: usize },
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

    /// Build a tokenizer backed by an OpenAI-compatible `tiktoken` encoding.
    pub fn from_tiktoken(encoding: Arc<Encoding>, model: Option<String>) -> Self {
        Self::Tiktoken { encoding, model }
    }

    /// Load a `tiktoken` encoding by name through the default loader.
    pub fn from_encoding(name: EncodingName, model: Option<String>) -> anyhow::Result<Self> {
        let encoding = Encoding::get(name)
            .with_context(|| format!("failed to load tiktoken encoding {}", name.as_str()))?;
        Ok(Self::from_tiktoken(encoding, model))
    }

    /// Load the `tiktoken` encoding that the default loader maps to `model`.
    pub fn from_model(model: impl AsRef<str>) -> anyhow::Result<Self> {
        let model = model.as_ref();
        let encoding = Encoding::for_model(model)
            .with_context(|| format!("failed to load tiktoken encoding for model {model}"))?;
        Ok(Self::from_tiktoken(encoding, Some(model.to_owned())))
    }

    /// Load a local `.tiktoken` rank file with the default encoding spec.
    pub fn from_tiktoken_file(
        path: impl AsRef<Path>,
        model: Option<String>,
    ) -> anyhow::Result<Self> {
        Self::from_tiktoken_file_with_encoding(path, EncodingName::default(), model)
    }

    /// Load a local `.tiktoken` rank file with an explicit encoding spec.
    pub fn from_tiktoken_file_with_encoding(
        path: impl AsRef<Path>,
        name: EncodingName,
        model: Option<String>,
    ) -> anyhow::Result<Self> {
        let path = path.as_ref();
        let encoding = Encoding::from_rank_file_with_name(path, name).with_context(|| {
            format!(
                "failed to load tiktoken rank file from {} ({})",
                path.display(),
                name.as_str()
            )
        })?;
        Ok(Self::from_tiktoken(Arc::new(encoding), model))
    }

    /// Count tokens in `text`, exactly as passed; text is never normalized
    /// before counting.
    pub fn count_text(&self, text: &str) -> anyhow::Result<TokenCount> {
        match self {
            Self::Tiktoken { encoding, model } => {
                let tokens = encoding.encode_ordinary(text).with_context(|| {
                    format!(
                        "tiktoken encode failed (encoding: {})",
                        encoding.name().as_str()
                    )
                })?;
                Ok(TokenCount {
                    tokens: tokens.len(),
                    method: TokenCountMethod::Tiktoken {
                        encoding: encoding.name().as_str().to_owned(),
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
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;

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

    fn tiktoken_fixture_bytes() -> Vec<u8> {
        let mut entries: Vec<(Vec<u8>, u32)> = (0..=u8::MAX)
            .map(|byte| (vec![byte], u32::from(byte)))
            .collect();
        entries.push((b"ab".to_vec(), 256));
        entries.push((b"cd".to_vec(), 257));
        let mut input = String::new();
        for (token, rank) in entries {
            input.push_str(&format!("{0} {rank}\n", BASE64.encode(token)));
        }
        input.into_bytes()
    }

    fn tiktoken_fixture_encoding() -> Arc<Encoding> {
        Arc::new(
            Encoding::from_rank_bytes(
                &tiktoken_fixture_bytes(),
                atlas_tokenizer::EncodingSpec::for_name(EncodingName::Cl100kBase),
            )
            .expect("synthetic tiktoken encoding"),
        )
    }

    #[test]
    fn tiktoken_counts_with_encoding_metadata() {
        let counter = TokenCounter::from_tiktoken(
            tiktoken_fixture_encoding(),
            Some("fixture-model".to_owned()),
        );
        let count = counter.count_text("abcd").expect("count");
        assert_eq!(count.tokens, 2);
        assert_eq!(
            count.method,
            TokenCountMethod::Tiktoken {
                encoding: "cl100k_base".to_owned(),
                model: Some("fixture-model".to_owned()),
            }
        );
        assert!(count.fallback_reason.is_none());
    }

    #[test]
    fn tiktoken_counts_json_bytes() {
        let counter = TokenCounter::from_tiktoken(tiktoken_fixture_encoding(), None);
        let count = counter.count_json_bytes(b"abcd").expect("count");
        assert_eq!(count.tokens, 2);
    }

    #[test]
    fn tiktoken_file_constructor_uses_explicit_encoding() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("fixture.tiktoken");
        std::fs::write(&path, tiktoken_fixture_bytes()).expect("write fixture");
        let counter = TokenCounter::from_tiktoken_file_with_encoding(
            &path,
            EncodingName::Cl100kBase,
            Some("file-model".to_owned()),
        )
        .expect("load file");
        let count = counter.count_text("abcd").expect("count");
        assert_eq!(count.tokens, 2);
        assert_eq!(
            count.method,
            TokenCountMethod::Tiktoken {
                encoding: "cl100k_base".to_owned(),
                model: Some("file-model".to_owned()),
            }
        );
    }

    #[test]
    fn unknown_model_errors_without_network() {
        let err = TokenCounter::from_model("definitely-not-a-model")
            .expect_err("unknown model")
            .to_string();
        assert!(err.contains("tiktoken"), "error context: {err}");
    }
}
