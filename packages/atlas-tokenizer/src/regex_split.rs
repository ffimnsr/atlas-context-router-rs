//! Unicode-aware regex pre-tokenization for encoding patterns.
//!
//! The upstream `tiktoken` patterns require lookahead, so this module uses
//! `fancy-regex`. The splitter always covers the whole input: any bytes not
//! matched by the pattern are emitted as their own piece, which keeps encode
//! and decode roundtrips lossless even if a custom spec is malformed.

use fancy_regex::Regex;

use crate::error::{Error, Result};

/// Compiled encoding split pattern.
#[derive(Debug)]
pub struct RegexSplit {
    regex: Regex,
}

impl RegexSplit {
    /// Compile an upstream encoding pattern.
    pub fn new(pattern: &str) -> Result<Self> {
        let regex = Regex::new(pattern).map_err(|err| Error::InvalidRegex {
            message: err.to_string(),
        })?;
        Ok(Self { regex })
    }

    /// Split text into contiguous ordinary pieces.
    pub fn split<'a>(&self, text: &'a str) -> Result<Vec<&'a str>> {
        let mut pieces = Vec::new();
        let mut cursor = 0;
        for found in self.regex.find_iter(text) {
            let found = found.map_err(|err| Error::InvalidRegex {
                message: err.to_string(),
            })?;
            let (start, end) = (found.start(), found.end());
            if start > cursor {
                pieces.push(&text[cursor..start]);
            }
            if end > start {
                pieces.push(&text[start..end]);
            }
            cursor = cursor.max(end);
        }
        if cursor < text.len() {
            pieces.push(&text[cursor..]);
        }
        Ok(pieces)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_covers_input_for_upstream_patterns() {
        let cases = [
            (crate::encoding::R50K_PATTERN, "hello,  world! 123\n"),
            (
                crate::encoding::CL100K_PATTERN,
                "naive\u{301} café 中文 1234",
            ),
            (crate::encoding::O200K_PATTERN, "emoji 👩‍💻 and\r\nnewlines"),
        ];
        for (pattern, text) in cases {
            let split = RegexSplit::new(pattern).expect("compile");
            let pieces = split.split(text).expect("split");
            assert_eq!(pieces.concat(), text);
            assert!(pieces.iter().all(|piece| !piece.is_empty()));
        }
    }

    #[test]
    fn split_empty_input_returns_no_pieces() {
        let split = RegexSplit::new(crate::encoding::R50K_PATTERN).expect("compile");
        assert!(split.split("").expect("split").is_empty());
    }

    #[test]
    fn invalid_pattern_is_a_typed_error() {
        let err = RegexSplit::new("(").expect_err("invalid regex");
        assert!(matches!(err, Error::InvalidRegex { .. }));
    }
}
