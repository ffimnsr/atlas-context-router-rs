use anyhow::{Context, Result};
use atlas_store_sqlite::Store;

use crate::cli::{Cli, Command, CommunitiesCommand, FlowsCommand};

use super::{db_path, print_json, resolve_repo};

pub fn run_flows(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);
    let store =
        Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;

    let sub = match &cli.command {
        Command::Flows { subcommand } => subcommand,
        _ => unreachable!(),
    };

    match sub {
        FlowsCommand::List => {
            let flows = store.list_flows()?;
            if cli.json {
                print_json("flows.list", serde_json::to_value(&flows)?)?;
            } else if flows.is_empty() {
                println!("No flows.");
            } else {
                for f in &flows {
                    let kind = f.kind.as_deref().unwrap_or("-");
                    let desc = f.description.as_deref().unwrap_or("");
                    println!("[{}] {} ({kind}) {desc}", f.id, f.name);
                }
            }
        }
        FlowsCommand::Create {
            name,
            kind,
            description,
        } => {
            let id = store.create_flow(name, kind.as_deref(), description.as_deref())?;
            if cli.json {
                print_json(
                    "flows.create",
                    serde_json::json!({ "id": id, "name": name }),
                )?;
            } else {
                println!("Created flow '{name}' (id={id})");
            }
        }
        FlowsCommand::Delete { name } => {
            let flow = store
                .get_flow_by_name(name)?
                .with_context(|| format!("flow '{name}' not found"))?;
            store.delete_flow(flow.id)?;
            if cli.json {
                print_json("flows.delete", serde_json::json!({ "name": name }))?;
            } else {
                println!("Deleted flow '{name}'");
            }
        }
        FlowsCommand::Members { name } => {
            let flow = store
                .get_flow_by_name(name)?
                .with_context(|| format!("flow '{name}' not found"))?;
            let members = store.get_flow_members(flow.id)?;
            if cli.json {
                print_json("flows.members", serde_json::to_value(&members)?)?;
            } else if members.is_empty() {
                println!("Flow '{name}' has no members.");
            } else {
                for m in &members {
                    let pos = m.position.map(|p| p.to_string()).unwrap_or("-".into());
                    let role = m.role.as_deref().unwrap_or("-");
                    println!("  [{pos}] {} (role={role})", m.node_qualified_name);
                }
            }
        }
        FlowsCommand::AddMember {
            flow,
            node_qn,
            position,
            role,
        } => {
            let f = store
                .get_flow_by_name(flow)?
                .with_context(|| format!("flow '{flow}' not found"))?;
            store.add_flow_member(f.id, node_qn, *position, role.as_deref())?;
            if cli.json {
                print_json(
                    "flows.add-member",
                    serde_json::json!({ "flow": flow, "node_qn": node_qn }),
                )?;
            } else {
                println!("Added '{node_qn}' to flow '{flow}'");
            }
        }
        FlowsCommand::RemoveMember { flow, node_qn } => {
            let f = store
                .get_flow_by_name(flow)?
                .with_context(|| format!("flow '{flow}' not found"))?;
            store.remove_flow_member(f.id, node_qn)?;
            if cli.json {
                print_json(
                    "flows.remove-member",
                    serde_json::json!({ "flow": flow, "node_qn": node_qn }),
                )?;
            } else {
                println!("Removed '{node_qn}' from flow '{flow}'");
            }
        }
        FlowsCommand::ForNode { node_qn } => {
            let flows = store.flows_for_node(node_qn)?;
            if cli.json {
                print_json("flows.for-node", serde_json::to_value(&flows)?)?;
            } else if flows.is_empty() {
                println!("No flows contain node '{node_qn}'.");
            } else {
                for f in &flows {
                    println!("[{}] {}", f.id, f.name);
                }
            }
        }
    }

    Ok(())
}

pub fn run_communities(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);
    let store =
        Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;

    let sub = match &cli.command {
        Command::Communities { subcommand } => subcommand,
        _ => unreachable!(),
    };

    match sub {
        CommunitiesCommand::List => {
            let comms = store.list_communities()?;
            if cli.json {
                print_json("communities.list", serde_json::to_value(&comms)?)?;
            } else if comms.is_empty() {
                println!("No communities.");
            } else {
                for c in &comms {
                    let alg = c.algorithm.as_deref().unwrap_or("-");
                    let parent = c
                        .parent_community_id
                        .map(|p| p.to_string())
                        .unwrap_or("-".into());
                    println!(
                        "[{}] {} (algorithm={alg}, level={}, parent={parent})",
                        c.id,
                        c.name,
                        c.level.unwrap_or(0)
                    );
                }
            }
        }
        CommunitiesCommand::Create {
            name,
            algorithm,
            level,
            parent,
        } => {
            let id = store.create_community(name, algorithm.as_deref(), *level, *parent)?;
            if cli.json {
                print_json(
                    "communities.create",
                    serde_json::json!({ "id": id, "name": name }),
                )?;
            } else {
                println!("Created community '{name}' (id={id})");
            }
        }
        CommunitiesCommand::Delete { name } => {
            let comm = store
                .get_community_by_name(name)?
                .with_context(|| format!("community '{name}' not found"))?;
            store.delete_community(comm.id)?;
            if cli.json {
                print_json("communities.delete", serde_json::json!({ "name": name }))?;
            } else {
                println!("Deleted community '{name}'");
            }
        }
        CommunitiesCommand::Nodes { name } => {
            let comm = store
                .get_community_by_name(name)?
                .with_context(|| format!("community '{name}' not found"))?;
            let nodes = store.get_community_nodes(comm.id)?;
            if cli.json {
                print_json("communities.nodes", serde_json::to_value(&nodes)?)?;
            } else if nodes.is_empty() {
                println!("Community '{name}' has no members.");
            } else {
                for n in &nodes {
                    println!("  {}", n.node_qualified_name);
                }
            }
        }
        CommunitiesCommand::AddNode { community, node_qn } => {
            let comm = store
                .get_community_by_name(community)?
                .with_context(|| format!("community '{community}' not found"))?;
            store.add_community_node(comm.id, node_qn)?;
            if cli.json {
                print_json(
                    "communities.add-node",
                    serde_json::json!({ "community": community, "node_qn": node_qn }),
                )?;
            } else {
                println!("Added '{node_qn}' to community '{community}'");
            }
        }
        CommunitiesCommand::RemoveNode { community, node_qn } => {
            let comm = store
                .get_community_by_name(community)?
                .with_context(|| format!("community '{community}' not found"))?;
            store.remove_community_node(comm.id, node_qn)?;
            if cli.json {
                print_json(
                    "communities.remove-node",
                    serde_json::json!({ "community": community, "node_qn": node_qn }),
                )?;
            } else {
                println!("Removed '{node_qn}' from community '{community}'");
            }
        }
        CommunitiesCommand::ForNode { node_qn } => {
            let comms = store.communities_for_node(node_qn)?;
            if cli.json {
                print_json("communities.for-node", serde_json::to_value(&comms)?)?;
            } else if comms.is_empty() {
                println!("No communities contain node '{node_qn}'.");
            } else {
                for c in &comms {
                    println!("[{}] {}", c.id, c.name);
                }
            }
        }
    }

    Ok(())
}
