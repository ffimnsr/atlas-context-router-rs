use std::io::{IsTerminal, Read};

use anyhow::{Context, Result};
use camino::Utf8Path;
use serde_json::{Value, json};

use atlas_adapters::redact_payload;
use atlas_agent_events::policy::MAX_HOOK_STDIN_BYTES;
use atlas_repo::find_repo_root;

use crate::cli::Cli;
use crate::cli_paths::canonicalize_cli_path;

use super::super::resolve_repo;

pub(crate) fn resolve_hook_repo(cli: &Cli) -> Result<String> {
    if cli.repo.is_some() {
        return canonicalize_cli_path(&resolve_repo(cli)?);
    }

    if let Ok(script_path) = std::env::var("ATLAS_HOOK_SCRIPT_PATH") {
        let script_path = script_path.trim();
        if !script_path.is_empty() {
            let script_path = Utf8Path::new(script_path);
            let start = if script_path.is_file() {
                script_path.parent().unwrap_or(script_path)
            } else {
                script_path
            };
            if let Ok(root) = find_repo_root(start) {
                return canonicalize_cli_path(root.as_str());
            }
        }
    }

    let cwd = resolve_repo(cli)?;
    if let Ok(root) = find_repo_root(Utf8Path::new(&cwd)) {
        return canonicalize_cli_path(root.as_str());
    }

    canonicalize_cli_path(&cwd)
}

pub(crate) fn hook_frontend() -> String {
    std::env::var("ATLAS_HOOK_FRONTEND")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "hook".to_owned())
}

pub(crate) fn read_hook_payload() -> Result<Value> {
    let stdin = std::io::stdin();
    let stdin_is_terminal = stdin.is_terminal();
    read_hook_payload_from(stdin, stdin_is_terminal)
}

pub(crate) fn read_hook_payload_from<R: Read>(reader: R, stdin_is_terminal: bool) -> Result<Value> {
    if stdin_is_terminal {
        return Ok(Value::Null);
    }

    let mut raw = String::new();
    reader
        .take(MAX_HOOK_STDIN_BYTES)
        .read_to_string(&mut raw)
        .context("cannot read hook payload from stdin")?;

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Value::Null);
    }

    let parsed =
        serde_json::from_str::<Value>(trimmed).unwrap_or_else(|_| json!({ "raw": trimmed }));
    Ok(redact_payload(parsed))
}
