use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;

use atlas_tokenizer::error::Error;
use atlas_tokenizer::{Encoding, EncodingName, EncodingSpec, Rank};

fn byte_rank_file() -> Vec<u8> {
    let mut input = String::new();
    for byte in 0..=u8::MAX {
        input.push_str(&format!("{} {}\n", BASE64.encode([byte]), u32::from(byte)));
    }
    input.into_bytes()
}

fn custom_spec(specials: &[(&str, Rank)]) -> EncodingSpec {
    EncodingSpec {
        name: EncodingName::Cl100kBase,
        pattern: r"\w+|\s+|[^\w\s]+",
        special_tokens: specials
            .iter()
            .map(|(token, rank)| ((*token).to_owned(), *rank))
            .collect(),
    }
}

#[test]
fn upstream_specials_decode_to_literals() {
    let cases = [
        (EncodingName::R50kBase, "<|endoftext|>", 50256),
        (EncodingName::P50kBase, "<|endoftext|>", 50256),
        (EncodingName::P50kEdit, "<|fim_prefix|>", 50281),
        (EncodingName::Cl100kBase, "<|endofprompt|>", 100276),
        (EncodingName::O200kBase, "<|endofprompt|>", 200018),
    ];
    for (name, literal, rank) in cases {
        let encoding = Encoding::from_rank_bytes(&byte_rank_file(), EncodingSpec::for_name(name))
            .expect("encoding");
        let encoded = encoding
            .encode_with_special_tokens(literal)
            .expect("encode special");
        assert_eq!(encoded, vec![rank], "encoding {}", name.as_str());
        assert_eq!(
            encoding.decode_strict(&encoded).expect("decode"),
            literal,
            "decoding {}",
            name.as_str()
        );
    }
}

#[test]
fn disallowed_special_errors() {
    let encoding = Encoding::from_rank_bytes(
        &byte_rank_file(),
        custom_spec(&[("<|endoftext|>", 300), ("<|fim_prefix|>", 301)]),
    )
    .expect("encoding");
    let err = encoding
        .encode("x<|fim_prefix|>y", &["<|endoftext|>"])
        .expect_err("disallowed special");
    assert!(matches!(
        err,
        Error::DisallowedSpecialToken { ref token } if token == "<|fim_prefix|>"
    ));
}

#[test]
fn encode_ordinary_keeps_special_literals_as_text() {
    let encoding =
        Encoding::from_rank_bytes(&byte_rank_file(), custom_spec(&[("<|endoftext|>", 300)]))
            .expect("encoding");
    let text = "a<|endoftext|>b";
    let tokens = encoding.encode_ordinary(text).expect("encode ordinary");
    assert!(!tokens.contains(&300));
    assert_eq!(encoding.decode_strict(&tokens).expect("decode"), text);
}

#[test]
fn longest_special_literal_wins() {
    let encoding = Encoding::from_rank_bytes(
        &byte_rank_file(),
        custom_spec(&[("<|end|>", 300), ("<|end|><|start|>", 301)]),
    )
    .expect("encoding");
    let tokens = encoding
        .encode_with_special_tokens("<|end|><|start|>")
        .expect("encode");
    assert_eq!(tokens, vec![301]);
}

#[test]
fn all_keyword_allows_every_special() {
    let encoding = Encoding::from_rank_bytes(
        &byte_rank_file(),
        custom_spec(&[("<|endoftext|>", 300), ("<|fim_prefix|>", 301)]),
    )
    .expect("encoding");
    let tokens = encoding
        .encode("<|endoftext|><|fim_prefix|>", &["all"])
        .expect("encode");
    assert_eq!(tokens, vec![300, 301]);
}

#[test]
fn empty_and_duplicate_specials_are_rejected() {
    let empty = Encoding::from_rank_bytes(&byte_rank_file(), custom_spec(&[("", 300)]))
        .expect_err("empty special");
    assert!(matches!(empty, Error::EmptySpecialToken { .. }));

    let duplicate = Encoding::from_rank_bytes(
        &byte_rank_file(),
        custom_spec(&[("<|endoftext|>", 300), ("<|endoftext|>", 301)]),
    )
    .expect_err("duplicate special");
    assert!(matches!(duplicate, Error::DuplicateSpecialToken { .. }));
}
