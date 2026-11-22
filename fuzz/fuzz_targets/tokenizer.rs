#![no_main]

use std::sync::OnceLock;

use atlas_tokenizer::rank_file::RankFile;
use atlas_tokenizer::{Encoding, EncodingName, EncodingSpec};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use libfuzzer_sys::fuzz_target;

static ENCODING: OnceLock<Encoding> = OnceLock::new();

fn encoding() -> &'static Encoding {
    ENCODING.get_or_init(|| {
        let mut input = String::new();
        for byte in 0..=u8::MAX {
            input.push_str(&format!("{} {}\n", BASE64.encode([byte]), u32::from(byte)));
        }
        Encoding::from_rank_bytes(
            input.as_bytes(),
            EncodingSpec::for_name(EncodingName::Cl100kBase),
        )
        .expect("synthetic encoding")
    })
}

fuzz_target!(|data: &[u8]| {
    // Malformed rank files must never panic.
    let _ = RankFile::from_bytes(data);

    if let Ok(text) = std::str::from_utf8(data) {
        let encoding = encoding();
        let tokens = encoding.encode_ordinary(text).expect("encode ordinary");
        assert_eq!(
            encoding.decode_bytes(&tokens).expect("decode bytes"),
            text.as_bytes()
        );
        let _ = encoding.encode_with_special_tokens(text);
    }
});
