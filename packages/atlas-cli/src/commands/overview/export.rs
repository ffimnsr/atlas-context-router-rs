use anyhow::{Context, Result};
use atlas_session::{OverviewExportRequest, OverviewExportScope};

use crate::cli::{Cli, Command, OverviewCommand};

use super::{open_graph_store, open_session_store, print_json};

pub(super) fn run_export(cli: &Cli, repo: &str, db_path: &str) -> Result<()> {
    let (
        scope,
        subjects,
        changed,
        since,
        limit,
        include_code_spans,
        include_callers,
        include_callees,
        include_tests,
        max_tokens,
    ) = match &cli.command {
        Command::Overview {
            subcommand:
                OverviewCommand::Export {
                    scope,
                    subjects,
                    changed,
                    since,
                    limit,
                    include_code_spans,
                    include_callers,
                    include_callees,
                    include_tests,
                    max_tokens,
                },
        } => (
            scope.clone(),
            subjects.clone(),
            *changed,
            since.clone(),
            *limit,
            *include_code_spans,
            *include_callers,
            *include_callees,
            *include_tests,
            *max_tokens,
        ),
        _ => unreachable!(),
    };

    let scope: OverviewExportScope = scope
        .parse()
        .map_err(anyhow::Error::from)
        .context("invalid --scope")?;
    let store = open_graph_store(db_path)?;
    let mut session = open_session_store(db_path)?;
    let request = OverviewExportRequest {
        scope,
        subjects,
        changed_only: changed,
        since,
        limit,
        include_code_spans,
        include_callers,
        include_callees,
        include_tests,
        max_tokens,
        json: cli.json,
    };
    let result = atlas_review::overview::export_overview_packets(
        repo,
        db_path,
        &store,
        Some(&mut session),
        &request,
    )?;

    if cli.json {
        print_json("overview.export", serde_json::to_value(&result)?)?;
        return Ok(());
    }

    println!(
        "Overview export: {} packet(s), {} subject(s), {} omitted",
        result.packets.len(),
        result.subject_count,
        result.omitted_subject_count
    );
    println!(
        "Graph   : {} ({})",
        result.graph.graph_state, result.freshness.state
    );
    println!("Commit  : {}", result.commit_sha);
    if let Some(reason) = &result.freshness.stale_reason {
        println!("Warning : {reason}");
    }
    for packet in &result.packets {
        println!(
            "  [{}] {} · {} nodes · {} edges · packet {}",
            packet.subject.kind,
            subject_label(packet),
            packet.evidence.node_count,
            packet.evidence.edge_count,
            packet.packet_id
        );
        if !packet.safe_to_answer {
            println!(
                "      stale: {}",
                packet
                    .freshness
                    .stale_reason
                    .as_deref()
                    .unwrap_or("unknown")
            );
        }
    }
    if result.budget.budget_hit {
        println!(
            "Budget hit: packet/byte cap reached ({} omitted)",
            result.budget.omitted_bytes
        );
    }
    Ok(())
}

fn subject_label(packet: &atlas_session::OverviewAnalysisPacket) -> String {
    packet
        .subject
        .qualified_name
        .clone()
        .or_else(|| packet.subject.canonical_file_path.clone())
        .or_else(|| packet.subject.package_name.clone())
        .or_else(|| packet.subject.module_path.clone())
        .unwrap_or_else(|| packet.subject.kind.as_str().to_owned())
}
