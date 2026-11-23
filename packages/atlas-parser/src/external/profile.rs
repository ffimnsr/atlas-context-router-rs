use std::borrow::Cow;

use anyhow::Result;
use atlas_core::ParsedFile;

use crate::traits::{LangParser, ParseContext};

use super::{ExternalExtractor, supports_extension};

struct ExternalProfileParser {
    inner: Box<dyn LangParser>,
    language_name: Cow<'static, str>,
    extensions: Vec<String>,
}

impl LangParser for ExternalProfileParser {
    fn language_name(&self) -> Cow<'static, str> {
        self.language_name.clone()
    }

    fn supports(&self, path: &str) -> bool {
        supports_extension(path, &self.extensions)
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> (ParsedFile, Option<tree_sitter::Tree>) {
        self.inner.parse(ctx)
    }
}

impl ExternalExtractor {
    /// Build the built-in extractor for a loaded grammar after validating its
    /// static query against that grammar.
    pub(crate) fn build(
        self,
        language: tree_sitter::Language,
        extensions: Vec<String>,
    ) -> Result<Box<dyn LangParser>> {
        let inner: Box<dyn LangParser> = match self {
            Self::Php => {
                crate::lang::php::validate_query(&language)
                    .map_err(|error| anyhow::anyhow!("{error}"))?;
                Box::new(crate::lang::php::PhpParser::new(language))
            }
            Self::Cpp => {
                crate::lang::cpp::validate_query(&language)
                    .map_err(|error| anyhow::anyhow!("{error}"))?;
                Box::new(crate::lang::cpp::CppParser::new(language))
            }
            Self::CSharp => {
                crate::lang::csharp::validate_query(&language)
                    .map_err(|error| anyhow::anyhow!("{error}"))?;
                Box::new(crate::lang::csharp::CSharpParser::new(language))
            }
            Self::Java => {
                crate::lang::java::validate_query(&language)
                    .map_err(|error| anyhow::anyhow!("{error}"))?;
                Box::new(crate::lang::java::JavaParser::new(language))
            }
            Self::Scala => {
                crate::lang::scala::validate_query(&language)
                    .map_err(|error| anyhow::anyhow!("{error}"))?;
                Box::new(crate::lang::scala::ScalaParser::new(language))
            }
            Self::Ruby => {
                crate::lang::ruby::validate_query(&language)
                    .map_err(|error| anyhow::anyhow!("{error}"))?;
                Box::new(crate::lang::ruby::RubyParser::new(language))
            }
            Self::Bash => {
                crate::lang::bash::validate_query(&language)
                    .map_err(|error| anyhow::anyhow!("{error}"))?;
                Box::new(crate::lang::bash::BashParser::new(language))
            }
        };

        Ok(Box::new(ExternalProfileParser {
            inner,
            language_name: Cow::Borrowed(self.language_name()),
            extensions,
        }))
    }
}
