use anyhow::{Context, Result};
use atlas_core::{GraphToolRequirement, kinds::normalize_kind_alias};
use atlas_repo::{find_repo_root, normalize_repo_file_path};
use atlas_store_sqlite::{NodeListFilter, Store};
use camino::Utf8Path;

use crate::cli::{Cli, Command};

use super::{
    check_graph_readiness, db_path, derive_graph_readiness, derive_graph_readiness_open_failed,
    print_json, readiness_overrides, resolve_repo,
};

pub fn run_symbols(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);

    let (kind, language, subpath, repo_id, limit, offset, allow_stale, allow_partial) =
        match &cli.command {
            Command::Symbols {
                kind,
                language,
                subpath,
                repo_id,
                limit,
                offset,
                allow_stale,
                allow_partial,
            } => (
                kind.clone(),
                language.clone(),
                subpath.clone(),
                repo_id.clone(),
                *limit,
                *offset,
                *allow_stale,
                *allow_partial,
            ),
            _ => unreachable!(),
        };

    let store = match Store::open(&db_path) {
        Ok(s) => s,
        Err(e) => {
            let readiness = derive_graph_readiness_open_failed(&repo, &db_path, &e.to_string());
            check_graph_readiness(
                &readiness,
                GraphToolRequirement::SymbolLookup,
                readiness_overrides(false, false),
                "symbols",
                cli,
            )?;
            return Err(e).with_context(|| format!("cannot open database at {db_path}"));
        }
    };

    let readiness = derive_graph_readiness(&store, &repo, &db_path);
    if let Some(warning) = check_graph_readiness(
        &readiness,
        GraphToolRequirement::SymbolLookup,
        readiness_overrides(allow_stale, allow_partial),
        "symbols",
        cli,
    )? {
        eprintln!("Warning: {warning}");
    }

    let kind = kind.map(|raw| normalize_kind_alias(&raw));
    // Best-effort subpath normalization: root-prefixed and absolute-under-root
    // forms collapse to canonical repo-relative prefixes; unmatched inputs stay
    // as typed so the prefix filter can still match literally.
    let subpath = subpath
        .map(|raw| {
            find_repo_root(Utf8Path::new(&repo))
                .ok()
                .and_then(|root| {
                    normalize_repo_file_path(root.as_path(), &raw)
                        .ok()
                        .map(|resolved| resolved.canonical)
                })
                .unwrap_or(raw)
        })
        .filter(|prefix| !prefix.trim().is_empty());

    let filter = NodeListFilter {
        kind: kind.as_deref(),
        language: language.as_deref(),
        subpath: subpath.as_deref(),
        repo_id: repo_id.as_deref(),
    };
    let page = store
        .list_nodes(&filter, limit, offset)
        .context("symbol listing failed")?;

    if cli.json {
        print_json(
            "symbols",
            serde_json::json!({
                "filters": {
                    "kind": kind,
                    "language": language,
                    "subpath": subpath,
                    "repo_id": repo_id,
                },
                "symbols": page
                    .nodes
                    .iter()
                    .map(|node| {
                        let mut symbol = serde_json::json!({
                            "qualified_name": node.qualified_name,
                            "name": node.name,
                            "kind": node.kind.as_str(),
                            "language": node.language,
                            "file": node.file_path,
                            "line_start": node.line_start,
                            "line_end": node.line_end,
                            "parent": node.parent_name,
                            "signature": node.params,
                            "return_type": node.return_type,
                            "repo_id": node
                                .extra_json
                                .get("repo_id")
                                .and_then(serde_json::Value::as_str),
                            "is_test": node.is_test,
                        });
                        // Match MCP shape: omit absent optional keys instead of
                        // emitting explicit nulls.
                        if let Some(object) = symbol.as_object_mut() {
                            object.retain(|_, value| !value.is_null());
                        }
                        symbol
                    })
                    .collect::<Vec<_>>(),
                "total": page.total,
                "limit": page.limit,
                "offset": page.offset,
                "returned": page.nodes.len(),
                "has_more": page.has_more(),
                "next_offset": page.next_offset(),
            }),
        )?;
    } else if page.nodes.is_empty() {
        println!("No symbols matched.");
    } else {
        for node in &page.nodes {
            println!(
                "{}\t{}\t{}:{}-{}",
                node.qualified_name,
                node.kind.as_str(),
                node.file_path,
                node.line_start,
                node.line_end
            );
        }
        println!();
        println!(
            "total {}  returned {}  offset {}",
            page.total,
            page.nodes.len(),
            page.offset
        );
        if let Some(next) = page.next_offset() {
            let mut hint = format!(
                "next page: atlas symbols --offset {next} --limit {}",
                page.limit
            );
            if let Some(kind) = &kind {
                hint.push_str(&format!(" --kind {kind}"));
            }
            if let Some(language) = &language {
                hint.push_str(&format!(" --language {language}"));
            }
            if let Some(subpath) = &subpath {
                hint.push_str(&format!(" --subpath {subpath}"));
            }
            if let Some(repo_id) = &repo_id {
                hint.push_str(&format!(" --repo-id {repo_id}"));
            }
            println!("{hint}");
        }
    }
    Ok(())
}
