use anyhow::{Context, Result};
use atlas_adapters::{AdapterHooks, CliAdapter};
use atlas_core::GraphToolRequirement;
use atlas_docs::{
    DocsExportFormat, DocsExportScope, DocsView, ExportRequest, export_diagram, generate_docs,
    load_docs_context_with_timestamp,
};
use atlas_store_sqlite::Store;
use camino::Utf8PathBuf;

use crate::cli::{Cli, Command, DocsCommand};

use super::{
    check_graph_readiness, db_path, derive_graph_readiness, derive_graph_readiness_open_failed,
    readiness_overrides, resolve_repo,
};

/// Maximum diagram size embedded by `atlas docs generate --include-diagrams`.
const EMBEDDED_DIAGRAM_MAX_NODES: usize = 200;
const EMBEDDED_DIAGRAM_MAX_EDGES: usize = 400;

pub fn run_docs(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let command_label = match &cli.command {
        Command::Docs { subcommand, .. } => match subcommand {
            DocsCommand::Generate { .. } => "docs:generate",
            DocsCommand::Export { .. } => "docs:export",
        },
        _ => "docs",
    };
    let mut adapter = CliAdapter::open(&repo);
    if let Some(ref mut active) = adapter {
        active.before_command(command_label);
    }

    let result = (|| -> Result<()> {
        let db_path = db_path(cli, &repo);
        let (allow_stale, allow_partial) = match &cli.command {
            Command::Docs {
                allow_stale,
                allow_partial,
                ..
            } => (*allow_stale, *allow_partial),
            _ => (false, false),
        };

        let store = match Store::open(&db_path) {
            Ok(store) => store,
            Err(error) => {
                let readiness =
                    derive_graph_readiness_open_failed(&repo, &db_path, &error.to_string());
                check_graph_readiness(
                    &readiness,
                    GraphToolRequirement::Analysis,
                    readiness_overrides(allow_stale, allow_partial),
                    "docs",
                    cli,
                )?;
                return Err(error).with_context(|| format!("cannot open database at {db_path}"));
            }
        };

        let readiness = derive_graph_readiness(&store, &repo, &db_path);
        if let Some(warning) = check_graph_readiness(
            &readiness,
            GraphToolRequirement::Analysis,
            readiness_overrides(allow_stale, allow_partial),
            "docs",
            cli,
        )? {
            eprintln!("Warning: {warning}");
        }
        // Docs are written artifacts: never publish content derived from an
        // un-indexed working tree unless the caller opts in explicitly.
        if readiness.stale_index && !allow_stale {
            anyhow::bail!(
                "graph is stale: {} pending change(s) are not indexed; run `atlas update` first or pass --allow-stale",
                readiness.pending_graph_changes.len()
            );
        }

        let subcommand = match &cli.command {
            Command::Docs { subcommand, .. } => subcommand,
            _ => unreachable!(),
        };

        match subcommand {
            DocsCommand::Generate {
                output,
                timestamp,
                include_diagrams,
            } => {
                let output_dir = match output {
                    Some(path) => Utf8PathBuf::from(path),
                    // Default into the ignored `.atlas` work directory: generated
                    // Markdown is indexable, so writing into the repo tree would
                    // make the graph stale on the very next run.
                    None => Utf8PathBuf::from(&repo).join(".atlas/docs"),
                };
                let data = load_docs_context_with_timestamp(&store, &repo, timestamp.clone())?;
                let view = DocsView::new(&data);
                let mut docs = generate_docs(&view);
                if *include_diagrams {
                    embed_diagrams(&view, &mut docs)?;
                }
                std::fs::create_dir_all(output_dir.as_std_path())
                    .with_context(|| format!("cannot create output directory {output_dir}"))?;
                for (name, content) in &docs {
                    std::fs::write(output_dir.join(name), content)
                        .with_context(|| format!("cannot write {output_dir}/{name}"))?;
                }
                println!(
                    "Generated {} Markdown file{} in {}",
                    docs.len(),
                    if docs.len() == 1 { "" } else { "s" },
                    output_dir
                );
            }
            DocsCommand::Export {
                format,
                scope,
                name,
                max_nodes,
                max_edges,
                output,
            } => {
                let data = load_docs_context_with_timestamp(&store, &repo, None)?;
                let view = DocsView::new(&data);
                let result = export_diagram(
                    &view,
                    &ExportRequest {
                        format: (*format).into(),
                        scope: (*scope).into(),
                        name: name.clone(),
                        max_nodes: *max_nodes,
                        max_edges: *max_edges,
                    },
                )?;
                match output {
                    Some(path) => {
                        std::fs::write(path, &result.content)
                            .with_context(|| format!("cannot write diagram to {path}"))?;
                    }
                    None => {
                        print!("{}", result.content);
                    }
                }
            }
        }

        Ok(())
    })();

    if let Some(ref mut active) = adapter {
        active.after_command(command_label, result.is_ok());
    }
    result
}

/// Append fenced Mermaid diagrams to `index.md` (repo graph) and `modules.md`
/// (one diagram per inferred module) when `--include-diagrams` is passed.
fn embed_diagrams(
    view: &DocsView<'_>,
    docs: &mut std::collections::BTreeMap<String, String>,
) -> Result<()> {
    let request = ExportRequest {
        format: DocsExportFormat::Mermaid,
        scope: DocsExportScope::Repo,
        name: None,
        max_nodes: EMBEDDED_DIAGRAM_MAX_NODES,
        max_edges: EMBEDDED_DIAGRAM_MAX_EDGES,
    };
    if let Some(index) = docs.get_mut("index.md") {
        index.push_str("\n## Repository dependency diagram\n\n");
        index.push_str(&fence_mermaid(export_diagram(view, &request)?.content));
        index.push('\n');
    }
    if let Some(modules) = docs.get_mut("modules.md") {
        for module in &view.data().modules {
            let module_request = ExportRequest {
                format: DocsExportFormat::Mermaid,
                scope: DocsExportScope::Module,
                name: Some(module.display_name.clone()),
                max_nodes: EMBEDDED_DIAGRAM_MAX_NODES,
                max_edges: EMBEDDED_DIAGRAM_MAX_EDGES,
            };
            let result = export_diagram(view, &module_request)?;
            if result.node_count == 0 {
                continue;
            }
            modules.push_str(&format!("\n### {} diagram\n\n", module.display_name));
            modules.push_str(&fence_mermaid(result.content));
            modules.push('\n');
        }
    }
    Ok(())
}

fn fence_mermaid(diagram: String) -> String {
    format!("```mermaid\n{diagram}```\n")
}
