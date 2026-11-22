#![cfg(feature = "download")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use atlas_tokenizer::assets::sha256_hex;
use atlas_tokenizer::cache::{CacheConfig, cache_key, read_verified, write_atomic};
use atlas_tokenizer::error::Error;
use atlas_tokenizer::registry::{AssetFetcher, EncodingLoader};
use atlas_tokenizer::{Encoding, EncodingName, Result};

#[derive(Debug)]
struct MemoryFetcher {
    bytes: Vec<u8>,
    calls: AtomicUsize,
}

impl MemoryFetcher {
    fn new(bytes: Vec<u8>) -> Arc<Self> {
        Arc::new(Self {
            bytes,
            calls: AtomicUsize::new(0),
        })
    }
}

impl AssetFetcher for MemoryFetcher {
    fn fetch(&self, _url: &str, _sha256: &str) -> Result<Vec<u8>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.bytes.clone())
    }
}

#[derive(Debug)]
struct PanicFetcher;

impl AssetFetcher for PanicFetcher {
    fn fetch(&self, _url: &str, _sha256: &str) -> Result<Vec<u8>> {
        panic!("warm cache must not call the network fetcher");
    }
}

fn rank_bytes() -> Vec<u8> {
    let mut entries: Vec<(Vec<u8>, u32)> = (0..=u8::MAX)
        .map(|byte| (vec![byte], u32::from(byte)))
        .collect();
    entries.push((b"hello".to_vec(), 256));
    entries.push((b" world".to_vec(), 257));
    let mut input = String::new();
    for (token, rank) in entries {
        input.push_str(&format!("{} {rank}\n", BASE64.encode(token)));
    }
    input.into_bytes()
}

const FIXTURE_URL: &str = "https://assets.test/fixture.tiktoken";

#[test]
fn cache_key_matches_upstream_sha1() {
    assert_eq!(
        cache_key("https://openaipublic.blob.core.windows.net/encodings/r50k_base.tiktoken"),
        "0ea1e91bbb3a60f729a8dc8f777fd2fc07cd8df4"
    );
}

#[test]
fn downloaded_asset_is_cached_and_warm_cache_works_offline() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = CacheConfig::with_dir(dir.path());
    let bytes = rank_bytes();
    let sha = sha256_hex(&bytes);
    let fetcher = MemoryFetcher::new(bytes.clone());

    let loader = EncodingLoader::with_fetcher(config.clone(), fetcher.clone());
    let encoding = loader
        .load_asset(FIXTURE_URL, &sha, EncodingName::Cl100kBase)
        .expect("load");
    assert_eq!(fetcher.calls.load(Ordering::SeqCst), 1);
    let tokens = encoding.encode_ordinary("hello world").expect("encode");
    assert_eq!(
        encoding.decode_strict(&tokens).expect("decode"),
        "hello world"
    );

    drop(loader);
    let offline = EncodingLoader::with_fetcher(config, Arc::new(PanicFetcher));
    let encoding = offline
        .load_asset(FIXTURE_URL, &sha, EncodingName::Cl100kBase)
        .expect("warm cache load");
    let tokens = encoding.encode_ordinary("hello world").expect("encode");
    assert_eq!(
        encoding.decode_strict(&tokens).expect("decode"),
        "hello world"
    );
}

#[test]
fn corrupt_cache_entry_is_replaced_after_refetch() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = CacheConfig::with_dir(dir.path());
    write_atomic(&config, FIXTURE_URL, b"corrupt").expect("seed corrupt entry");
    let bytes = rank_bytes();
    let sha = sha256_hex(&bytes);
    let fetcher = MemoryFetcher::new(bytes);

    let loader = EncodingLoader::with_fetcher(config.clone(), fetcher.clone());
    loader
        .load_asset(FIXTURE_URL, &sha, EncodingName::Cl100kBase)
        .expect("load after corrupt entry");
    assert_eq!(fetcher.calls.load(Ordering::SeqCst), 1);
    assert!(
        read_verified(&config, FIXTURE_URL, &sha)
            .expect("read")
            .is_some()
    );
}

#[test]
fn fetcher_hash_mismatch_is_rejected_and_not_cached() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = CacheConfig::with_dir(dir.path());
    let expected = sha256_hex(&rank_bytes());
    let fetcher = MemoryFetcher::new(b"wrong bytes".to_vec());

    let loader = EncodingLoader::with_fetcher(config.clone(), fetcher);
    let err = loader
        .load_asset(FIXTURE_URL, &expected, EncodingName::Cl100kBase)
        .expect_err("hash mismatch");
    assert!(matches!(err, Error::HashMismatch { .. }));
    assert!(
        read_verified(&config, FIXTURE_URL, &expected)
            .expect("read")
            .is_none()
    );
}

#[test]
fn explicit_unwritable_cache_is_an_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("not-a-directory");
    std::fs::write(&file, b"x").expect("write file");
    let config = CacheConfig::with_dir(&file);
    let bytes = rank_bytes();
    let sha = sha256_hex(&bytes);
    let loader = EncodingLoader::with_fetcher(config, MemoryFetcher::new(bytes));

    let err = loader
        .load_asset(FIXTURE_URL, &sha, EncodingName::Cl100kBase)
        .expect_err("explicit cache must error");
    assert!(matches!(err, Error::CacheIo { .. }));
}

#[test]
fn tokenizer_file_roundtrips_without_network() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("custom.tiktoken");
    std::fs::write(&path, rank_bytes()).expect("write rank file");
    let encoding = Encoding::from_rank_file_with_name(&path, EncodingName::Cl100kBase)
        .expect("load rank file");
    let text = "hello world, atlas";
    let tokens = encoding.encode_ordinary(text).expect("encode");
    assert_eq!(encoding.decode_strict(&tokens).expect("decode"), text);
}
