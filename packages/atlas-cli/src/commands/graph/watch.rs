use crate::cli::{Cli, Command};
use anyhow::{Context, Result};
use atlas_repo::find_repo_root;
use camino::Utf8Path;

use super::super::{db_path, resolve_repo};

pub fn run_watch(cli: &Cli) -> Result<()> {
    use atlas_engine::{WatchRunner, config};
    use std::time::Duration;

    let repo = resolve_repo(cli)?;
    let repo_root_path =
        find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
    let db_path = db_path(cli, &repo);

    let engine_config = config::Config::load(&atlas_engine::paths::atlas_dir(&repo))?;

    let (debounce_ms, watch_json) = match &cli.command {
        Command::Watch { debounce_ms, json } => (*debounce_ms, *json),
        _ => (200, false),
    };
    let json_output = cli.json || watch_json;
    let debounce = Duration::from_millis(debounce_ms);

    let mut runner = WatchRunner::new(
        repo_root_path.as_path(),
        db_path.clone(),
        debounce,
        engine_config.parse_batch_size(),
    )
    .context("cannot start watch runner")?;

    if !json_output {
        println!(
            "Watching '{}' (debounce {}ms) — press Ctrl+C to stop",
            repo_root_path, debounce_ms
        );
    }

    runner.run(|result| {
        if json_output {
            let obj = serde_json::json!({
                "schema_version": "atlas_cli.v1",
                "command": "watch",
                "data": {
                    "files_updated": result.files_updated,
                    "observed_events": result.observed_events,
                    "coalesced_events": result.coalesced_events,
                    "dropped_events": result.dropped_events,
                    "recovery_mode": result.recovery_mode,
                    "nodes_updated": result.nodes_updated,
                    "errors": result.errors,
                    "elapsed_ms": result.elapsed_ms,
                    "error_messages": result.error_messages,
                }
            });
            println!("{}", serde_json::to_string(&obj).unwrap_or_default());
        } else if result.errors > 0 {
            if result.dropped_events > 0 {
                eprintln!(
                    "watch: backlog overflowed; recovered via {} after dropping {} raw event(s)",
                    result.recovery_mode, result.dropped_events,
                );
            }
            eprintln!(
                "watch: {} file(s) — {} event(s) observed — {} coalesced — {} node(s) updated — {} error(s) [{} ms]",
                result.files_updated,
                result.observed_events,
                result.coalesced_events,
                result.nodes_updated,
                result.errors,
                result.elapsed_ms,
            );
            for msg in &result.error_messages {
                eprintln!("  {msg}");
            }
        } else {
            if result.dropped_events > 0 {
                println!(
                    "watch: backlog overflowed; recovered via {} after dropping {} raw event(s)",
                    result.recovery_mode, result.dropped_events,
                );
            }
            println!(
                "watch: {} file(s) — {} event(s) observed — {} coalesced — {} node(s) updated [{} ms]",
                result.files_updated,
                result.observed_events,
                result.coalesced_events,
                result.nodes_updated,
                result.elapsed_ms,
            );
        }
    })
}
