use arbitrary::{Arbitrary, Unstructured};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug)]
pub enum SupportedPathKind {
    Rust,
    Go,
    Python,
    JavaScript,
    TypeScript,
    Json,
    Toml,
    Html,
    Css,
    Bash,
    Markdown,
    Java,
    CSharp,
    Php,
    C,
    Cpp,
    Scala,
    Ruby,
}

impl<'a> Arbitrary<'a> for SupportedPathKind {
    fn arbitrary(u: &mut Unstructured<'a>) -> arbitrary::Result<Self> {
        Ok(match u.int_in_range(0..=17u8)? {
            0 => Self::Rust,
            1 => Self::Go,
            2 => Self::Python,
            3 => Self::JavaScript,
            4 => Self::TypeScript,
            5 => Self::Json,
            6 => Self::Toml,
            7 => Self::Html,
            8 => Self::Css,
            9 => Self::Bash,
            10 => Self::Markdown,
            11 => Self::Java,
            12 => Self::CSharp,
            13 => Self::Php,
            14 => Self::C,
            15 => Self::Cpp,
            16 => Self::Scala,
            _ => Self::Ruby,
        })
    }
}

impl SupportedPathKind {
    pub fn rel_path(self) -> &'static str {
        match self {
            Self::Rust => "src/fuzz.rs",
            Self::Go => "src/fuzz.go",
            Self::Python => "src/fuzz.py",
            Self::JavaScript => "src/fuzz.js",
            Self::TypeScript => "src/fuzz.ts",
            Self::Json => "src/fuzz.json",
            Self::Toml => "src/fuzz.toml",
            Self::Html => "src/fuzz.html",
            Self::Css => "src/fuzz.css",
            Self::Bash => "src/fuzz.sh",
            Self::Markdown => "src/fuzz.md",
            Self::Java => "src/Fuzz.java",
            Self::CSharp => "src/Fuzz.cs",
            Self::Php => "src/fuzz.php",
            Self::C => "src/fuzz.c",
            Self::Cpp => "src/fuzz.cpp",
            Self::Scala => "src/Fuzz.scala",
            Self::Ruby => "src/fuzz.rb",
        }
    }
}

#[derive(Arbitrary, Debug)]
pub struct ParserCase {
    pub path_kind: SupportedPathKind,
    pub source: Vec<u8>,
    pub next_source: Vec<u8>,
    pub reuse_old_tree: bool,
}

#[derive(Arbitrary, Debug)]
pub struct RegexCase {
    pub pattern: String,
    pub value: String,
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
