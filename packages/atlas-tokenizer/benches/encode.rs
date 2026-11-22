use std::hint::black_box;
use std::sync::Arc;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};

use atlas_tokenizer::EncodingName;
use atlas_tokenizer::assets::asset_for;
use atlas_tokenizer::cache::{CacheConfig, read_verified};
use atlas_tokenizer::registry::EncodingLoader;

const TEXT: &str = concat!(
    "The quick brown fox jumps over the lazy dog. ",
    "Atlas context payloads mix symbols, code spans, and prose. ",
    "café 中文 مرحبا नमस्ते 1234 👩‍💻\n",
);

fn cached_encoding(name: EncodingName) -> Option<Arc<atlas_tokenizer::Encoding>> {
    let config = CacheConfig::from_env();
    let asset = asset_for(name);
    if read_verified(&config, asset.url, asset.sha256)
        .expect("cache read")
        .is_none()
    {
        eprintln!(
            "skipping {} bench: no verified asset in {} (set TIKTOKEN_CACHE_DIR)",
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

fn bench_encoding(c: &mut Criterion, name: EncodingName) {
    let Some(encoding) = cached_encoding(name) else {
        return;
    };
    let tokens = encoding.encode_ordinary(TEXT).expect("encode");
    let label = name.as_str();
    let mut group = c.benchmark_group(format!("tokenizer/{label}"));
    group.throughput(Throughput::Bytes(TEXT.len() as u64));
    group.bench_function("encode_ordinary", |b| {
        b.iter(|| encoding.encode_ordinary(black_box(TEXT)).expect("encode"))
    });
    group.throughput(Throughput::Elements(tokens.len() as u64));
    group.bench_function("decode", |b| {
        b.iter(|| encoding.decode_bytes(black_box(&tokens)).expect("decode"))
    });
    group.finish();
}

fn bench_all(c: &mut Criterion) {
    #[cfg(feature = "encoding-r50k")]
    bench_encoding(c, EncodingName::R50kBase);
    #[cfg(feature = "encoding-p50k")]
    bench_encoding(c, EncodingName::P50kBase);
    #[cfg(feature = "encoding-cl100k")]
    bench_encoding(c, EncodingName::Cl100kBase);
    #[cfg(feature = "encoding-o200k")]
    bench_encoding(c, EncodingName::O200kBase);
}

criterion_group!(benches, bench_all);
criterion_main!(benches);
