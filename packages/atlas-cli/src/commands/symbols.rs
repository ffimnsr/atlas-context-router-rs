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

    let (
        resolve,
        file,
        kind,
        language,
        subpath,
        exact_dir,
        exclude_subpaths,
        repo_id,
        limit,
        offset,
        allow_stale,
        allow_partial,
    ) = match &cli.command {
        Command::Symbols {
            resolve,
            file,
            kind,
            language,
            subpath,
            exact_dir,
            exclude_subpaths,
            repo_id,
            limit,
            offset,
            allow_stale,
            allow_partial,
        } => (
            resolve.clone(),
            file.clone(),
            kind.clone(),
            language.clone(),
            subpath.clone(),
            exact_dir.clone(),
            exclude_subpaths.clone(),
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
    let normalize = |raw: String| {
        find_repo_root(Utf8Path::new(&repo))
            .ok()
            .and_then(|root| {
                normalize_repo_file_path(root.as_path(), &raw)
                    .ok()
                    .map(|resolved| resolved.canonical)
            })
            .unwrap_or(raw)
    };
    let subpath = subpath
        .map(&normalize)
        .filter(|prefix| !prefix.trim().is_empty());
    let exact_dir = exact_dir
        .map(&normalize)
        .filter(|scope| !scope.trim().is_empty());
    let exclude_subpaths: Vec<String> = exclude_subpaths
        .into_iter()
        .map(normalize)
        .filter(|scope| !scope.trim().is_empty())
        .collect();

    if let Some(name) = resolve {
        let file_filter = file.map(&normalize);
        return run_symbol_resolve(
            cli,
            &store,
            &name,
            file_filter.as_deref(),
            kind.as_deref(),
            language.as_deref(),
            limit,
        );
    }

    let filter = NodeListFilter {
        kind: kind.as_deref(),
        language: language.as_deref(),
        subpath: subpath.as_deref(),
        repo_id: repo_id.as_deref(),
        exact_dir: exact_dir.as_deref(),
        exclude_subpaths: &exclude_subpaths,
        include_synthetic: false,
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
                    "exact_dir": exact_dir,
                    "exclude_subpaths": exclude_subpaths,
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

/// Resolve one symbol without fuzzy fallback.
///
/// Order: exact qualified name, exact short name, then FTS prefix candidates.
/// `file`, `kind`, and `language` filters apply before ranking so a unique
/// post-filter match resolves deterministically.
fn run_symbol_resolve(
    cli: &Cli,
    store: &Store,
    name: &str,
    file: Option<&str>,
    kind: Option<&str>,
    language: Option<&str>,
    limit: usize,
) -> Result<()> {
    // One resolution ladder shared with MCP `resolve_symbol`: exact qualified
    // name, exact name, then name prefix. Fuzzy recovery stays opt-in.
    let resolution = atlas_search::resolve_symbol_nodes(
        store,
        &atlas_search::SymbolResolveRequest {
            name,
            kind,
            language,
            file,
            limit: limit.max(1),
            fuzzy: false,
        },
    )
    .context("symbol resolution failed")?;

    let status = resolution.status();
    let matches = resolution.matches;
    let match_count = matches.len();
    let best_match = matches.first();

    if cli.json {
        print_json(
            "symbols",
            serde_json::json!({
                "tool": "symbols",
                "action": "resolve",
                "query": {
                    "name": name,
                    "file": file,
                    "kind": kind,
                    "language": language,
                },
                "status": status,
                "ambiguous": match_count > 1,
                "resolution_stage": resolution.stage.map(|stage| stage.as_str()),
                "match_count": match_count,
                "best_match": best_match.map(symbol_json).unwrap_or(serde_json::Value::Null),
                "matches": matches.iter().map(symbol_json).collect::<Vec<_>>(),
                "truncated": resolution.truncated,
            }),
        )?;
    } else if let Some(node) = best_match {
        println!(
            "{}\t{}\t{}:{}-{}",
            node.qualified_name,
            node.kind.as_str(),
            node.file_path,
            node.line_start,
            node.line_end
        );
        if match_count > 1 {
            eprintln!("{match_count} symbols matched; showing best match.");
        }
    } else {
        println!("No symbols matched.");
    }
    Ok(())
}

fn symbol_json(node: &atlas_core::Node) -> serde_json::Value {
    serde_json::json!({
        "qualified_name": node.qualified_name,
        "name": node.name,
        "kind": node.kind.as_str(),
        "language": node.language,
        "file_path": node.file_path,
        "line_start": node.line_start,
        "line_end": node.line_end,
    })
}
