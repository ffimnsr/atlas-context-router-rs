//! `atlas uri` command dispatch.

use anyhow::Result;
use serde_json::json;

use crate::cli::{Cli, Command, UriCommand};

use super::{db_path, print_json, resolve_repo};

pub fn run_uri(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);
    let (sub, shorthand_uri, shorthand_level) = match &cli.command {
        Command::Uri {
            uri,
            level,
            subcommand,
        } => (subcommand.as_ref(), uri.as_deref(), level.as_deref()),
        _ => unreachable!(),
    };

    if let Some(uri) = shorthand_uri {
        let (mime_type, text) = atlas_mcp::atlas_uri_read(uri, shorthand_level, &repo, &db_path)?;
        return emit(cli, "uri.read", uri, &mime_type, text);
    }

    let sub = sub.ok_or_else(|| anyhow::anyhow!("uri command requires a uri or subcommand"))?;

    match sub {
        UriCommand::Read { uri, level } => {
            let (mime_type, text) =
                atlas_mcp::atlas_uri_read(uri, level.as_deref(), &repo, &db_path)?;
            emit(cli, "uri.read", uri, &mime_type, text)
        }
        UriCommand::List { uri } => {
            let (mime_type, text) = atlas_mcp::atlas_uri_list(uri, &repo, &db_path)?;
            emit(cli, "uri.list", uri, &mime_type, text)
        }
        UriCommand::Grep {
            uri,
            pattern,
            regex,
        } => {
            let (mime_type, text) =
                atlas_mcp::atlas_uri_grep(uri, pattern, *regex, &repo, &db_path)?;
            emit(cli, "uri.grep", uri, &mime_type, text)
        }
        UriCommand::Search { uri, query } => {
            let (mime_type, text) = atlas_mcp::atlas_uri_search(uri, query, &repo, &db_path)?;
            emit(cli, "uri.search", uri, &mime_type, text)
        }
    }
}

fn emit(cli: &Cli, command: &str, uri: &str, mime_type: &str, text: String) -> Result<()> {
    if cli.json {
        print_json(
            command,
            json!({
                "uri": uri,
                "mimeType": mime_type,
                "text": text,
            }),
        )
    } else {
        println!("{text}");
        Ok(())
    }
}
