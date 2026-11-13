//! ICM-F2 — `atlas memoir` command implementation.

use std::path::Path;

use anyhow::{Context, Result};

use atlas_session::{
    MemoirInspectQuery, MemoryScope, NewMemoirConcept, NewMemoirGraph, NewMemoirRelation,
    SessionStore,
};

use crate::cli::{Cli, Command, MemoirCommand};

use super::{print_json, resolve_repo};

pub fn run_memoir(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let sub = match &cli.command {
        Command::Memoir { subcommand } => subcommand,
        _ => unreachable!(),
    };

    let mut store = SessionStore::open_in_repo(Path::new(&repo))
        .with_context(|| format!("cannot open session store in {repo}"))?;

    match sub {
        MemoirCommand::Create {
            name,
            description,
            scope,
        } => {
            let graph = store.create_memoir_graph(&NewMemoirGraph {
                repo_root: repo.clone(),
                scope: parse_scope(scope.as_deref())?,
                name: name.clone(),
                description: description.clone().unwrap_or_default(),
                metadata: serde_json::json!({}),
            })?;

            if cli.json {
                print_json("memoir.create", serde_json::json!({ "graph": graph }))?;
            } else {
                println!("Memoir graph created: {}", graph.name);
                println!("Id   : {}", graph.id);
                println!("Scope: {}", graph.scope);
                if !graph.description.is_empty() {
                    println!("About: {}", graph.description);
                }
            }
        }

        MemoirCommand::AddConcept {
            graph,
            name,
            description,
            kind,
            source_id,
            scope,
        } => {
            let concept = store.add_memoir_concept(&NewMemoirConcept {
                repo_root: repo.clone(),
                scope: parse_scope(scope.as_deref())?,
                graph: graph.clone(),
                name: name.clone(),
                description: description.clone().unwrap_or_default(),
                kind: kind.clone().unwrap_or_else(|| "concept".to_owned()),
                source_id: source_id.clone(),
                metadata: serde_json::json!({}),
            })?;

            if cli.json {
                print_json(
                    "memoir.add-concept",
                    serde_json::json!({ "concept": concept }),
                )?;
            } else {
                println!("Memoir concept added: {}", concept.name);
                println!("Id   : {}", concept.id);
                println!("Kind : {}", concept.kind);
                println!("Graph: {}", graph);
            }
        }

        MemoirCommand::Link {
            graph,
            source,
            target,
            relation,
            relation_positional,
            confidence,
            source_id,
            scope,
            allow_missing_concepts,
        } => {
            let relation = relation
                .as_deref()
                .or(relation_positional.as_deref())
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "relation required; pass a positional RELATION or --relation <TYPE>"
                    )
                })?;
            let relation = store.link_memoir_concepts(&NewMemoirRelation {
                repo_root: repo.clone(),
                scope: parse_scope(scope.as_deref())?,
                graph: graph.clone(),
                source: source.clone(),
                target: target.clone(),
                relation: relation.to_owned(),
                confidence: *confidence,
                source_id: source_id.clone(),
                metadata: serde_json::json!({}),
                create_missing: *allow_missing_concepts,
            })?;

            if cli.json {
                print_json("memoir.link", serde_json::json!({ "relation": relation }))?;
            } else {
                println!("Memoir relation linked: {}", relation.relation_type);
                println!("Id        : {}", relation.id);
                println!("Source id : {}", relation.source_concept_id);
                println!("Target id : {}", relation.target_concept_id);
                println!("Confidence: {:.2}", relation.confidence);
                if let Some(evidence) = &relation.source_id {
                    println!("Evidence  : {evidence}");
                }
            }
        }

        MemoirCommand::Inspect {
            targets,
            graph,
            depth,
            relation,
            scope,
        } => {
            let (graph, concept) = match (targets.as_slice(), graph.as_deref()) {
                ([graph_name, concept], None) => (Some(graph_name.clone()), concept.clone()),
                ([concept], graph_name) => (graph_name.map(str::to_owned), concept.clone()),
                _ => {
                    anyhow::bail!(
                        "inspect requires <CONCEPT> with optional --graph, or <GRAPH> <CONCEPT>"
                    )
                }
            };
            let inspection = store.inspect_memoir_concept(&MemoirInspectQuery {
                repo_root: repo.clone(),
                graph: graph.clone(),
                concept: concept.clone(),
                scope: parse_optional_scope(scope.as_deref())?,
                depth: *depth,
                relation: relation.clone(),
            })?;

            if cli.json {
                print_json(
                    "memoir.inspect",
                    serde_json::json!({ "inspection": inspection }),
                )?;
            } else {
                println!(
                    "Memoir inspect: {} in graph {} (depth {})",
                    inspection.root.name, inspection.graph.name, inspection.depth
                );
                if inspection.edges.is_empty() {
                    println!("No relations found.");
                }
                for edge in &inspection.edges {
                    println!(
                        "  [depth {} · {}] {} --{}--> {} (confidence {:.2})",
                        edge.depth,
                        edge.direction.as_str(),
                        edge.from_concept,
                        edge.relation_type,
                        edge.to_concept,
                        edge.confidence
                    );
                    println!("      relation id: {}", edge.relation_id);
                    if let Some(evidence) = &edge.source_id {
                        println!("      evidence   : {evidence}");
                    }
                }
                if inspection.truncated {
                    println!("  (truncated: inspect limit reached)");
                }
            }
        }
    }

    Ok(())
}

fn parse_scope(value: Option<&str>) -> Result<MemoryScope> {
    parse_optional_scope(value).map(|scope| scope.unwrap_or_default())
}

fn parse_optional_scope(value: Option<&str>) -> Result<Option<MemoryScope>> {
    value
        .map(|raw| raw.parse().map_err(anyhow::Error::from))
        .transpose()
}
