//! Pinned upstream asset metadata: URLs, sha256 digests, and commit.
//!
//! Values in this module are copied from
//! `tiktoken_ext/openai_public.py` at [`UPSTREAM_COMMIT`]. Do not regenerate
//! them from Atlas; update them by diffing the pinned upstream files.

use sha2::{Digest, Sha256};

use crate::encoding::EncodingName;

/// Upstream `tiktoken` commit that defines URLs, hashes, patterns, and
/// special tokens.
pub const UPSTREAM_COMMIT: &str = "3591ff175d6a80efbe4fcc7f0e219ddd4b8c52f1";

/// Exact upstream encoding-definition file at [`UPSTREAM_COMMIT`].
pub const UPSTREAM_OPENAI_PUBLIC_PY: &str = "https://github.com/openai/tiktoken/blob/3591ff175d6a80efbe4fcc7f0e219ddd4b8c52f1/tiktoken_ext/openai_public.py";

/// Exact upstream download/cache implementation at [`UPSTREAM_COMMIT`].
pub const UPSTREAM_LOAD_PY: &str = "https://github.com/openai/tiktoken/blob/3591ff175d6a80efbe4fcc7f0e219ddd4b8c52f1/tiktoken/load.py";

/// One vocabulary asset pinned by the upstream encoding definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Asset {
    /// Encoding that uses this asset.
    pub encoding: EncodingName,
    /// Upstream HTTPS download URL.
    pub url: &'static str,
    /// Upstream sha256 digest, lowercase hex.
    pub sha256: &'static str,
    /// Exact upstream file that defines this asset.
    pub upstream_file: &'static str,
}

const R50K_ASSET: Asset = Asset {
    encoding: EncodingName::R50kBase,
    url: "https://openaipublic.blob.core.windows.net/encodings/r50k_base.tiktoken",
    sha256: "306cd27f03c1a714eca7108e03d66b7dc042abe8c258b44c199a7ed9838dd930",
    upstream_file: UPSTREAM_OPENAI_PUBLIC_PY,
};

const P50K_ASSET: Asset = Asset {
    encoding: EncodingName::P50kBase,
    url: "https://openaipublic.blob.core.windows.net/encodings/p50k_base.tiktoken",
    sha256: "94b5ca7dff4d00767bc256fdd1b27e5b17361d7b8a5f968547f9f23eb70d2069",
    upstream_file: UPSTREAM_OPENAI_PUBLIC_PY,
};

const P50K_EDIT_ASSET: Asset = Asset {
    encoding: EncodingName::P50kEdit,
    url: P50K_ASSET.url,
    sha256: P50K_ASSET.sha256,
    upstream_file: UPSTREAM_OPENAI_PUBLIC_PY,
};

const CL100K_ASSET: Asset = Asset {
    encoding: EncodingName::Cl100kBase,
    url: "https://openaipublic.blob.core.windows.net/encodings/cl100k_base.tiktoken",
    sha256: "223921b76ee99bde995b7ff738513eef100fb51d18c93597a113bcffe865b2a7",
    upstream_file: UPSTREAM_OPENAI_PUBLIC_PY,
};

const O200K_ASSET: Asset = Asset {
    encoding: EncodingName::O200kBase,
    url: "https://openaipublic.blob.core.windows.net/encodings/o200k_base.tiktoken",
    sha256: "446a9538cb6c348e3516120d7c08b09f57c36495e2acfffe59a5bf8b0cfb1a2d",
    upstream_file: UPSTREAM_OPENAI_PUBLIC_PY,
};

const O200K_HARMONY_ASSET: Asset = Asset {
    encoding: EncodingName::O200kHarmony,
    url: O200K_ASSET.url,
    sha256: O200K_ASSET.sha256,
    upstream_file: UPSTREAM_OPENAI_PUBLIC_PY,
};

/// Return the pinned asset for an encoding.
pub fn asset_for(encoding: EncodingName) -> &'static Asset {
    match encoding {
        EncodingName::R50kBase => &R50K_ASSET,
        EncodingName::P50kBase => &P50K_ASSET,
        EncodingName::P50kEdit => &P50K_EDIT_ASSET,
        EncodingName::Cl100kBase => &CL100K_ASSET,
        EncodingName::O200kBase => &O200K_ASSET,
        EncodingName::O200kHarmony => &O200K_HARMONY_ASSET,
    }
}

/// Lowercase hex sha256 digest of arbitrary bytes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_encoding_has_a_pinned_asset() {
        for encoding in [
            EncodingName::R50kBase,
            EncodingName::P50kBase,
            EncodingName::P50kEdit,
            EncodingName::Cl100kBase,
            EncodingName::O200kBase,
            EncodingName::O200kHarmony,
        ] {
            let asset = asset_for(encoding);
            assert!(asset.url.starts_with("https://"));
            assert_eq!(asset.sha256.len(), 64);
            assert!(asset.upstream_file.contains(UPSTREAM_COMMIT));
        }
    }

    #[test]
    fn shared_assets_reuse_the_same_url_and_hash() {
        let p50k = asset_for(EncodingName::P50kBase);
        let edit = asset_for(EncodingName::P50kEdit);
        assert_eq!(p50k.url, edit.url);
        assert_eq!(p50k.sha256, edit.sha256);

        let o200k = asset_for(EncodingName::O200kBase);
        let harmony = asset_for(EncodingName::O200kHarmony);
        assert_eq!(o200k.url, harmony.url);
        assert_eq!(o200k.sha256, harmony.sha256);
    }

    #[test]
    fn sha256_hex_matches_known_value() {
        assert_eq!(
            sha256_hex(b"atlas"),
            "7c82602500857aa6ed0cf38c4c3e4ec645bdcaa82c00b9155eb08be100c778a9"
        );
    }
}
