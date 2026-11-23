use std::path::{Path, PathBuf};

use anyhow::{Result, ensure};
use tree_sitter_loader::{CompileConfig, Loader};

use super::ExternalParserConfig;

/// Load and ABI-validate the grammar configured by `config`.
pub(crate) fn load_external_language(
    config: &ExternalParserConfig,
) -> Result<tree_sitter::Language> {
    ensure!(
        config.grammar_dir.is_some() != config.lib_path.is_some(),
        "external parser '{}' must configure exactly one of grammar_dir or lib_path",
        config.language_name
    );

    let language = if let Some(lib_path) = &config.lib_path {
        let function = config
            .lib_function
            .clone()
            .or_else(|| {
                config
                    .extractor
                    .map(|extractor| extractor.default_lib_function().to_owned())
            })
            .unwrap_or_else(|| lib_function_name(&config.language_name));
        Loader::load_language(Path::new(lib_path), &function).map_err(|error| {
            anyhow::anyhow!(
                "cannot load external grammar library {} (symbol {function}): {error}",
                lib_path
            )
        })?
    } else {
        let grammar_dir = config.grammar_dir.as_deref().expect("validated above");
        let src_path = PathBuf::from(grammar_dir).join("src");
        ensure!(
            src_path.join("parser.c").exists(),
            "grammar_dir '{grammar_dir}' has no src/parser.c (point at a tree-sitter grammar checkout)"
        );
        let loader = match &config.grammar_lib_dir {
            Some(cache) => Loader::with_parser_lib_path(PathBuf::from(cache)),
            None => Loader::new()
                .map_err(|error| anyhow::anyhow!("cannot initialize grammar loader: {error}"))?,
        };
        loader
            .load_language_at_path(CompileConfig::new(&src_path, None, None))
            .map_err(|error| {
                anyhow::anyhow!(
                    "cannot build external grammar '{}' from {grammar_dir}: {error}",
                    config.language_name
                )
            })?
    };

    let abi = language.abi_version();
    ensure!(
        (tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=tree_sitter::LANGUAGE_VERSION)
            .contains(&abi),
        "external parser '{}' grammar ABI {abi} outside supported range {}..={}",
        config.language_name,
        tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION,
        tree_sitter::LANGUAGE_VERSION
    );

    Ok(language)
}

fn lib_function_name(language_name: &str) -> String {
    format!("tree_sitter_{}", language_name.replace('-', "_"))
}
