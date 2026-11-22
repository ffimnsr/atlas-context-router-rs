//! Encoding definitions, special tokens, and encode/decode entry points.
//!
//! Pattern strings and special-token values are copied verbatim from
//! `tiktoken_ext/openai_public.py` at the pinned upstream commit. Do not
//! simplify or rewrite them; parity tests catch drift.

use std::collections::HashMap;
#[cfg(feature = "file-assets")]
use std::path::Path;
use std::str::FromStr;
use std::sync::Arc;

use crate::Rank;
use crate::bpe;
use crate::error::{Error, Result};
use crate::rank_file::RankFile;
use crate::regex_split::RegexSplit;

/// `r50k_base` and `p50k_*` split pattern from upstream.
pub const R50K_PATTERN: &str =
    r"'(?:[sdmt]|ll|ve|re)| ?\p{L}++| ?\p{N}++| ?[^\s\p{L}\p{N}]++|\s++$|\s+(?!\S)|\s";

/// `cl100k_base` split pattern from upstream.
pub const CL100K_PATTERN: &str = r"'(?i:[sdmt]|ll|ve|re)|[^\r\n\p{L}\p{N}]?+\p{L}++|\p{N}{1,3}+| ?[^\s\p{L}\p{N}]++[\r\n]*+|\s++$|\s*[\r\n]|\s+(?!\S)|\s";

/// `o200k_base` and `o200k_harmony` split pattern from upstream.
pub const O200K_PATTERN: &str = r"[^\r\n\p{L}\p{N}]?[\p{Lu}\p{Lt}\p{Lm}\p{Lo}\p{M}]*[\p{Ll}\p{Lm}\p{Lo}\p{M}]+(?i:'s|'t|'re|'ve|'m|'ll|'d)?|[^\r\n\p{L}\p{N}]?[\p{Lu}\p{Lt}\p{Lm}\p{Lo}\p{M}]+[\p{Ll}\p{Lm}\p{Lo}\p{M}]*(?i:'s|'t|'re|'ve|'m|'ll|'d)?|\p{N}{1,3}| ?[^\s\p{L}\p{N}]+[\r\n/]*|\s*[\r\n]+|\s+(?!\S)|\s+";

/// Supported encoding names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub enum EncodingName {
    /// `r50k_base` (GPT-2 era).
    R50kBase,
    /// `p50k_base` (Codex era).
    P50kBase,
    /// `p50k_edit` (edit models).
    P50kEdit,
    /// `cl100k_base` (GPT-4, embeddings).
    Cl100kBase,
    /// `o200k_base` (GPT-4o and newer).
    #[default]
    O200kBase,
    /// `o200k_harmony` (harmony special tokens).
    O200kHarmony,
}

impl EncodingName {
    /// Canonical upstream encoding name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::R50kBase => "r50k_base",
            Self::P50kBase => "p50k_base",
            Self::P50kEdit => "p50k_edit",
            Self::Cl100kBase => "cl100k_base",
            Self::O200kBase => "o200k_base",
            Self::O200kHarmony => "o200k_harmony",
        }
    }

    /// Cargo feature required to load this encoding.
    pub fn feature(self) -> &'static str {
        match self {
            Self::R50kBase => "encoding-r50k",
            Self::P50kBase | Self::P50kEdit => "encoding-p50k",
            Self::Cl100kBase => "encoding-cl100k",
            Self::O200kBase => "encoding-o200k",
            Self::O200kHarmony => "encoding-o200k-harmony",
        }
    }

    /// Whether this encoding was compiled in.
    pub const fn is_enabled(self) -> bool {
        match self {
            Self::R50kBase => cfg!(feature = "encoding-r50k"),
            Self::P50kBase | Self::P50kEdit => cfg!(feature = "encoding-p50k"),
            Self::Cl100kBase => cfg!(feature = "encoding-cl100k"),
            Self::O200kBase => cfg!(feature = "encoding-o200k"),
            Self::O200kHarmony => cfg!(feature = "encoding-o200k-harmony"),
        }
    }
}

impl FromStr for EncodingName {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "r50k_base" => Ok(Self::R50kBase),
            "p50k_base" => Ok(Self::P50kBase),
            "p50k_edit" => Ok(Self::P50kEdit),
            "cl100k_base" => Ok(Self::Cl100kBase),
            "o200k_base" => Ok(Self::O200kBase),
            "o200k_harmony" => Ok(Self::O200kHarmony),
            other => Err(Error::unknown_encoding(other)),
        }
    }
}

/// One encoding's pattern and special-token definitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodingSpec {
    /// Encoding name.
    pub name: EncodingName,
    /// Exact upstream split pattern.
    pub pattern: &'static str,
    /// Special-token literals and ranks in upstream insertion order.
    pub special_tokens: Vec<(String, Rank)>,
}

impl EncodingSpec {
    /// Build the upstream spec for one encoding name.
    pub fn for_name(name: EncodingName) -> Self {
        let pattern = match name {
            EncodingName::R50kBase | EncodingName::P50kBase | EncodingName::P50kEdit => {
                R50K_PATTERN
            }
            EncodingName::Cl100kBase => CL100K_PATTERN,
            EncodingName::O200kBase | EncodingName::O200kHarmony => O200K_PATTERN,
        };
        let mut special_tokens = match name {
            EncodingName::R50kBase | EncodingName::P50kBase => {
                vec![("<|endoftext|>".to_owned(), 50256)]
            }
            EncodingName::P50kEdit => vec![
                ("<|endoftext|>".to_owned(), 50256),
                ("<|fim_prefix|>".to_owned(), 50281),
                ("<|fim_middle|>".to_owned(), 50282),
                ("<|fim_suffix|>".to_owned(), 50283),
            ],
            EncodingName::Cl100kBase => vec![
                ("<|endoftext|>".to_owned(), 100257),
                ("<|fim_prefix|>".to_owned(), 100258),
                ("<|fim_middle|>".to_owned(), 100259),
                ("<|fim_suffix|>".to_owned(), 100260),
                ("<|endofprompt|>".to_owned(), 100276),
            ],
            EncodingName::O200kBase => vec![
                ("<|endoftext|>".to_owned(), 199999),
                ("<|endofprompt|>".to_owned(), 200018),
            ],
            EncodingName::O200kHarmony => harmony_special_tokens(),
        };
        // Upstream harmony inserts reserved names in ascending rank order
        // after the named tokens; keep that order so decode twins resolve the
        // same way as Python `tiktoken`.
        if name == EncodingName::O200kHarmony {
            special_tokens.sort_by_key(|(_, rank)| *rank);
        }
        Self {
            name,
            pattern,
            special_tokens,
        }
    }
}

fn harmony_special_tokens() -> Vec<(String, Rank)> {
    let mut tokens = vec![
        ("<|startoftext|>".to_owned(), 199998),
        ("<|endoftext|>".to_owned(), 199999),
        ("<|reserved_200000|>".to_owned(), 200000),
        ("<|reserved_200001|>".to_owned(), 200001),
        ("<|return|>".to_owned(), 200002),
        ("<|constrain|>".to_owned(), 200003),
        ("<|reserved_200004|>".to_owned(), 200004),
        ("<|channel|>".to_owned(), 200005),
        ("<|start|>".to_owned(), 200006),
        ("<|end|>".to_owned(), 200007),
        ("<|message|>".to_owned(), 200008),
        ("<|reserved_200009|>".to_owned(), 200009),
        ("<|reserved_200010|>".to_owned(), 200010),
        ("<|reserved_200011|>".to_owned(), 200011),
        ("<|call|>".to_owned(), 200012),
        ("<|endofprompt|>".to_owned(), 200018),
    ];
    tokens.extend((200013..=201087).map(|rank| (format!("<|reserved_{rank}|>"), rank)));
    tokens
}

/// Immutable tokenizer for one encoding.
#[derive(Debug)]
pub struct Encoding {
    name: EncodingName,
    ranks: RankFile,
    split: RegexSplit,
    special_encoder: HashMap<String, Rank>,
    special_decoder: HashMap<Rank, Vec<u8>>,
    special_first_bytes: [bool; 256],
}

impl Encoding {
    /// Load an encoding by name through the process-wide default loader.
    pub fn get(name: EncodingName) -> Result<Arc<Self>> {
        crate::registry::default_loader().load(name)
    }

    /// Load the encoding for a model through the process-wide default loader.
    pub fn for_model(model: &str) -> Result<Arc<Self>> {
        crate::registry::default_loader().load_for_model(model)
    }

    /// Build an encoding from `.tiktoken` rank bytes and an explicit spec.
    pub fn from_rank_bytes(bytes: &[u8], spec: EncodingSpec) -> Result<Self> {
        let ranks = RankFile::from_bytes(bytes)?;
        Self::from_rank_file_parts(ranks, spec)
    }

    /// Build an encoding from a local rank file using the default encoding
    /// spec (`o200k_base`).
    #[cfg(feature = "file-assets")]
    pub fn from_rank_file(path: impl AsRef<Path>) -> Result<Self> {
        Self::from_rank_file_with_name(path, EncodingName::default())
    }

    /// Build an encoding from a local rank file with an explicit spec name.
    #[cfg(feature = "file-assets")]
    pub fn from_rank_file_with_name(path: impl AsRef<Path>, name: EncodingName) -> Result<Self> {
        let ranks = RankFile::from_file(path)?;
        Self::from_rank_file_parts(ranks, EncodingSpec::for_name(name))
    }

    fn from_rank_file_parts(ranks: RankFile, spec: EncodingSpec) -> Result<Self> {
        let split = RegexSplit::new(spec.pattern)?;
        let mut special_encoder = HashMap::new();
        let mut special_decoder = HashMap::new();
        let mut special_first_bytes = [false; 256];
        for (token, rank) in spec.special_tokens {
            if token.is_empty() {
                return Err(Error::EmptySpecialToken { name: token });
            }
            if special_encoder.contains_key(&token) {
                return Err(Error::DuplicateSpecialToken { name: token });
            }
            if ranks.token_for(rank).is_some() {
                return Err(Error::SpecialTokenRankCollision { name: token, rank });
            }
            special_first_bytes[usize::from(token.as_bytes()[0])] = true;
            special_encoder.insert(token.clone(), rank);
            // Duplicate special ranks are valid upstream (for example
            // `o200k_harmony` aliases `endofprompt` and `reserved_200018`).
            // Insertion order is preserved by the spec, so the last alias
            // wins on decode, matching Python dict construction.
            special_decoder.insert(rank, token.into_bytes());
        }
        Ok(Self {
            name: spec.name,
            ranks,
            split,
            special_encoder,
            special_decoder,
            special_first_bytes,
        })
    }

    /// Encoding name.
    pub fn name(&self) -> EncodingName {
        self.name
    }

    /// Special-token literal to rank map.
    pub fn special_tokens(&self) -> &HashMap<String, Rank> {
        &self.special_encoder
    }

    /// Encode text without recognizing special-token literals.
    pub fn encode_ordinary(&self, text: &str) -> Result<Vec<Rank>> {
        let mut ranks = Vec::new();
        self.encode_ordinary_segment(text, &mut ranks)?;
        Ok(ranks)
    }

    /// Encode text, recognizing only the listed special-token literals.
    ///
    /// The literal `"all"` in `allowed_special` allows every special token.
    /// Any other special literal present in the input is an error.
    pub fn encode(&self, text: &str, allowed_special: &[&str]) -> Result<Vec<Rank>> {
        let mut ranks = Vec::new();
        let mut plain_start = 0;
        let mut cursor = 0;
        while cursor < text.len() {
            let Some((token, rank)) = self.longest_special_at(text, cursor) else {
                cursor += text[cursor..].chars().next().map_or(1, char::len_utf8);
                continue;
            };
            if !special_is_allowed(token, allowed_special) {
                return Err(Error::DisallowedSpecialToken {
                    token: token.to_owned(),
                });
            }
            if plain_start < cursor {
                self.encode_ordinary_segment(&text[plain_start..cursor], &mut ranks)?;
            }
            ranks.push(rank);
            cursor += token.len();
            plain_start = cursor;
        }
        if plain_start < text.len() {
            self.encode_ordinary_segment(&text[plain_start..], &mut ranks)?;
        }
        Ok(ranks)
    }

    /// Encode text allowing every special token in this encoding.
    pub fn encode_with_special_tokens(&self, text: &str) -> Result<Vec<Rank>> {
        self.encode(text, &["all"])
    }

    /// Decode ranks to bytes, checking special and ordinary tokens.
    pub fn decode_bytes(&self, tokens: &[Rank]) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        for &rank in tokens {
            if let Some(token) = self.special_decoder.get(&rank) {
                bytes.extend_from_slice(token);
                continue;
            }
            let Some(token) = self.ranks.token_for(rank) else {
                return Err(Error::UnknownRank { rank });
            };
            bytes.extend_from_slice(token);
        }
        Ok(bytes)
    }

    /// Decode ranks to UTF-8 with invalid sequences replaced.
    pub fn decode(&self, tokens: &[Rank]) -> Result<String> {
        let bytes = self.decode_bytes(tokens)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Decode ranks to UTF-8, rejecting invalid sequences.
    pub fn decode_strict(&self, tokens: &[Rank]) -> Result<String> {
        let bytes = self.decode_bytes(tokens)?;
        String::from_utf8(bytes).map_err(|_| Error::InvalidUtf8Output)
    }

    fn encode_ordinary_segment(&self, text: &str, out: &mut Vec<Rank>) -> Result<()> {
        for piece in self.split.split(text)? {
            out.extend(bpe::encode_piece(&self.ranks, piece.as_bytes())?);
        }
        Ok(())
    }

    fn longest_special_at<'a>(&'a self, text: &str, cursor: usize) -> Option<(&'a str, Rank)> {
        let first = *text.as_bytes().get(cursor)?;
        if !self.special_first_bytes[usize::from(first)] {
            return None;
        }
        let rest = &text[cursor..];
        let mut best: Option<(&str, Rank)> = None;
        for (token, &rank) in &self.special_encoder {
            if rest.starts_with(token.as_str())
                && best.is_none_or(|(best_token, _)| token.len() > best_token.len())
            {
                best = Some((token.as_str(), rank));
            }
        }
        best
    }
}

fn special_is_allowed(token: &str, allowed_special: &[&str]) -> bool {
    allowed_special
        .iter()
        .any(|allowed| *allowed == "all" || *allowed == token)
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;

    use super::*;

    fn byte_rank_file() -> Vec<u8> {
        let mut input = String::new();
        for byte in 0..=u8::MAX {
            input.push_str(&format!("{} {}\n", BASE64.encode([byte]), u32::from(byte)));
        }
        input.into_bytes()
    }

    fn simple_spec(specials: Vec<(String, Rank)>) -> EncodingSpec {
        EncodingSpec {
            name: EncodingName::Cl100kBase,
            pattern: r"\w+|\s+|[^\w\s]+",
            special_tokens: specials,
        }
    }

    fn spec_with_specials(specials: &[(&str, Rank)]) -> EncodingSpec {
        simple_spec(
            specials
                .iter()
                .map(|(token, rank)| ((*token).to_owned(), *rank))
                .collect(),
        )
    }

    #[test]
    fn encoding_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Encoding>();
    }

    #[test]
    fn ordinary_roundtrip_uses_upstream_patterns() {
        let specs = [
            EncodingSpec::for_name(EncodingName::R50kBase),
            EncodingSpec::for_name(EncodingName::Cl100kBase),
            EncodingSpec::for_name(EncodingName::O200kBase),
        ];
        for spec in specs {
            let encoding = Encoding::from_rank_bytes(&byte_rank_file(), spec).expect("encoding");
            let text = "Hello, naïve 中文 👩‍💻!\r\n1234 'quoted'";
            let tokens = encoding.encode_ordinary(text).expect("encode");
            assert_eq!(encoding.decode_strict(&tokens).expect("decode"), text);
        }
    }

    #[test]
    fn encode_ordinary_treats_special_literals_as_text() {
        let encoding = Encoding::from_rank_bytes(
            &byte_rank_file(),
            spec_with_specials(&[("<|endoftext|>", 300)]),
        )
        .expect("encoding");
        let text = "a<|endoftext|>b";
        let tokens = encoding.encode_ordinary(text).expect("encode");
        assert!(!tokens.contains(&300));
        assert_eq!(encoding.decode_strict(&tokens).expect("decode"), text);
    }

    #[test]
    fn encode_allows_listed_specials() {
        let encoding = Encoding::from_rank_bytes(
            &byte_rank_file(),
            spec_with_specials(&[("<|endoftext|>", 300), ("<|endofprompt|>", 301)]),
        )
        .expect("encoding");
        let tokens = encoding
            .encode("a<|endoftext|>b", &["<|endoftext|>"])
            .expect("encode");
        assert!(tokens.contains(&300));
        assert_eq!(
            encoding.decode_strict(&tokens).expect("decode"),
            "a<|endoftext|>b"
        );
    }

    #[test]
    fn encode_rejects_unlisted_specials() {
        let encoding = Encoding::from_rank_bytes(
            &byte_rank_file(),
            spec_with_specials(&[("<|endoftext|>", 300), ("<|endofprompt|>", 301)]),
        )
        .expect("encoding");
        let err = encoding
            .encode("a<|endofprompt|>", &["<|endoftext|>"])
            .expect_err("disallowed");
        assert!(
            matches!(err, Error::DisallowedSpecialToken { ref token } if token == "<|endofprompt|>")
        );
    }

    #[test]
    fn encode_with_special_tokens_allows_all() {
        let encoding = Encoding::from_rank_bytes(
            &byte_rank_file(),
            spec_with_specials(&[("<|endoftext|>", 300), ("<|endofprompt|>", 301)]),
        )
        .expect("encoding");
        let tokens = encoding
            .encode_with_special_tokens("<|endoftext|><|endofprompt|>")
            .expect("encode");
        assert_eq!(tokens, vec![300, 301]);
    }

    #[test]
    fn special_matching_prefers_longest_literal() {
        let encoding = Encoding::from_rank_bytes(
            &byte_rank_file(),
            spec_with_specials(&[("<|a|>", 300), ("<|a|><|b|>", 301)]),
        )
        .expect("encoding");
        let tokens = encoding
            .encode_with_special_tokens("x<|a|><|b|>")
            .expect("encode");
        assert_eq!(tokens.last(), Some(&301));
        assert_eq!(
            encoding.decode_strict(&tokens).expect("decode"),
            "x<|a|><|b|>"
        );
    }

    #[test]
    fn decode_maps_special_ranks_back_to_literals() {
        let encoding = Encoding::from_rank_bytes(
            &byte_rank_file(),
            spec_with_specials(&[("<|endoftext|>", 300)]),
        )
        .expect("encoding");
        assert_eq!(
            encoding.decode_strict(&[300]).expect("decode"),
            "<|endoftext|>"
        );
    }

    #[test]
    fn rejects_empty_special_token() {
        let err = Encoding::from_rank_bytes(&byte_rank_file(), spec_with_specials(&[("", 300)]))
            .expect_err("empty special");
        assert!(matches!(err, Error::EmptySpecialToken { .. }));
    }

    #[test]
    fn rejects_duplicate_special_token_names() {
        let err = Encoding::from_rank_bytes(
            &byte_rank_file(),
            spec_with_specials(&[("<|endoftext|>", 300), ("<|endoftext|>", 301)]),
        )
        .expect_err("duplicate special");
        assert!(matches!(err, Error::DuplicateSpecialToken { .. }));
    }

    #[test]
    fn rejects_special_rank_collision() {
        let err = Encoding::from_rank_bytes(
            &byte_rank_file(),
            spec_with_specials(&[("<|endoftext|>", 0)]),
        )
        .expect_err("collision");
        assert!(matches!(
            err,
            Error::SpecialTokenRankCollision { rank: 0, .. }
        ));
    }

    #[test]
    fn parses_encoding_names() {
        for name in [
            EncodingName::R50kBase,
            EncodingName::P50kBase,
            EncodingName::P50kEdit,
            EncodingName::Cl100kBase,
            EncodingName::O200kBase,
            EncodingName::O200kHarmony,
        ] {
            assert_eq!(name.as_str().parse::<EncodingName>().expect("parse"), name);
        }
        assert!(matches!(
            "nope".parse::<EncodingName>(),
            Err(Error::UnknownEncoding { .. })
        ));
    }
}
