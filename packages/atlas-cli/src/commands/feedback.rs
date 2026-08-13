//! ICM-C2 — `atlas feedback` commands.
//!
//! All behavior lives in the shared feedback service layer in `atlas-session`;
//! this module only parses CLI flags, builds the shared input/filter types,
//! and renders output. `atlas analyze` and `atlas refactor remove-dead`
//! consume the same records through `FeedbackAdjuster` (ICM-C3).

use std::path::Path;

use anyhow::{Context, Result};

use atlas_session::{FeedbackSearchFilter, NewFeedback, SessionStore};

use crate::cli::{Cli, Command, FeedbackCommand};

use super::{print_json, resolve_repo};

pub fn run_feedback(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;

    let sub = match &cli.command {
        Command::Feedback { subcommand } => subcommand,
        _ => unreachable!(),
    };

    let mut store = SessionStore::open_in_repo(Path::new(&repo))
        .with_context(|| format!("cannot open session store in {repo}"))?;

    match sub {
        FeedbackCommand::Record {
            predicted,
            actual,
            correction,
            tool,
            analysis_kind,
            symbol,
            file,
            source_id,
        } => {
            let input = NewFeedback {
                repo_root: repo.clone(),
                session_id: None,
                tool_name: tool.clone().unwrap_or_else(|| "cli".to_owned()),
                analysis_kind: analysis_kind.clone().unwrap_or_default(),
                predicted: predicted.clone(),
                actual: actual.clone(),
                correction: correction.clone().unwrap_or_default(),
                related_symbol: symbol.clone(),
                related_file: file.clone(),
                source_id: source_id.clone(),
                metadata: serde_json::json!({}),
            };
            input.validate()?;
            let record = store.store_feedback(&input)?;

            if cli.json {
                print_json(
                    "feedback.record",
                    serde_json::json!({
                        "record": record,
                        "summary": {
                            "record_id": record.id,
                            "analysis_kind": record.analysis_kind,
                            "is_false_positive_evidence": record.is_false_positive_evidence(),
                        },
                    }),
                )?;
            } else {
                println!("Feedback recorded: {}", record.id);
                println!("Analysis kind : {}", display_or_dash(&record.analysis_kind));
                println!("Predicted     : {}", record.predicted);
                println!("Actual        : {}", record.actual);
                if !record.correction.is_empty() {
                    println!("Correction    : {}", record.correction);
                }
                println!(
                    "Evidence      : {}",
                    if record.is_false_positive_evidence() {
                        "false-positive (may lower matching confidence)"
                    } else {
                        "neutral (no confidence adjustment)"
                    }
                );
            }
        }

        FeedbackCommand::Search {
            query,
            tool,
            analysis_kind,
            symbol,
            file,
            limit,
        } => {
            let filter = FeedbackSearchFilter {
                tool_name: tool.clone(),
                analysis_kind: analysis_kind.clone(),
                related_symbol: symbol.clone(),
                related_file: file.clone(),
            };
            let hits = store.search_feedback(&repo, query, &filter, *limit)?;

            if cli.json {
                print_json(
                    "feedback.search",
                    serde_json::json!({
                        "query": query,
                        "count": hits.len(),
                        "results": hits,
                    }),
                )?;
            } else if hits.is_empty() {
                println!("No feedback records found for {query:?}");
            } else {
                println!("Found {} feedback records for {query:?}:", hits.len());
                for (index, hit) in hits.iter().enumerate() {
                    let record = &hit.feedback;
                    println!(
                        "{}. {} [{}] score={:.2}",
                        index + 1,
                        record.predicted,
                        display_or_dash(&record.analysis_kind),
                        hit.relevance_score
                    );
                    println!("   actual: {}", record.actual);
                    if !record.correction.is_empty() {
                        println!("   fix   : {}", record.correction);
                    }
                    if record.related_symbol.is_some() || record.related_file.is_some() {
                        println!(
                            "   ref   : {} {}",
                            record.related_symbol.as_deref().unwrap_or("-"),
                            record.related_file.as_deref().unwrap_or("-")
                        );
                    }
                    println!("   id    : {}", record.id);
                }
            }
        }

        FeedbackCommand::Stats => {
            let stats = store.feedback_stats(&repo)?;

            if cli.json {
                print_json("feedback.stats", serde_json::to_value(&stats)?)?;
            } else {
                println!("Feedback statistics:");
                println!("  Total records       : {}", stats.total_count);
                println!("  With correction     : {}", stats.correction_count);
                println!("  False-positive evid.: {}", stats.false_positive_count);
                println!("  By analysis kind    :");
                for (kind, count) in &stats.by_analysis_kind {
                    println!("    {kind:<20} {count}");
                }
                println!("  By tool             :");
                for (tool, count) in &stats.by_tool {
                    println!("    {tool:<20} {count}");
                }
            }
        }
    }

    Ok(())
}

fn display_or_dash(value: &str) -> &str {
    if value.is_empty() { "-" } else { value }
}
