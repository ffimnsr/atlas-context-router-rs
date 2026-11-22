#![cfg(feature = "download")]

//! Warm-cache startup gate.
//!
//! Ignored by default because it needs a verified upstream cache. Run with a
//! warm cache to record the lazy-init latency:
//!
//! ```sh
//! TIKTOKEN_CACHE_DIR=/tmp/data-gym-cache \
//!   cargo test --release -p atlas-tokenizer --test startup -- --ignored --nocapture
//! ```

use std::time::Instant;

use atlas_tokenizer::EncodingName;
use atlas_tokenizer::assets::asset_for;
use atlas_tokenizer::cache::{CacheConfig, read_verified};
use atlas_tokenizer::registry::EncodingLoader;

#[test]
#[ignore = "requires a warm verified upstream cache"]
fn warm_cache_o200k_lazy_init_p95_under_150ms() {
    let config = CacheConfig::from_env();
    let asset = asset_for(EncodingName::O200kBase);
    assert!(
        read_verified(&config, asset.url, asset.sha256)
            .expect("cache read")
            .is_some(),
        "startup gate requires a verified o200k_base cache entry in {}",
        config.dir().display()
    );

    let mut samples = Vec::with_capacity(10);
    for _ in 0..10 {
        let started = Instant::now();
        let loader = EncodingLoader::new(config.clone());
        let encoding = loader.load(EncodingName::O200kBase).expect("load encoding");
        let elapsed = started.elapsed();
        assert_eq!(encoding.name(), EncodingName::O200kBase);
        samples.push(elapsed);
    }
    samples.sort_unstable();
    let p95 = samples[(samples.len() * 95).div_ceil(100) - 1];
    eprintln!("warm-cache o200k_base lazy init p95: {p95:?}");
    assert!(
        p95.as_millis() <= 150,
        "warm-cache lazy init p95 {p95:?} exceeds the 150 ms gate"
    );
}
