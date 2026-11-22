#![cfg(feature = "download")]

use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Deserialize;

use atlas_tokenizer::EncodingName;
use atlas_tokenizer::assets::asset_for;
use atlas_tokenizer::cache::{CacheConfig, read_verified};
use atlas_tokenizer::registry::EncodingLoader;

#[derive(Debug, Deserialize)]
struct Golden {
    text: String,
    tokens: Vec<u32>,
    bytes: String,
    special: bool,
}

fn golden_path(name: EncodingName) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(format!("{}.jsonl", name.as_str()))
}

fn read_goldens(name: EncodingName) -> Vec<Golden> {
    let path = golden_path(name);
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("cannot read golden {}: {err}", path.display()));
    contents
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("golden JSONL row"))
        .collect()
}

/// Load the encoding only when the verified upstream asset is already cached.
/// Default test runs stay offline; with a warm upstream cache this exercises
/// exact parity, and without one the test reports a skip instead of fetching.
fn cached_encoding(name: EncodingName) -> Option<Arc<atlas_tokenizer::Encoding>> {
    let config = CacheConfig::from_env();
    let asset = asset_for(name);
    if read_verified(&config, asset.url, asset.sha256)
        .expect("cache read")
        .is_none()
    {
        eprintln!(
            "skipping {} parity: no verified asset in {} (set TIKTOKEN_CACHE_DIR)",
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

fn assert_parity(name: EncodingName) {
    let Some(encoding) = cached_encoding(name) else {
        return;
    };
    let goldens = read_goldens(name);
    assert!(!goldens.is_empty(), "golden corpus must not be empty");
    for (index, golden) in goldens.iter().enumerate() {
        let tokens = if golden.special {
            encoding
                .encode_with_special_tokens(&golden.text)
                .expect("encode special")
        } else {
            encoding.encode_ordinary(&golden.text).expect("encode")
        };
        assert_eq!(
            tokens,
            golden.tokens,
            "{} token mismatch at case {index}: {:?}",
            name.as_str(),
            golden.text
        );
        let expected_bytes = BASE64.decode(&golden.bytes).expect("golden bytes");
        assert_eq!(expected_bytes, golden.text.as_bytes());
        assert_eq!(
            encoding.decode_bytes(&tokens).expect("decode bytes"),
            expected_bytes,
            "{} byte roundtrip mismatch at case {index}",
            name.as_str()
        );
        assert_eq!(
            encoding.decode_strict(&tokens).expect("decode utf8"),
            golden.text,
            "{} text roundtrip mismatch at case {index}",
            name.as_str()
        );
    }
}

#[cfg(feature = "encoding-r50k")]
#[test]
fn parity_r50k_base() {
    assert_parity(EncodingName::R50kBase);
}

#[cfg(feature = "encoding-p50k")]
#[test]
fn parity_p50k_base() {
    assert_parity(EncodingName::P50kBase);
}

#[cfg(feature = "encoding-p50k")]
#[test]
fn parity_p50k_edit() {
    assert_parity(EncodingName::P50kEdit);
}

#[cfg(feature = "encoding-cl100k")]
#[test]
fn parity_cl100k_base() {
    assert_parity(EncodingName::Cl100kBase);
}

#[cfg(feature = "encoding-o200k")]
#[test]
fn parity_o200k_base() {
    assert_parity(EncodingName::O200kBase);
}

#[cfg(feature = "encoding-o200k-harmony")]
#[test]
fn parity_o200k_harmony() {
    assert_parity(EncodingName::O200kHarmony);
}
