//! Byte-pair encoding and decoding primitives.
//!
//! Encoding works on one regex-split piece at a time. A direct vocabulary hit
//! is O(1). Otherwise the merge loop starts from single-byte ranks and
//! repeatedly merges the adjacent pair with the lowest ordinary rank, using
//! the rank file's lazily built pair table.

use crate::Rank;
use crate::error::{Error, Result};
use crate::rank_file::RankFile;

/// Encode one ordinary pre-tokenized piece into ranks.
///
/// The input bytes must be raw UTF-8 bytes from the caller's split; no
/// normalization is applied. Empty input produces no tokens.
pub fn encode_piece(ranks: &RankFile, piece: &[u8]) -> Result<Vec<Rank>> {
    if piece.is_empty() {
        return Ok(Vec::new());
    }
    if let Some(rank) = ranks.rank_for(piece) {
        return Ok(vec![rank]);
    }

    let mut parts = Vec::with_capacity(piece.len());
    for &byte in piece {
        let Some(rank) = ranks.rank_for(&[byte]) else {
            return Err(Error::MissingByteRank { byte });
        };
        parts.push(rank);
    }

    let pair_ranks = ranks.pair_ranks();
    loop {
        let mut best: Option<(usize, Rank)> = None;
        for index in 0..parts.len().saturating_sub(1) {
            let Some(&rank) = pair_ranks.get(&(parts[index], parts[index + 1])) else {
                continue;
            };
            if best.is_none_or(|(_, best_rank)| rank < best_rank) {
                best = Some((index, rank));
            }
        }
        let Some((index, rank)) = best else {
            break;
        };
        parts[index] = rank;
        parts.remove(index + 1);
    }
    Ok(parts)
}

/// Decode ordinary ranks into raw bytes.
///
/// Unknown ranks are an error; no replacement bytes are inserted.
pub fn decode_bytes(ranks: &RankFile, tokens: &[Rank]) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for &rank in tokens {
        let Some(token) = ranks.token_for(rank) else {
            return Err(Error::UnknownRank { rank });
        };
        bytes.extend_from_slice(token);
    }
    Ok(bytes)
}

/// Decode ordinary ranks into UTF-8 with invalid sequences replaced.
///
/// This mirrors Python `tiktoken`'s default `errors="replace"` behavior.
pub fn decode_lossy(ranks: &RankFile, tokens: &[Rank]) -> Result<String> {
    let bytes = decode_bytes(ranks, tokens)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Decode ordinary ranks into UTF-8, rejecting invalid sequences.
pub fn decode_strict(ranks: &RankFile, tokens: &[Rank]) -> Result<String> {
    let bytes = decode_bytes(ranks, tokens)?;
    String::from_utf8(bytes).map_err(|_| Error::InvalidUtf8Output)
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use proptest::prelude::*;

    use super::*;

    fn rank_file(entries: &[(&[u8], Rank)]) -> RankFile {
        let mut input = String::new();
        for (token, rank) in entries {
            input.push_str(&format!("{} {rank}\n", BASE64.encode(token)));
        }
        RankFile::from_bytes(input.as_bytes()).expect("fixture rank file")
    }

    fn byte_rank_file() -> RankFile {
        let mut input = String::new();
        for byte in 0..=u8::MAX {
            input.push_str(&format!("{} {}\n", BASE64.encode([byte]), u32::from(byte)));
        }
        RankFile::from_bytes(input.as_bytes()).expect("byte rank file")
    }

    #[test]
    fn encodes_empty_piece_to_no_tokens() {
        let ranks = byte_rank_file();
        assert_eq!(encode_piece(&ranks, b"").unwrap(), Vec::<Rank>::new());
        assert_eq!(decode_bytes(&ranks, &[]).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn uses_direct_vocabulary_hit() {
        let ranks = rank_file(&[(b"a", 0), (b"b", 1), (b"ab", 2)]);
        assert_eq!(encode_piece(&ranks, b"ab").unwrap(), vec![2]);
    }

    #[test]
    fn keeps_single_bytes_when_no_pair_merges() {
        let ranks = byte_rank_file();
        assert_eq!(
            encode_piece(&ranks, b"ab").unwrap(),
            vec![Rank::from(b'a'), Rank::from(b'b')]
        );
    }

    #[test]
    fn applies_deep_merges_in_rank_order() {
        let ranks = rank_file(&[
            (b"a", 0),
            (b"b", 1),
            (b"c", 2),
            (b"ab", 3),
            (b"bc", 4),
            (b"abc", 5),
        ]);
        assert_eq!(encode_piece(&ranks, b"abc").unwrap(), vec![5]);
    }

    #[test]
    fn lowest_rank_pair_wins() {
        let ranks = rank_file(&[(b"a", 0), (b"b", 1), (b"c", 2), (b"bc", 3), (b"ab", 9)]);
        assert_eq!(encode_piece(&ranks, b"abc").unwrap(), vec![0, 3]);
    }

    #[test]
    fn tie_break_is_leftmost_pair() {
        // Both pairs have rank 4 and 5; the lowest rank is unique, then the
        // resulting token participates in the next merge.
        let ranks = rank_file(&[
            (b"x", 0),
            (b"y", 1),
            (b"z", 2),
            (b"xy", 4),
            (b"yz", 5),
            (b"xyz", 6),
        ]);
        assert_eq!(encode_piece(&ranks, b"xyz").unwrap(), vec![6]);
    }

    #[test]
    fn rejects_piece_with_missing_byte_rank() {
        let ranks = rank_file(&[(b"aa", 0)]);
        let err = encode_piece(&ranks, b"b").expect_err("missing byte");
        assert!(matches!(err, Error::MissingByteRank { byte: b'b' }));
    }

    #[test]
    fn decode_rejects_unknown_rank() {
        let ranks = rank_file(&[(b"a", 0)]);
        let err = decode_bytes(&ranks, &[99]).expect_err("unknown rank");
        assert!(matches!(err, Error::UnknownRank { rank: 99 }));
    }

    #[test]
    fn decode_lossy_replaces_invalid_utf8() {
        let ranks = rank_file(&[(b"\xff", 0)]);
        assert_eq!(decode_lossy(&ranks, &[0]).unwrap(), "\u{fffd}");
        assert!(matches!(
            decode_strict(&ranks, &[0]).expect_err("strict"),
            Error::InvalidUtf8Output
        ));
    }

    #[test]
    fn decode_concatenates_tokens() {
        let ranks = rank_file(&[(b"he", 0), (b"llo", 1)]);
        assert_eq!(decode_bytes(&ranks, &[0, 1]).unwrap(), b"hello");
        assert_eq!(decode_strict(&ranks, &[0, 1]).unwrap(), "hello");
    }

    proptest! {
        #[test]
        fn ordinary_roundtrip_preserves_arbitrary_utf8(text in "\\PC*") {
            let ranks = byte_rank_file();
            let tokens = encode_piece(&ranks, text.as_bytes()).unwrap();
            prop_assert_eq!(decode_bytes(&ranks, &tokens).unwrap(), text.as_bytes());
        }

        #[test]
        fn ordinary_roundtrip_preserves_arbitrary_bytes(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
            let ranks = byte_rank_file();
            let tokens = encode_piece(&ranks, &bytes).unwrap();
            prop_assert_eq!(decode_bytes(&ranks, &tokens).unwrap(), bytes);
        }

        #[test]
        fn merged_vocabulary_roundtrip_preserves_text(text in "\\PC*") {
            let mut entries: Vec<(Vec<u8>, Rank)> = (0..=u8::MAX)
                .map(|byte| (vec![byte], u32::from(byte)))
                .collect();
            for (next_rank, token) in (256..).zip([b"ab".as_slice(), b"bc", b"abc", b"ing", b"the"]) {
                entries.push((token.to_vec(), next_rank));
            }
            let mut input = String::new();
            for (token, rank) in &entries {
                input.push_str(&format!("{} {rank}\n", BASE64.encode(token)));
            }
            let ranks = RankFile::from_bytes(input.as_bytes()).expect("fixture");
            let tokens = encode_piece(&ranks, text.as_bytes()).unwrap();
            prop_assert_eq!(decode_bytes(&ranks, &tokens).unwrap(), text.as_bytes());
        }
    }
}
