//! `.tiktoken` rank file parsing and validation.
//!
//! The format is one `base64(token_bytes) <rank>` entry per line. Parsing
//! accepts LF and CRLF, skips blank lines and `#` comment lines, and rejects
//! malformed input before any tokenizer state is built. Caps on bytes, line
//! length, entry count, and rank values bound memory use for untrusted files.

use std::collections::{HashMap, HashSet};
#[cfg(feature = "file-assets")]
use std::fs;
#[cfg(feature = "file-assets")]
use std::path::Path;
use std::sync::{Arc, OnceLock};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use crate::Rank;
use crate::error::{Error, Result};

/// Resource limits applied while parsing a rank file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RankFileLimits {
    /// Maximum total rank-file bytes.
    pub max_bytes: usize,
    /// Maximum bytes in one line, excluding the line terminator.
    pub max_line_bytes: usize,
    /// Maximum number of token entries.
    pub max_entries: usize,
    /// Maximum accepted rank value, inclusive.
    pub max_rank: Rank,
}

impl Default for RankFileLimits {
    fn default() -> Self {
        Self {
            max_bytes: 64 * 1024 * 1024,
            max_line_bytes: 64 * 1024,
            max_entries: 1_000_000,
            max_rank: 1_000_000,
        }
    }
}

/// Parsed immutable rank file.
///
/// Construction validates every entry and builds the reverse rank-to-token
/// table. The `(left_rank, right_rank) -> merged_rank` pair table is built
/// lazily on the first encode call because it is the largest derived mapping.
#[derive(Debug)]
pub struct RankFile {
    mergeable_ranks: HashMap<Arc<[u8]>, Rank>,
    decoder: Vec<Option<Arc<[u8]>>>,
    pair_ranks: OnceLock<HashMap<(Rank, Rank), Rank>>,
    max_rank: Rank,
}

impl RankFile {
    /// Parse a rank file from bytes with default resource limits.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Self::from_bytes_with_limits(bytes, RankFileLimits::default())
    }

    /// Parse a rank file from bytes with explicit resource limits.
    pub fn from_bytes_with_limits(bytes: &[u8], limits: RankFileLimits) -> Result<Self> {
        if bytes.len() > limits.max_bytes {
            return Err(Error::RankFileTooLarge {
                actual: bytes.len(),
                limit: limits.max_bytes,
            });
        }

        let mut mergeable_ranks = HashMap::with_capacity(
            bytes
                .iter()
                .filter(|byte| **byte == b'\n')
                .count()
                .saturating_add(1),
        );
        let mut seen_ranks = HashSet::new();
        let mut max_rank = 0;
        let mut entry_count = 0usize;

        for (index, raw_line) in bytes.split(|byte| *byte == b'\n').enumerate() {
            let line_number = index + 1;
            if raw_line.len() > limits.max_line_bytes {
                return Err(Error::RankLineTooLong {
                    line: line_number,
                    limit: limits.max_line_bytes,
                });
            }
            let line = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
            let line = std::str::from_utf8(line)
                .map_err(|_| Error::RankLineNotUtf8 { line: line_number })?;
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let mut fields = line.split_ascii_whitespace();
            let token_field = fields
                .next()
                .ok_or(Error::MalformedRankLine { line: line_number })?;
            let rank_field = fields
                .next()
                .ok_or(Error::MalformedRankLine { line: line_number })?;
            if fields.next().is_some() {
                return Err(Error::MalformedRankLine { line: line_number });
            }

            let token = BASE64
                .decode(token_field)
                .map_err(|err| Error::InvalidRankBase64 {
                    line: line_number,
                    message: err.to_string(),
                })?;

            let rank = rank_field
                .parse::<Rank>()
                .map_err(|_| Error::InvalidRankValue {
                    line: line_number,
                    value: rank_field.to_owned(),
                })?;
            if rank > limits.max_rank {
                return Err(Error::RankValueTooHigh {
                    line: line_number,
                    rank,
                    limit: limits.max_rank,
                });
            }

            entry_count += 1;
            if entry_count > limits.max_entries {
                return Err(Error::RankEntriesTooMany {
                    limit: limits.max_entries,
                });
            }
            if mergeable_ranks.contains_key(token.as_slice()) {
                return Err(Error::DuplicateRankToken { line: line_number });
            }
            if !seen_ranks.insert(rank) {
                return Err(Error::DuplicateRankValue {
                    line: line_number,
                    rank,
                });
            }

            max_rank = max_rank.max(rank);
            mergeable_ranks.insert(Arc::from(token), rank);
        }

        if mergeable_ranks.is_empty() {
            return Err(Error::EmptyRankFile);
        }

        // `max_rank <= max_rank + 1` holds for every practical cap; reject a
        // cap that would overflow the decoder index on the target platform.
        let slot_count = usize::try_from(max_rank)
            .ok()
            .and_then(|rank| rank.checked_add(1))
            .ok_or(Error::RankLimitOverflow { limit: max_rank })?;
        let mut decoder = vec![None; slot_count];
        for (token, rank) in &mergeable_ranks {
            decoder[*rank as usize] = Some(token.clone());
        }

        Ok(Self {
            mergeable_ranks,
            decoder,
            pair_ranks: OnceLock::new(),
            max_rank,
        })
    }

    /// Read and parse a rank file with default resource limits.
    #[cfg(feature = "file-assets")]
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let bytes = fs::read(path).map_err(|source| Error::ReadRankFile {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_bytes(&bytes)
    }

    /// Number of token entries.
    pub fn len(&self) -> usize {
        self.mergeable_ranks.len()
    }

    /// Whether the rank file has no entries. Construction rejects this state,
    /// so the method always returns `false` for a successfully built value.
    pub fn is_empty(&self) -> bool {
        self.mergeable_ranks.is_empty()
    }

    /// Highest ordinary token rank in the file.
    pub fn max_rank(&self) -> Rank {
        self.max_rank
    }

    /// Ordinary token-to-rank map.
    pub fn mergeable_ranks(&self) -> &HashMap<Arc<[u8]>, Rank> {
        &self.mergeable_ranks
    }

    /// Reverse rank-to-token table; gaps are `None`.
    pub fn decoder(&self) -> &[Option<Arc<[u8]>>] {
        &self.decoder
    }

    /// Look up the rank for one token's bytes.
    pub fn rank_for(&self, token: &[u8]) -> Option<Rank> {
        self.mergeable_ranks.get(token).copied()
    }

    /// Look up the token bytes for one ordinary rank.
    pub fn token_for(&self, rank: Rank) -> Option<&[u8]> {
        self.decoder.get(rank as usize).and_then(Option::as_deref)
    }

    /// Lazily built adjacent-rank pair table used by the merge loop.
    pub fn pair_ranks(&self) -> &HashMap<(Rank, Rank), Rank> {
        self.pair_ranks.get_or_init(|| {
            let mut pairs = HashMap::<(Rank, Rank), Rank>::new();
            for (token, &rank) in &self.mergeable_ranks {
                if token.len() < 2 {
                    continue;
                }
                for split in 1..token.len() {
                    let Some(&left) = self.mergeable_ranks.get(&token[..split]) else {
                        continue;
                    };
                    let Some(&right) = self.mergeable_ranks.get(&token[split..]) else {
                        continue;
                    };
                    // Token bytes determine the pair, so collisions only occur
                    // for malformed custom files. Keep the lower rank to stay
                    // deterministic.
                    pairs
                        .entry((left, right))
                        .and_modify(|existing| *existing = (*existing).min(rank))
                        .or_insert(rank);
                }
            }
            pairs
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(token: &[u8], rank: Rank) -> String {
        format!("{} {rank}\n", BASE64.encode(token))
    }

    #[test]
    fn parses_lf_and_crlf_entries() {
        let mut input = String::from("# header\n\n");
        input.push_str(&line(b"a", 0).replace('\n', "\r\n"));
        input.push_str(&line(b"b", 1));
        let ranks = RankFile::from_bytes(input.as_bytes()).expect("parse");
        assert_eq!(ranks.len(), 2);
        assert_eq!(ranks.rank_for(b"a"), Some(0));
        assert_eq!(ranks.rank_for(b"b"), Some(1));
    }

    #[test]
    fn builds_reverse_decoder_and_pair_table() {
        let input = format!("{}{}{}", line(b"a", 0), line(b"b", 1), line(b"ab", 2));
        let ranks = RankFile::from_bytes(input.as_bytes()).expect("parse");
        assert_eq!(ranks.decoder().len(), 3);
        assert_eq!(ranks.token_for(2), Some(&b"ab"[..]));
        assert_eq!(ranks.pair_ranks().get(&(0, 1)), Some(&2));
    }

    #[test]
    fn rejects_invalid_base64() {
        let err = RankFile::from_bytes(b"%%% 0\n").expect_err("invalid base64");
        assert!(matches!(err, Error::InvalidRankBase64 { line: 1, .. }));
    }

    #[test]
    fn rejects_invalid_rank_value() {
        let err = RankFile::from_bytes(b"YQ== nope\n").expect_err("invalid rank");
        assert!(matches!(err, Error::InvalidRankValue { line: 1, .. }));
    }

    #[test]
    fn rejects_duplicate_token() {
        let input = format!("{}{}", line(b"a", 0), line(b"a", 1));
        let err = RankFile::from_bytes(input.as_bytes()).expect_err("duplicate token");
        assert!(matches!(err, Error::DuplicateRankToken { line: 2 }));
    }

    #[test]
    fn rejects_duplicate_rank() {
        let input = format!("{}{}", line(b"a", 0), line(b"b", 0));
        let err = RankFile::from_bytes(input.as_bytes()).expect_err("duplicate rank");
        assert!(matches!(
            err,
            Error::DuplicateRankValue { line: 2, rank: 0 }
        ));
    }

    #[test]
    fn rejects_empty_file() {
        let err = RankFile::from_bytes(b"# only comments\n").expect_err("empty file");
        assert!(matches!(err, Error::EmptyRankFile));
    }

    #[test]
    fn rejects_oversized_line() {
        let limits = RankFileLimits {
            max_line_bytes: 4,
            ..RankFileLimits::default()
        };
        let err =
            RankFile::from_bytes_with_limits(b"YWJjZA== 0\n", limits).expect_err("line too long");
        assert!(matches!(err, Error::RankLineTooLong { line: 1, limit: 4 }));
    }

    #[test]
    fn rejects_too_many_entries() {
        let limits = RankFileLimits {
            max_entries: 1,
            ..RankFileLimits::default()
        };
        let input = format!("{}{}", line(b"a", 0), line(b"b", 1));
        let err =
            RankFile::from_bytes_with_limits(input.as_bytes(), limits).expect_err("entry cap");
        assert!(matches!(err, Error::RankEntriesTooMany { limit: 1 }));
    }

    #[test]
    fn rejects_rank_above_cap() {
        let limits = RankFileLimits {
            max_rank: 1,
            ..RankFileLimits::default()
        };
        let input = line(b"a", 2);
        let err = RankFile::from_bytes_with_limits(input.as_bytes(), limits).expect_err("rank cap");
        assert!(matches!(
            err,
            Error::RankValueTooHigh {
                line: 1,
                rank: 2,
                limit: 1
            }
        ));
    }

    #[test]
    fn rejects_extra_fields() {
        let err = RankFile::from_bytes(b"YQ== 0 extra\n").expect_err("extra field");
        assert!(matches!(err, Error::MalformedRankLine { line: 1 }));
    }

    #[test]
    fn rejects_invalid_utf8_line() {
        let err = RankFile::from_bytes(b"\xff 0\n").expect_err("invalid utf8");
        assert!(matches!(err, Error::RankLineNotUtf8 { line: 1 }));
    }
}
