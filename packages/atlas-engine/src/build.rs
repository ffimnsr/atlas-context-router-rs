//! Full-build pipeline: scan tracked files, parse, persist to SQLite.

use std::collections::HashMap;
use std::fs;
use std::time::Instant;

use anyhow::{Context, Result};
use atlas_core::model::ParsedFile;
use atlas_parser::ParserRegistry;
use atlas_repo::{collect_supported_files, find_repo_root, hash_file};
use atlas_store_sqlite::Store;
use camino::Utf8Path;
use rayon::prelude::*;

use crate::call_resolution::reconcile_call_targets;

/// Options controlling the build pipeline.
pub struct BuildOptions {
    /// Abort on the first parse or I/O failure instead of continuing.
    pub fail_fast: bool,
    /// Number of files parsed per parallel batch.
    pub batch_size: usize,
}

impl Default for BuildOptions {
    fn default() -> Self {
        Self {
            fail_fast: false,
            batch_size: crate::config::DEFAULT_PARSE_BATCH_SIZE,
        }
    }
}

/// Summary returned by `build_graph`.
#[derive(Debug, Default)]
pub struct BuildSummary {
    pub scanned: usize,
    pub skipped_unsupported: usize,
    pub skipped_unchanged: usize,
    pub parsed: usize,
    pub parse_errors: usize,
    pub nodes_inserted: usize,
    pub edges_inserted: usize,
    pub elapsed_ms: u128,
}

/// Scan `repo_root`, parse all supported changed files, persist graph to `db_path`.
///
/// `repo_root` must be the directory returned by `find_repo_root` (absolute path
/// string).  `db_path` is the path to the Atlas SQLite database, which must
/// already exist (run `atlas init` first).
pub fn build_graph(
    repo_root: &Utf8Path,
    db_path: &str,
    opts: &BuildOptions,
) -> Result<BuildSummary> {
    let started = Instant::now();

    let mut store =
        Store::open(db_path).with_context(|| format!("cannot open database at {db_path}"))?;

    let registry = ParserRegistry::with_defaults();

    let _scan_span = tracing::info_span!("build.scan").entered();

    let stored_hashes: HashMap<String, String> =
        store.file_hashes().context("cannot read stored hashes")?;

    let (all_files, mut skipped_unsupported) =
        collect_supported_files(repo_root, None, |rel_path| {
            registry.supports(rel_path.as_str())
        })
        .context("cannot collect tracked files")?;

    let scanned = all_files.len() + skipped_unsupported;
    let mut skipped_unchanged = 0usize;
    let mut parse_errors = 0usize;

    type Candidate = (String, camino::Utf8PathBuf, String);
    let mut candidates: Vec<Candidate> = Vec::new();

    for rel_path in &all_files {
        let rel_str = rel_path.as_str().to_owned();
        let abs_path = repo_root.join(rel_path);
        let hash = match hash_file(&abs_path) {
            Ok(h) => h,
            Err(e) => {
                tracing::warn!("hashing '{}' failed: {e}", rel_str);
                parse_errors += 1;
                if opts.fail_fast {
                    return Err(e.context(format!("hashing '{rel_str}' failed (--fail-fast)")));
                }
                continue;
            }
        };

        if stored_hashes.get(&rel_str).is_some_and(|h| h == &hash) {
            skipped_unchanged += 1;
            continue;
        }

        candidates.push((rel_str, abs_path, hash));
    }

    drop(_scan_span);

    let _parse_span = tracing::info_span!("build.parse_and_write").entered();

    let mut parsed_count = 0usize;
    let mut total_nodes = 0usize;
    let mut total_edges = 0usize;
    let mut resolved_paths: Vec<String> = Vec::new();

    for chunk in candidates.chunks(opts.batch_size) {
        let results: Vec<(String, Result<ParsedFile, String>)> = chunk
            .par_iter()
            .map(|(rel_str, abs_path, hash)| {
                let source = match fs::read(abs_path.as_std_path()) {
                    Ok(b) => b,
                    Err(e) => return (rel_str.clone(), Err(format!("read error: {e}"))),
                };
                match registry.parse(rel_str, hash, &source, None) {
                    Some((pf, _tree)) => (rel_str.clone(), Ok(pf)),
                    None => (rel_str.clone(), Err("unsupported (skipped)".into())),
                }
            })
            .collect();

        let mut parsed_files: Vec<ParsedFile> = Vec::with_capacity(chunk.len());
        for (rel_str, outcome) in results {
            match outcome {
                Ok(pf) => {
                    parsed_count += 1;
                    resolved_paths.push(pf.path.clone());
                    parsed_files.push(pf);
                }
                Err(msg) if msg == "unsupported (skipped)" => {
                    skipped_unsupported += 1;
                }
                Err(msg) => {
                    tracing::warn!("parsing '{}' failed: {msg}", rel_str);
                    parse_errors += 1;
                    if opts.fail_fast {
                        return Err(anyhow::anyhow!(
                            "parsing '{rel_str}' failed: {msg} (--fail-fast)"
                        ));
                    }
                }
            }
        }

        if !parsed_files.is_empty() {
            let (n, e) = store
                .replace_files_transactional(&parsed_files)
                .context("cannot store parsed files")?;
            total_nodes += n;
            total_edges += e;

            // Index chunk text for retrieval (embeddings generated separately).
            for pf in &parsed_files {
                for node in &pf.nodes {
                    if let Err(err) =
                        store.upsert_chunk(&node.qualified_name, 0, &node.chunk_text())
                    {
                        tracing::warn!("chunk upsert failed for {}: {err:#}", node.qualified_name);
                    }
                }
            }
        }
    }

    drop(_parse_span);

    if !resolved_paths.is_empty()
        && let Err(err) = reconcile_call_targets(&mut store, repo_root, &resolved_paths)
    {
        tracing::warn!("late call-target resolution failed during build: {err:#}");
    }

    Ok(BuildSummary {
        scanned,
        skipped_unsupported,
        skipped_unchanged,
        parsed: parsed_count,
        parse_errors,
        nodes_inserted: total_nodes,
        edges_inserted: total_edges,
        elapsed_ms: started.elapsed().as_millis(),
    })
}

/// Detect and return the repo root for `start_dir`, delegating to git.
///
/// Convenience wrapper exposed for callers that only have a raw path string.
#[allow(dead_code)]
pub fn resolve_repo_root(start_dir: &str) -> Result<camino::Utf8PathBuf> {
    find_repo_root(Utf8Path::new(start_dir)).context("cannot find git repo root")
}
