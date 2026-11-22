#![cfg(feature = "download")]

use atlas_tokenizer::Encoding;
use atlas_tokenizer::EncodingName;
use atlas_tokenizer::assets::{asset_for, sha256_hex};
use atlas_tokenizer::download::fetch_verified;

#[test]
#[ignore = "network access; run explicitly to refresh or verify upstream assets"]
fn upstream_assets_verify_and_load() {
    for name in [
        EncodingName::R50kBase,
        EncodingName::P50kBase,
        EncodingName::P50kEdit,
        EncodingName::Cl100kBase,
        EncodingName::O200kBase,
    ] {
        let asset = asset_for(name);
        let bytes = fetch_verified(asset.url, asset.sha256).expect("verified upstream asset");
        assert_eq!(sha256_hex(&bytes), asset.sha256);
        let encoding = Encoding::get(name).expect("load encoding");
        assert_eq!(encoding.name(), name);
        let text = "atlas tokenizer warm cache smoke";
        let tokens = encoding.encode_ordinary(text).expect("encode");
        assert_eq!(encoding.decode_strict(&tokens).expect("decode"), text);
    }
}
