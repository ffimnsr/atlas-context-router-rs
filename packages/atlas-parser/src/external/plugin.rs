//! Out-of-process parser plugin runner.
//!
//! Each file is parsed by spawning the configured argv vector, writing one JSON
//! request to stdin, and validating the JSON response. Plugin execution is
//! trusted local code: Atlas provides no OS sandbox.

use std::borrow::Cow;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context, Result};
use atlas_core::ParsedFile;
use wait_timeout::ChildExt;

use crate::ast_helpers::file_node;
use crate::traits::{LangParser, ParseContext};

use super::protocol::{DEFAULT_MAX_RESPONSE_BYTES, DEFAULT_TIMEOUT_MS, PLUGIN_PROTOCOL};
use super::{ExternalParserConfig, supports_extension};

const STDERR_CAPTURE_BYTES: usize = 8 * 1024;

pub(super) struct PluginExternalParser {
    language_name: String,
    extensions: Vec<String>,
    command: String,
    args: Vec<String>,
    working_dir: Option<PathBuf>,
    protocol: String,
    timeout: Duration,
    max_response_bytes: usize,
}

impl PluginExternalParser {
    pub(super) fn new(config: &ExternalParserConfig) -> Result<Self> {
        let command = config
            .command
            .clone()
            .context("plugin mode requires a command")?;
        Ok(Self {
            language_name: config.language_name.clone(),
            extensions: config.extensions.clone(),
            command,
            args: config.args.clone(),
            working_dir: config.working_dir.as_ref().map(PathBuf::from),
            protocol: config
                .protocol
                .clone()
                .unwrap_or_else(|| PLUGIN_PROTOCOL.to_owned()),
            timeout: Duration::from_millis(config.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS)),
            max_response_bytes: usize::try_from(
                config
                    .max_response_bytes
                    .unwrap_or(DEFAULT_MAX_RESPONSE_BYTES),
            )
            .unwrap_or(usize::MAX),
        })
    }

    fn run(&self, request: &[u8]) -> Result<Vec<u8>, String> {
        let mut command = Command::new(&self.command);
        command
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(working_dir) = &self.working_dir {
            command.current_dir(working_dir);
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("cannot spawn '{}': {error}", self.command))?;

        let mut stdin = child.stdin.take();
        let request = request.to_vec();
        let writer = std::thread::spawn(move || {
            if let Some(mut stdin) = stdin.take() {
                use std::io::Write;
                let _ = stdin.write_all(&request);
            }
        });

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "plugin stdout unavailable".to_owned())?;
        let cap = self.max_response_bytes;
        let reader = std::thread::spawn(move || drain_capped(stdout, cap));
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "plugin stderr unavailable".to_owned())?;
        let stderr_reader = std::thread::spawn(move || drain_capped(stderr, STDERR_CAPTURE_BYTES));

        let status = match child.wait_timeout(self.timeout) {
            Ok(Some(status)) => status,
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = writer.join();
                let _ = reader.join();
                let _ = stderr_reader.join();
                return Err(format!("timed out after {} ms", self.timeout.as_millis()));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = writer.join();
                let _ = reader.join();
                let _ = stderr_reader.join();
                return Err(format!("cannot wait for plugin: {error}"));
            }
        };

        let _ = writer.join();
        let (stdout_bytes, stdout_total) = reader
            .join()
            .map_err(|_| "plugin stdout reader panicked".to_owned())?
            .map_err(|error| format!("cannot read plugin stdout: {error}"))?;
        let (stderr_bytes, _) = stderr_reader
            .join()
            .map_err(|_| "plugin stderr reader panicked".to_owned())?
            .map_err(|error| format!("cannot read plugin stderr: {error}"))?;

        if stdout_total > self.max_response_bytes as u64 {
            return Err(format!(
                "response exceeded {} bytes (got {stdout_total})",
                self.max_response_bytes
            ));
        }
        if !status.success() {
            let stderr_text = String::from_utf8_lossy(&stderr_bytes);
            let detail = stderr_text.trim();
            return Err(if detail.is_empty() {
                format!("plugin exited with {status}")
            } else {
                format!("plugin exited with {status}: {detail}")
            });
        }
        Ok(stdout_bytes)
    }
}

impl LangParser for PluginExternalParser {
    fn language_name(&self) -> Cow<'static, str> {
        Cow::Owned(self.language_name.clone())
    }

    fn supports(&self, path: &str) -> bool {
        supports_extension(path, &self.extensions)
    }

    fn parse_checked(
        &self,
        ctx: &ParseContext<'_>,
    ) -> Result<(ParsedFile, Option<tree_sitter::Tree>), String> {
        let request = super::protocol::encode_request(
            &self.protocol,
            &self.language_name,
            ctx.rel_path,
            ctx.file_hash,
            ctx.source,
        )
        .map_err(|error| format!("plugin '{}' request failed: {error}", self.language_name))?;
        let response = self.run(&request).map_err(|error| {
            format!(
                "plugin '{}' failed for '{}': {error}",
                self.language_name, ctx.rel_path
            )
        })?;
        let parsed = super::protocol::decode_response(
            &response,
            &self.protocol,
            &self.language_name,
            ctx.rel_path,
            ctx.file_hash,
        )
        .map_err(|error| {
            format!(
                "plugin '{}' returned invalid output for '{}': {error}",
                self.language_name, ctx.rel_path
            )
        })?;
        Ok((parsed, None))
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> (ParsedFile, Option<tree_sitter::Tree>) {
        match self.parse_checked(ctx) {
            Ok(result) => result,
            Err(error) => {
                tracing::warn!(
                    path = ctx.rel_path,
                    error = %error,
                    "external plugin parse failed"
                );
                (empty_parsed_file(ctx, &self.language_name), None)
            }
        }
    }
}

fn drain_capped<R: Read>(mut reader: R, cap: usize) -> std::io::Result<(Vec<u8>, u64)> {
    let mut buffer = Vec::new();
    let mut total = 0u64;
    let mut chunk = [0u8; 8192];
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            return Ok((buffer, total));
        }
        total += read as u64;
        if buffer.len() < cap {
            let keep = read.min(cap - buffer.len());
            buffer.extend_from_slice(&chunk[..keep]);
        }
    }
}

fn empty_parsed_file(ctx: &ParseContext<'_>, language_name: &str) -> ParsedFile {
    let line_count = ctx.source.iter().filter(|&&byte| byte == b'\n').count() as u32 + 1;
    ParsedFile {
        path: ctx.rel_path.to_owned(),
        language: Some(language_name.to_owned()),
        hash: ctx.file_hash.to_owned(),
        size: Some(ctx.source.len() as i64),
        nodes: vec![file_node(
            ctx.rel_path,
            ctx.file_hash,
            line_count,
            language_name,
        )],
        edges: Vec::new(),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use atlas_core::NodeId;
    use serde_json::json;
    use std::fs;

    const PATH: &str = "src/main.cust";
    const HASH: &str = "hash-success";
    const LANGUAGE: &str = "custom";

    fn response(path: &str, hash: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "protocol": PLUGIN_PROTOCOL,
            "parsed_file": {
                "path": path,
                "language": "plugin-language",
                "hash": hash,
                "size": 12,
                "nodes": [{
                    "id": 99,
                    "kind": "function",
                    "name": "greet",
                    "qualified_name": "src/main.cust::function::greet",
                    "file_path": path,
                    "line_start": 1,
                    "line_end": 1,
                    "language": "plugin-language",
                    "parent_name": null,
                    "params": null,
                    "return_type": null,
                    "modifiers": null,
                    "is_test": false,
                    "file_hash": hash,
                    "extra_json": null,
                }],
                "edges": [],
            }
        }))
        .expect("serialize plugin response")
    }

    fn config(
        command: &str,
        args: Vec<String>,
        timeout_ms: u64,
        max_response_bytes: u64,
    ) -> ExternalParserConfig {
        ExternalParserConfig {
            language_name: LANGUAGE.to_owned(),
            extensions: vec!["cust".to_owned()],
            command: Some(command.to_owned()),
            args,
            protocol: Some(PLUGIN_PROTOCOL.to_owned()),
            timeout_ms: Some(timeout_ms),
            max_response_bytes: Some(max_response_bytes),
            ..Default::default()
        }
    }

    fn context(source: &'static [u8]) -> ParseContext<'static> {
        ParseContext {
            rel_path: PATH,
            file_hash: HASH,
            source,
            old_tree: None,
        }
    }

    fn run(config: &ExternalParserConfig, ctx: &ParseContext<'_>) -> Result<ParsedFile, String> {
        PluginExternalParser::new(config)
            .expect("plugin parser")
            .parse_checked(ctx)
            .map(|(parsed, _)| parsed)
    }

    #[test]
    fn plugin_success_returns_validated_parsed_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let response_path = dir.path().join("response.json");
        fs::write(&response_path, response(PATH, HASH)).expect("write response");

        let parsed = run(
            &config(
                "cat",
                vec![response_path.to_string_lossy().into_owned()],
                5_000,
                1_048_576,
            ),
            &context(b"fn greet() {}\n"),
        )
        .expect("plugin success");

        assert_eq!(parsed.language.as_deref(), Some(LANGUAGE));
        assert_eq!(parsed.nodes.len(), 1);
        assert_eq!(parsed.nodes[0].id, NodeId::UNSET);
        assert_eq!(parsed.nodes[0].name, "greet");
        assert_eq!(parsed.nodes[0].file_hash, HASH);
    }

    #[test]
    fn external_lang_parser_dispatches_plugin_mode() {
        let dir = tempfile::tempdir().expect("tempdir");
        let response_path = dir.path().join("response.json");
        fs::write(&response_path, response(PATH, HASH)).expect("write response");

        let parser = crate::ExternalLangParser::new(&config(
            "cat",
            vec![response_path.to_string_lossy().into_owned()],
            5_000,
            1_048_576,
        ))
        .expect("plugin-mode external parser");
        assert_eq!(parser.language_name(), LANGUAGE);
        assert!(parser.supports(PATH));

        let (parsed, tree) = parser.parse(&context(b"fn greet() {}\n"));
        assert!(tree.is_none());
        assert_eq!(parsed.nodes[0].name, "greet");
    }

    #[test]
    fn external_lang_parser_surfaces_plugin_errors() {
        let parser = crate::ExternalLangParser::new(&config(
            "printf",
            vec!["not json".to_owned()],
            5_000,
            1_048_576,
        ))
        .expect("plugin-mode external parser");
        let error = parser
            .parse_checked(&context(b"fn greet() {}\n"))
            .expect_err("wrapper must forward plugin errors");
        assert!(error.contains("invalid JSON"), "unexpected error: {error}");
    }

    #[test]
    fn registry_surfaces_plugin_failures_as_parse_errors() {
        let mut registry = crate::ParserRegistry::with_defaults();
        registry.register(Box::new(
            PluginExternalParser::new(&config(
                "printf",
                vec!["not json".to_owned()],
                5_000,
                1_048_576,
            ))
            .expect("plugin parser"),
        ));

        let error = registry
            .parse_checked(PATH, HASH, b"fn greet() {}\n", None)
            .expect_err("malformed plugin output must be a parse error");
        assert!(error.contains("invalid JSON"), "unexpected error: {error}");
        assert!(
            registry
                .parse(PATH, HASH, b"fn greet() {}\n", None)
                .is_none(),
            "legacy parse wrapper must stay best-effort"
        );
    }

    #[test]
    fn plugin_timeout_is_a_parse_error() {
        let error = run(
            &config("sleep", vec!["5".to_owned()], 100, 1_048_576),
            &context(b"fn greet() {}\n"),
        )
        .expect_err("timeout must fail");
        assert!(error.contains("timed out"), "unexpected error: {error}");
    }

    #[test]
    fn plugin_nonzero_exit_is_a_parse_error() {
        let error = run(
            &config(
                "sh",
                vec!["-c".to_owned(), "exit 7".to_owned()],
                5_000,
                1_048_576,
            ),
            &context(b"fn greet() {}\n"),
        )
        .expect_err("nonzero exit must fail");
        assert!(error.contains("exited with"), "unexpected error: {error}");
    }

    #[test]
    fn plugin_malformed_json_is_a_parse_error() {
        let error = run(
            &config("printf", vec!["not json".to_owned()], 5_000, 1_048_576),
            &context(b"fn greet() {}\n"),
        )
        .expect_err("malformed JSON must fail");
        assert!(error.contains("invalid JSON"), "unexpected error: {error}");
    }

    #[test]
    fn plugin_oversized_response_is_a_parse_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let response_path = dir.path().join("response.json");
        fs::write(&response_path, response(PATH, HASH)).expect("write response");

        let error = run(
            &config(
                "cat",
                vec![response_path.to_string_lossy().into_owned()],
                5_000,
                64,
            ),
            &context(b"fn greet() {}\n"),
        )
        .expect_err("oversized response must fail");
        assert!(
            error.contains("response exceeded"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn plugin_hash_mismatch_is_a_parse_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let response_path = dir.path().join("response.json");
        fs::write(&response_path, response(PATH, "other-hash")).expect("write response");

        let error = run(
            &config(
                "cat",
                vec![response_path.to_string_lossy().into_owned()],
                5_000,
                1_048_576,
            ),
            &context(b"fn greet() {}\n"),
        )
        .expect_err("hash mismatch must fail");
        assert!(error.contains("hash mismatch"), "unexpected error: {error}");
    }

    #[test]
    fn plugin_parse_fallback_never_panics() {
        let parser = PluginExternalParser::new(&config(
            "printf",
            vec!["not json".to_owned()],
            5_000,
            1_048_576,
        ))
        .expect("plugin parser");
        let (parsed, tree) = parser.parse(&context(b"fn greet() {}\n"));
        assert!(tree.is_none());
        assert_eq!(parsed.nodes.len(), 1);
        assert_eq!(parsed.nodes[0].kind, atlas_core::NodeKind::File);
    }
}
