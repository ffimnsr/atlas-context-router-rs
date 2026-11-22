//! Optional rayon batch encoding helpers.
//!
//! Enabled by the `batch` feature. Batch calls preserve the same deterministic
//! output as sequential encoding; rayon only fans out independent pieces.

use rayon::prelude::*;

use crate::Rank;
use crate::encoding::Encoding;
use crate::error::Result;

impl Encoding {
    /// Encode many ordinary texts in parallel, preserving input order.
    pub fn encode_ordinary_batch(&self, texts: &[&str]) -> Result<Vec<Vec<Rank>>> {
        texts
            .par_iter()
            .map(|text| self.encode_ordinary(text))
            .collect()
    }

    /// Encode many texts with all special tokens allowed, preserving input
    /// order.
    pub fn encode_with_special_tokens_batch(&self, texts: &[&str]) -> Result<Vec<Vec<Rank>>> {
        texts
            .par_iter()
            .map(|text| self.encode_with_special_tokens(text))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;

    use super::*;
    use crate::{EncodingName, EncodingSpec};

    fn byte_rank_file() -> Vec<u8> {
        let mut input = String::new();
        for byte in 0..=u8::MAX {
            input.push_str(&format!("{} {}\n", BASE64.encode([byte]), u32::from(byte)));
        }
        input.into_bytes()
    }

    #[test]
    fn batch_matches_sequential_encoding() {
        let encoding = Encoding::from_rank_bytes(
            &byte_rank_file(),
            EncodingSpec::for_name(EncodingName::Cl100kBase),
        )
        .expect("encoding");
        let texts = ["", "hello", "café 中文 👩‍💻", "repeat repeat repeat"];
        let batch = encoding.encode_ordinary_batch(&texts).expect("batch");
        for (index, text) in texts.iter().enumerate() {
            assert_eq!(
                batch[index],
                encoding.encode_ordinary(text).expect("sequential")
            );
        }
    }
}
