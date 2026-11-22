#![cfg(feature = "download")]

use atlas_tokenizer::EncodingName;
use atlas_tokenizer::assets::asset_for;
use atlas_tokenizer::cache::{CacheConfig, read_verified};
use atlas_tokenizer::registry::EncodingLoader;
use proptest::prelude::*;
use std::sync::Arc;

fn cached_encoding(name: EncodingName) -> Option<Arc<atlas_tokenizer::Encoding>> {
    let config = CacheConfig::from_env();
    let asset = asset_for(name);
    if read_verified(&config, asset.url, asset.sha256)
        .expect("cache read")
        .is_none()
    {
        eprintln!(
            "skipping {} roundtrip: no verified asset in {} (set TIKTOKEN_CACHE_DIR)",
            name.as_str(),
            config.dir().display()
        );
        return None;
    }
    Some(
        EncodingLoader::new(config)
            .load(name)
            .expect("load cached encoding"),
    )
}

fn roundtrip(name: EncodingName) {
    let Some(encoding) = cached_encoding(name) else {
        return;
    };
    let corpus = [
        "",
        "hello",
        "naïve 中文 مرحبا नमस्ते",
        "👩‍💻 \r\n\t 1234",
        "fn main() { println!(\"x\"); }",
        "repeat repeat repeat repeat repeat repeat",
    ];
    for text in corpus {
        let tokens = encoding.encode_ordinary(text).expect("encode");
        assert_eq!(
            encoding.decode_bytes(&tokens).expect("decode bytes"),
            text.as_bytes(),
            "{} roundtrip failed for {text:?}",
            name.as_str()
        );
        assert_eq!(encoding.decode_strict(&tokens).expect("decode utf8"), text);
    }
}

#[cfg(feature = "encoding-r50k")]
#[test]
fn roundtrip_r50k_base() {
    roundtrip(EncodingName::R50kBase);
}

#[cfg(feature = "encoding-p50k")]
#[test]
fn roundtrip_p50k_base() {
    roundtrip(EncodingName::P50kBase);
}

#[cfg(feature = "encoding-cl100k")]
#[test]
fn roundtrip_cl100k_base() {
    roundtrip(EncodingName::Cl100kBase);
}

#[cfg(feature = "encoding-o200k")]
#[test]
fn roundtrip_o200k_base() {
    roundtrip(EncodingName::O200kBase);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

    #[test]
    fn arbitrary_unicode_roundtrips_with_cached_cl100k(text in "\\PC{0,256}") {
        static ENCODING: std::sync::OnceLock<Option<Arc<atlas_tokenizer::Encoding>>> =
            std::sync::OnceLock::new();
        let Some(encoding) = ENCODING.get_or_init(|| cached_encoding(EncodingName::Cl100kBase)) else {
            return Ok(());
        };
        let tokens = encoding.encode_ordinary(&text).expect("encode");
        prop_assert_eq!(
            encoding.decode_bytes(&tokens).expect("decode"),
            text.as_bytes()
        );
    }
}
