use std::io::Write;
use std::process::{Command as ProcessCommand, Stdio};

use anyhow::{Context, Result};
use atlas_adapters::{RedactionRules, load_redaction_rules_file};
use atlas_contentstore::ContentStore;
use atlas_session::{OverviewAnalysisInput, OverviewExportRequest, OverviewExportScope};

use crate::cli::{Cli, Command, OverviewCommand};

use super::{open_graph_store, open_session_store, print_json};

pub(super) fn run_external(cli: &Cli, repo: &str, db_path: &str) -> Result<()> {
    let (dry_run, apply, scope, since, limit) = match &cli.command {
        Command::Overview {
            subcommand:
                OverviewCommand::RunExternal {
                    dry_run,
                    apply,
                    scope,
                    since,
                    limit,
                },
        } => (*dry_run, *apply, scope.clone(), since.clone(), *limit),
        _ => unreachable!(),
    };
    if dry_run == apply {
        anyhow::bail!("pass exactly one of --dry-run or --apply");
    }

    let config =
        atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo)).unwrap_or_default();
    if !config.overview.enabled {
        anyhow::bail!("overview memory is disabled in config ([overview].enabled = false)");
    }
    let command = config
        .overview
        .external_analyzer_command
        .as_deref()
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no external analyzer configured; set [overview].external_analyzer_command"
            )
        })?
        .to_owned();

    let scope: OverviewExportScope = scope.parse().map_err(anyhow::Error::from)?;
    let max_subjects = config.overview.max_subjects_per_run;
    let request = OverviewExportRequest {
        scope,
        changed_only: scope == OverviewExportScope::Changed,
        since,
        limit: Some(limit.unwrap_or(max_subjects).min(max_subjects)),
        include_code_spans: true,
        include_callers: true,
        include_callees: true,
        include_tests: true,
        max_tokens: Some((config.overview.max_packet_bytes / 4).clamp(256, 8_192)),
        ..OverviewExportRequest::default()
    };

    let store = open_graph_store(db_path)?;
    let mut session = open_session_store(db_path)?;
    let export = atlas_review::overview::export_overview_packets(
        repo,
        db_path,
        &store,
        Some(&mut session),
        &request,
    )?;

    if dry_run {
        let invocations = export
            .packets
            .iter()
            .map(|packet| {
                let packet_json = serde_json::to_string(packet).unwrap_or_default();
                serde_json::json!({
                    "command": command,
                    "packet_id": packet.packet_id,
                    "subject_id": packet.subject_id,
                    "subject_kind": packet.subject.kind.as_str(),
                    "stdin_bytes": packet_json.len(),
                    "stdin": "analysis packet JSON",
                })
            })
            .collect::<Vec<_>>();
        if cli.json {
            print_json(
                "overview.run_external",
                serde_json::json!({
                    "dry_run": true,
                    "command": command,
                    "packet_count": invocations.len(),
                    "invocations": invocations,
                }),
            )?;
        } else {
            println!(
                "Dry run: {} invocation(s) of `{}`",
                invocations.len(),
                command
            );
            for invocation in invocations {
                println!(
                    "  {} · {} ({}, {} bytes on stdin)",
                    invocation["subject_kind"].as_str().unwrap_or(""),
                    invocation["packet_id"].as_str().unwrap_or(""),
                    invocation["subject_id"].as_str().unwrap_or(""),
                    invocation["stdin_bytes"].as_u64().unwrap_or(0)
                );
            }
        }
        return Ok(());
    }

    // Apply mode: execute configured command per packet, require JSON output,
    // validate through the same ingest path as CLI/MCP.
    let content_db = atlas_adapters::derive_content_db_path(db_path);
    let mut content = ContentStore::open(&content_db)
        .with_context(|| format!("cannot open content store at {content_db}"))?;
    let rules = load_rules(repo)?;
    let mut successes = 0usize;
    let mut failures = Vec::new();
    for packet in &export.packets {
        let packet_json = serde_json::to_string(packet)?;
        match run_one(&command, &packet_json) {
            Ok(output) => match serde_json::from_str::<OverviewAnalysisInput>(&output) {
                Ok(input) => match atlas_review::overview::ingest_overview_analysis(
                    repo,
                    &store,
                    &mut session,
                    &mut content,
                    &rules,
                    &input,
                ) {
                    Ok(result) => {
                        successes += 1;
                        if !cli.json {
                            println!(
                                "  ingested {} · {} · {}",
                                packet.subject_id, result.record.id, result.status
                            );
                        }
                    }
                    Err(error) => failures.push(serde_json::json!({
                        "packet_id": packet.packet_id,
                        "subject_id": packet.subject_id,
                        "error": error.to_string(),
                    })),
                },
                Err(error) => failures.push(serde_json::json!({
                    "packet_id": packet.packet_id,
                    "subject_id": packet.subject_id,
                    "error": format!("malformed external JSON: {error}"),
                })),
            },
            Err(error) => failures.push(serde_json::json!({
                "packet_id": packet.packet_id,
                "subject_id": packet.subject_id,
                "error": error.to_string(),
            })),
        }
    }

    if cli.json {
        print_json(
            "overview.run_external",
            serde_json::json!({
                "dry_run": false,
                "command": command,
                "packet_count": export.packets.len(),
                "ingested_count": successes,
                "failure_count": failures.len(),
                "failures": failures,
            }),
        )?;
    } else {
        println!(
            "External analysis: {} ingested, {} failed",
            successes,
            failures.len()
        );
        for failure in &failures {
            println!(
                "  {}: {}",
                failure["subject_id"].as_str().unwrap_or(""),
                failure["error"].as_str().unwrap_or("")
            );
        }
    }
    if successes == 0 && !failures.is_empty() {
        anyhow::bail!("external analyzer produced no valid overview records");
    }
    Ok(())
}

fn run_one(command: &str, packet_json: &str) -> Result<String> {
    let mut child = ProcessCommand::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("cannot spawn external analyzer: {command}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(packet_json.as_bytes())
            .context("cannot write packet to external analyzer stdin")?;
    }
    let output = child
        .wait_with_output()
        .context("external analyzer execution failed")?;
    if !output.status.success() {
        anyhow::bail!(
            "external analyzer exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout).context("external analyzer output is not UTF-8")
}

fn load_rules(repo: &str) -> Result<RedactionRules> {
    let config =
        atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo)).unwrap_or_default();
    let Some(file) = config.sanitization.redaction_rules_file else {
        return Ok(RedactionRules::default());
    };
    let path = std::path::Path::new(&file);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        atlas_engine::paths::atlas_dir(repo).join(file)
    };
    load_redaction_rules_file(&path)
        .with_context(|| format!("cannot load redaction rules {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::run_one;

    #[test]
    fn run_one_pipes_packet_to_command_and_returns_stdout() {
        let output = run_one("cat", "{\"packet\":true}").unwrap();
        assert_eq!(output, "{\"packet\":true}");
    }

    #[test]
    fn run_one_fails_on_nonzero_exit() {
        let error = run_one("exit 7", "").unwrap_err().to_string();
        assert!(error.contains("exited with"), "{error}");
    }
}
