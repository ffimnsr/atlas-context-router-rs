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
    pub fn seed_name(self) -> &'static str {
        match self {
            Self::Rust => "rust",
            Self::Go => "go",
            Self::Python => "python",
            Self::JavaScript => "javascript",
            Self::TypeScript => "typescript",
            Self::Json => "json",
            Self::Toml => "toml",
            Self::Html => "html",
            Self::Css => "css",
            Self::Bash => "bash",
            Self::Markdown => "markdown",
            Self::Java => "java",
            Self::CSharp => "csharp",
            Self::Php => "php",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::Scala => "scala",
            Self::Ruby => "ruby",
        }
    }

    pub fn from_seed_name(name: &str) -> Option<Self> {
        Some(match name {
            "rust" => Self::Rust,
            "go" => Self::Go,
            "python" => Self::Python,
            "javascript" => Self::JavaScript,
            "typescript" => Self::TypeScript,
            "json" => Self::Json,
            "toml" => Self::Toml,
            "html" => Self::Html,
            "css" => Self::Css,
            "bash" => Self::Bash,
            "markdown" => Self::Markdown,
            "java" => Self::Java,
            "csharp" => Self::CSharp,
            "php" => Self::Php,
            "c" => Self::C,
            "cpp" => Self::Cpp,
            "scala" => Self::Scala,
            "ruby" => Self::Ruby,
            _ => return None,
        })
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Rust => "rs",
            Self::Go => "go",
            Self::Python => "py",
            Self::JavaScript => "js",
            Self::TypeScript => "ts",
            Self::Json => "json",
            Self::Toml => "toml",
            Self::Html => "html",
            Self::Css => "css",
            Self::Bash => "sh",
            Self::Markdown => "md",
            Self::Java => "java",
            Self::CSharp => "cs",
            Self::Php => "php",
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::Scala => "scala",
            Self::Ruby => "rb",
        }
    }

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

#[derive(Arbitrary, Debug)]
pub struct SourceSeedCase {
    pub path_kind: SupportedPathKind,
    pub source: Vec<u8>,
}

pub fn parser_case_from_bytes(data: &[u8]) -> Option<ParserCase> {
    parse_parser_seed(data).or_else(|| ParserCase::arbitrary(&mut Unstructured::new(data)).ok())
}

pub fn source_seed_case_from_bytes(data: &[u8]) -> Option<SourceSeedCase> {
    parse_source_seed(data)
        .map(|(path_kind, source)| SourceSeedCase { path_kind, source })
        .or_else(|| SourceSeedCase::arbitrary(&mut Unstructured::new(data)).ok())
}

pub fn regex_case_from_bytes(data: &[u8]) -> Option<RegexCase> {
    parse_regex_seed(data).or_else(|| RegexCase::arbitrary(&mut Unstructured::new(data)).ok())
}

pub fn split_once_bytes<'a>(haystack: &'a [u8], needle: &[u8]) -> Option<(&'a [u8], &'a [u8])> {
    let index = haystack.windows(needle.len()).position(|window| window == needle)?;
    Some((&haystack[..index], &haystack[index + needle.len()..]))
}

pub fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn parse_parser_seed(data: &[u8]) -> Option<ParserCase> {
    let body = data.strip_prefix(b"ATLAS_PARSER_SEED\n")?;
    let (meta, rest) = split_once_bytes(body, b"\n===SOURCE===\n")?;
    let (source, next_source) = split_once_bytes(rest, b"\n===NEXT===\n")?;
    let (path_kind, reuse_old_tree) = parse_parser_meta(meta)?;

    Some(ParserCase {
        path_kind,
        source: source.to_vec(),
        next_source: next_source.to_vec(),
        reuse_old_tree,
    })
}

fn parse_source_seed(data: &[u8]) -> Option<(SupportedPathKind, Vec<u8>)> {
    let body = data.strip_prefix(b"ATLAS_SOURCE_SEED\n")?;
    let (meta, source) = split_once_bytes(body, b"\n===SOURCE===\n")?;
    let path_kind = parse_kind_meta(meta)?;
    Some((path_kind, source.to_vec()))
}

fn parse_regex_seed(data: &[u8]) -> Option<RegexCase> {
    let body = data.strip_prefix(b"ATLAS_REGEX_SEED\n===PATTERN===\n")?;
    let (pattern, value) = split_once_bytes(body, b"\n===VALUE===\n")?;
    Some(RegexCase {
        pattern: String::from_utf8(pattern.to_vec()).ok()?,
        value: String::from_utf8(value.to_vec()).ok()?,
    })
}

fn parse_parser_meta(meta: &[u8]) -> Option<(SupportedPathKind, bool)> {
    let meta = std::str::from_utf8(meta).ok()?;
    let mut path_kind = None;
    let mut reuse_old_tree = None;

    for line in meta.lines() {
        if let Some(value) = line.strip_prefix("kind=") {
            path_kind = SupportedPathKind::from_seed_name(value);
        } else if let Some(value) = line.strip_prefix("reuse_old_tree=") {
            reuse_old_tree = parse_bool_value(value);
        }
    }

    Some((path_kind?, reuse_old_tree?))
}

fn parse_kind_meta(meta: &[u8]) -> Option<SupportedPathKind> {
    let meta = std::str::from_utf8(meta).ok()?;
    meta.lines()
        .find_map(|line| line.strip_prefix("kind="))
        .and_then(SupportedPathKind::from_seed_name)
}

fn parse_bool_value(value: &str) -> Option<bool> {
    Some(match value {
        "true" => true,
        "false" => false,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ParserCase, RegexCase, SupportedPathKind, parser_case_from_bytes, regex_case_from_bytes,
        source_seed_case_from_bytes,
    };

    #[test]
    fn parser_seed_decodes_fixture_layout() {
        let seed = concat!(
            "ATLAS_PARSER_SEED\n",
            "kind=rust\n",
            "reuse_old_tree=true\n",
            "===SOURCE===\n",
            "fn main() {}\n",
            "===NEXT===\n",
            "fn main() { println!(\"hi\"); }\n"
        );

        let ParserCase {
            path_kind,
            source,
            next_source,
            reuse_old_tree,
        } = parser_case_from_bytes(seed.as_bytes()).expect("seed should decode");

        assert!(matches!(path_kind, SupportedPathKind::Rust));
        assert_eq!(source, b"fn main() {}\n");
        assert_eq!(next_source, b"fn main() { println!(\"hi\"); }\n");
        assert!(reuse_old_tree);
    }

    #[test]
    fn source_seed_decodes_fixture_layout() {
        let seed = concat!(
            "ATLAS_SOURCE_SEED\n",
            "kind=markdown\n",
            "===SOURCE===\n",
            "# hello\n"
        );

        let case = source_seed_case_from_bytes(seed.as_bytes()).expect("seed should decode");
        assert!(matches!(case.path_kind, SupportedPathKind::Markdown));
        assert_eq!(case.source, b"# hello\n");
    }

    #[test]
    fn regex_seed_decodes_unicode_samples() {
        let seed = concat!(
            "ATLAS_REGEX_SEED\n",
            "===PATTERN===\n",
            "^\\p{L}+$\n",
            "===VALUE===\n",
            "naive\u{301}\n"
        );

        let RegexCase { pattern, value } =
            regex_case_from_bytes(seed.as_bytes()).expect("seed should decode");
        assert_eq!(pattern, "^\\p{L}+$\n");
        assert_eq!(value, "naive\u{301}\n");
    }
}
