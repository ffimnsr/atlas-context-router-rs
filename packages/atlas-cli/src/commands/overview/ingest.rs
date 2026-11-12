use std::io::Read;

use anyhow::{Context, Result};
use atlas_adapters::{RedactionRules, load_redaction_rules_file};
use atlas_contentstore::ContentStore;
use atlas_session::OverviewAnalysisInput;

use crate::cli::{Cli, Command, OverviewCommand};

use super::{open_graph_store, open_session_store, print_json};

pub(super) fn run_ingest(cli: &Cli, repo: &str, db_path: &str) -> Result<()> {
    let input = match &cli.command {
        Command::Overview {
            subcommand: OverviewCommand::Ingest { input },
        } => input.clone(),
        _ => unreachable!(),
    };

    let raw = if input == "-" {
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .context("cannot read overview analysis from stdin")?;
        buffer
    } else {
        std::fs::read_to_string(&input)
            .with_context(|| format!("cannot read overview analysis file {input}"))?
    };
    let analysis: OverviewAnalysisInput =
        serde_json::from_str(&raw).context("invalid overview analysis JSON")?;

    let store = open_graph_store(db_path)?;
    let mut session = open_session_store(db_path)?;
    let content_db = atlas_adapters::derive_content_db_path(db_path);
    let mut content = ContentStore::open(&content_db)
        .with_context(|| format!("cannot open content store at {content_db}"))?;
    let rules = load_rules(repo)?;
    let result = atlas_review::overview::ingest_overview_analysis(
        repo,
        &store,
        &mut session,
        &mut content,
        &rules,
        &analysis,
    )?;

    if cli.json {
        print_json("overview.ingest", serde_json::to_value(&result)?)?;
        return Ok(());
    }

    println!("Overview record {} ({})", result.record.id, result.status);
    println!(
        "Subject : {} · {}",
        result.record.subject_kind, result.record.subject_id
    );
    println!(
        "Checks  : resolved={} packet_validated={} redacted={} body_routed={}",
        result.subject_resolved, result.packet_validated, result.redacted, result.body_routed
    );
    if let Some(source_id) = &result.record.source_id {
        println!("Evidence: source_id={source_id}");
    }
    Ok(())
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
