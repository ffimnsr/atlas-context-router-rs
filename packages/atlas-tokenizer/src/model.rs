//! Model name to encoding mapping.
//!
//! Mirrors upstream `tiktoken/model.py` at the pinned commit. Exact model
//! names win first; otherwise the longest matching prefix is used, which is
//! the upstream behavior for its overlapping `ft:gpt-4o` / `ft:gpt-4` style
//! prefixes. `gpt2` and `gpt-2` are intentionally unsupported in v1 because
//! upstream builds GPT-2 from `vocab.bpe` and `encoder.json`.

use crate::encoding::EncodingName;
use crate::error::{Error, Result};

const MODEL_TO_ENCODING: &[(&str, EncodingName)] = &[
    // reasoning
    ("o1", EncodingName::O200kBase),
    ("o3", EncodingName::O200kBase),
    ("o4-mini", EncodingName::O200kBase),
    // chat
    ("gpt-4.1", EncodingName::O200kBase),
    ("gpt-4o", EncodingName::O200kBase),
    ("gpt-4", EncodingName::Cl100kBase),
    ("gpt-3.5-turbo", EncodingName::Cl100kBase),
    ("gpt-3.5", EncodingName::Cl100kBase),
    ("gpt-35-turbo", EncodingName::Cl100kBase),
    // base
    ("davinci-002", EncodingName::Cl100kBase),
    ("babbage-002", EncodingName::Cl100kBase),
    // embeddings
    ("text-embedding-ada-002", EncodingName::Cl100kBase),
    ("text-embedding-3-small", EncodingName::Cl100kBase),
    ("text-embedding-3-large", EncodingName::Cl100kBase),
    // deprecated text
    ("text-davinci-003", EncodingName::P50kBase),
    ("text-davinci-002", EncodingName::P50kBase),
    ("text-davinci-001", EncodingName::R50kBase),
    ("text-curie-001", EncodingName::R50kBase),
    ("text-babbage-001", EncodingName::R50kBase),
    ("text-ada-001", EncodingName::R50kBase),
    ("davinci", EncodingName::R50kBase),
    ("curie", EncodingName::R50kBase),
    ("babbage", EncodingName::R50kBase),
    ("ada", EncodingName::R50kBase),
    // deprecated code
    ("code-davinci-002", EncodingName::P50kBase),
    ("code-davinci-001", EncodingName::P50kBase),
    ("code-cushman-002", EncodingName::P50kBase),
    ("code-cushman-001", EncodingName::P50kBase),
    ("davinci-codex", EncodingName::P50kBase),
    ("cushman-codex", EncodingName::P50kBase),
    // deprecated edit
    ("text-davinci-edit-001", EncodingName::P50kEdit),
    ("code-davinci-edit-001", EncodingName::P50kEdit),
    // deprecated old embeddings
    ("text-similarity-davinci-001", EncodingName::R50kBase),
    ("text-similarity-curie-001", EncodingName::R50kBase),
    ("text-similarity-babbage-001", EncodingName::R50kBase),
    ("text-similarity-ada-001", EncodingName::R50kBase),
    ("text-search-davinci-doc-001", EncodingName::R50kBase),
    ("text-search-curie-doc-001", EncodingName::R50kBase),
    ("text-search-babbage-doc-001", EncodingName::R50kBase),
    ("text-search-ada-doc-001", EncodingName::R50kBase),
    ("code-search-babbage-code-001", EncodingName::R50kBase),
    ("code-search-ada-code-001", EncodingName::R50kBase),
];

const MODEL_PREFIX_TO_ENCODING: &[(&str, EncodingName)] = &[
    ("o1-", EncodingName::O200kBase),
    ("o3-", EncodingName::O200kBase),
    ("o4-mini-", EncodingName::O200kBase),
    // chat
    ("gpt-4.5-", EncodingName::O200kBase),
    ("gpt-4.1-", EncodingName::O200kBase),
    ("chatgpt-4o-", EncodingName::O200kBase),
    ("gpt-4o-", EncodingName::O200kBase),
    ("gpt-4-", EncodingName::Cl100kBase),
    ("gpt-3.5-turbo-", EncodingName::Cl100kBase),
    ("gpt-35-turbo-", EncodingName::Cl100kBase),
    ("gpt-oss-", EncodingName::O200kHarmony),
    // fine-tuned
    ("ft:gpt-4o", EncodingName::O200kBase),
    ("ft:gpt-4", EncodingName::Cl100kBase),
    ("ft:gpt-3.5-turbo", EncodingName::Cl100kBase),
    ("ft:davinci-002", EncodingName::Cl100kBase),
    ("ft:babbage-002", EncodingName::Cl100kBase),
];

/// Resolve a model name to its upstream encoding name.
///
/// Exact matches are checked first. Otherwise the longest matching prefix
/// wins. Unknown models return [`Error::UnknownModel`].
pub fn encoding_name_for_model(model: &str) -> Result<EncodingName> {
    if let Some((_, encoding)) = MODEL_TO_ENCODING
        .iter()
        .find(|(candidate, _)| *candidate == model)
    {
        return Ok(*encoding);
    }
    if let Some((_, encoding)) = MODEL_PREFIX_TO_ENCODING
        .iter()
        .filter(|(prefix, _)| model.starts_with(prefix))
        .max_by_key(|(prefix, _)| prefix.len())
    {
        return Ok(*encoding);
    }
    Err(Error::unknown_model(model))
}
