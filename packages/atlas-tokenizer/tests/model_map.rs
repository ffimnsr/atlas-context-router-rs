use atlas_tokenizer::EncodingName;
use atlas_tokenizer::error::Error;
use atlas_tokenizer::model::encoding_name_for_model;

#[test]
fn every_upstream_exact_model_resolves() {
    let cases = [
        ("o1", EncodingName::O200kBase),
        ("o3", EncodingName::O200kBase),
        ("o4-mini", EncodingName::O200kBase),
        ("gpt-4.1", EncodingName::O200kBase),
        ("gpt-4o", EncodingName::O200kBase),
        ("gpt-4", EncodingName::Cl100kBase),
        ("gpt-3.5-turbo", EncodingName::Cl100kBase),
        ("gpt-3.5", EncodingName::Cl100kBase),
        ("gpt-35-turbo", EncodingName::Cl100kBase),
        ("davinci-002", EncodingName::Cl100kBase),
        ("babbage-002", EncodingName::Cl100kBase),
        ("text-embedding-ada-002", EncodingName::Cl100kBase),
        ("text-embedding-3-small", EncodingName::Cl100kBase),
        ("text-embedding-3-large", EncodingName::Cl100kBase),
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
        ("code-davinci-002", EncodingName::P50kBase),
        ("code-davinci-001", EncodingName::P50kBase),
        ("code-cushman-002", EncodingName::P50kBase),
        ("code-cushman-001", EncodingName::P50kBase),
        ("davinci-codex", EncodingName::P50kBase),
        ("cushman-codex", EncodingName::P50kBase),
        ("text-davinci-edit-001", EncodingName::P50kEdit),
        ("code-davinci-edit-001", EncodingName::P50kEdit),
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
    for (model, expected) in cases {
        assert_eq!(
            encoding_name_for_model(model).expect(model),
            expected,
            "model {model}"
        );
    }
}

#[test]
fn upstream_prefixes_resolve_with_longest_match() {
    let cases = [
        ("o1-preview", EncodingName::O200kBase),
        ("o3-mini-2025-01-31", EncodingName::O200kBase),
        ("o4-mini-2025-04-16", EncodingName::O200kBase),
        ("gpt-4.5-preview", EncodingName::O200kBase),
        ("gpt-4.1-nano", EncodingName::O200kBase),
        ("chatgpt-4o-latest", EncodingName::O200kBase),
        ("gpt-4o-2024-05-13", EncodingName::O200kBase),
        ("gpt-4-0314", EncodingName::Cl100kBase),
        ("gpt-3.5-turbo-0301", EncodingName::Cl100kBase),
        ("gpt-35-turbo-16k", EncodingName::Cl100kBase),
        ("gpt-oss-120b", EncodingName::O200kHarmony),
        ("ft:gpt-4o-2024-08-06:org", EncodingName::O200kBase),
        ("ft:gpt-4-0613:org", EncodingName::Cl100kBase),
        ("ft:gpt-3.5-turbo-0125:org", EncodingName::Cl100kBase),
        ("ft:davinci-002:org", EncodingName::Cl100kBase),
        ("ft:babbage-002:org", EncodingName::Cl100kBase),
    ];
    for (model, expected) in cases {
        assert_eq!(
            encoding_name_for_model(model).expect(model),
            expected,
            "model {model}"
        );
    }
}

#[test]
fn unknown_models_error() {
    for model in ["", "not-a-model", "gpt2", "gpt-2"] {
        assert!(
            matches!(
                encoding_name_for_model(model),
                Err(Error::UnknownModel { .. })
            ),
            "model {model} should be unknown"
        );
    }
}
