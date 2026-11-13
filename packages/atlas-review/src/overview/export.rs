//! ICM-G3 — bounded analysis packet export.

use std::collections::BTreeSet;

use atlas_core::model::{FileRecord, Node};
use atlas_core::{AtlasError, Result};
use atlas_repo::{DiffTarget, changed_files};
use atlas_session::{
    OVERVIEW_SCHEMA_VERSION, OverviewAnalysisPacket, OverviewCodeExcerpt, OverviewExportRequest,
    OverviewExportScope, OverviewPacketBudget, OverviewPacketEvidence, OverviewSubject,
    OverviewSubjectKind, SessionStore, packet_id, subject_id,
};
use atlas_store_sqlite::Store;
use camino::Utf8Path;

use super::types::OverviewExportResult;
use super::util;

const DEFAULT_SUBJECT_LIMIT: usize = 50;
const MAX_SUBJECT_LIMIT: usize = 200;
const DEFAULT_MAX_TOKENS: usize = 2_048;
const MAX_MAX_TOKENS: usize = 8_192;

/// Export bounded overview analysis packets. Atlas never calls a model here;
/// it only assembles evidence for an external analyzer.
pub fn export_overview_packets(
    repo_root: &str,
    db_path: &str,
    store: &Store,
    mut session: Option<&mut SessionStore>,
    request: &OverviewExportRequest,
) -> Result<OverviewExportResult> {
    let graph = util::graph_provenance(store, db_path, repo_root)?;
    util::ensure_graph_not_corrupt(&graph)?;

    let commit_sha = util::current_commit_sha(repo_root);
    let freshness = util::graph_freshness(&graph, &commit_sha);
    let nodes = store.list_all_nodes()?;
    let edges = store.list_all_edges()?;
    let files = store.list_files()?;
    let (by_qname, by_file) = util::index_nodes(&nodes);

    let mut subjects = resolve_subjects(
        repo_root,
        &commit_sha,
        request,
        &nodes,
        &files,
        &by_qname,
        &by_file,
    )?;
    subjects.sort_by(|left, right| {
        left.kind
            .specificity()
            .cmp(&right.kind.specificity())
            .then_with(|| subject_label(left).cmp(&subject_label(right)))
    });
    subjects.dedup_by(|left, right| subject_id(left) == subject_id(right));

    let limit = request
        .limit
        .unwrap_or(DEFAULT_SUBJECT_LIMIT)
        .clamp(1, MAX_SUBJECT_LIMIT);
    let subject_count = subjects.len();
    let omitted_subject_count = subject_count.saturating_sub(limit);
    subjects.truncate(limit);

    let max_tokens = request
        .max_tokens
        .unwrap_or(DEFAULT_MAX_TOKENS)
        .clamp(256, MAX_MAX_TOKENS);

    let mut packets = Vec::with_capacity(subjects.len());
    for subject in &subjects {
        let packet = build_packet(
            repo_root,
            &commit_sha,
            &graph,
            &freshness,
            subject,
            request,
            &nodes,
            &edges,
            &by_file,
            &by_qname,
            max_tokens,
        )?;
        if let Some(session) = session.as_deref_mut() {
            let _ = session.record_overview_packet(
                &packet.packet_id,
                repo_root,
                &packet.subject_id,
                &commit_sha,
                &packet.evidence_ids,
            );
        }
        packets.push(packet);
    }

    // One rollup flag for the whole export: subject truncation and per-packet
    // budget degradation are the same signal to callers.
    let packet_budget_hit = packets.iter().any(|packet| packet.budget.budget_hit);
    let omitted_bytes = packets
        .iter()
        .map(|packet| packet.budget.omitted_bytes)
        .sum();

    Ok(OverviewExportResult {
        schema_version: OVERVIEW_SCHEMA_VERSION,
        repo_root: repo_root.to_owned(),
        commit_sha,
        graph,
        freshness: freshness.clone(),
        packets,
        subject_count,
        omitted_subject_count,
        budget: OverviewPacketBudget {
            max_tokens,
            budget_hit: omitted_subject_count > 0 || packet_budget_hit,
            omitted_bytes,
        },
    })
}

fn resolve_subjects(
    repo_root: &str,
    commit_sha: &str,
    request: &OverviewExportRequest,
    nodes: &[Node],
    files: &[FileRecord],
    by_qname: &std::collections::HashMap<String, usize>,
    by_file: &std::collections::HashMap<String, Vec<usize>>,
) -> Result<Vec<OverviewSubject>> {
    if !request.subjects.is_empty() {
        let packages = util::collect_packages(files, nodes);
        let modules = util::collect_modules(files);
        let mut subjects = Vec::new();
        for target in &request.subjects {
            let canonical = util::canonical_path(repo_root, target).ok();
            let file_match = canonical.as_deref().and_then(|canonical| {
                files
                    .iter()
                    .find(|file| file.path == canonical)
                    .map(|file| util::file_subject(file, repo_root, commit_sha))
            });
            let node_match = by_qname
                .get(target)
                .map(|index| &nodes[*index])
                .filter(|node| node.kind != atlas_core::NodeKind::File)
                .filter(|node| match request.scope {
                    OverviewExportScope::Functions => node.kind == atlas_core::NodeKind::Function,
                    OverviewExportScope::Methods => node.kind == atlas_core::NodeKind::Method,
                    _ => true,
                })
                .map(|node| util::node_subject(node, repo_root, commit_sha));
            let package_match = packages
                .iter()
                .any(|package| package == target)
                .then(|| util::package_subject(target, repo_root, commit_sha));
            let module_match = modules
                .iter()
                .any(|module| module == target)
                .then(|| util::module_subject(target, repo_root, commit_sha));

            let allows = |kind: &str| match request.scope {
                OverviewExportScope::Files => kind == "file",
                OverviewExportScope::Modules => kind == "module",
                OverviewExportScope::Packages => kind == "package",
                OverviewExportScope::Symbols
                | OverviewExportScope::Functions
                | OverviewExportScope::Methods => kind == "symbol",
                OverviewExportScope::Project | OverviewExportScope::Changed => true,
            };

            let mut matches = Vec::new();
            if allows("file")
                && let Some(subject) = file_match
            {
                matches.push(("file", subject));
            }
            if allows("symbol")
                && let Some(subject) = node_match
            {
                matches.push(("symbol", subject));
            }
            if allows("package")
                && let Some(subject) = package_match
            {
                matches.push(("package", subject));
            }
            if allows("module")
                && let Some(subject) = module_match
            {
                matches.push(("module", subject));
            }
            // A canonical file path also resolves through the file node's
            // qualified name. Prefer the file subject in that case, and prefer
            // a package over its containing module path.
            if matches.iter().any(|(kind, _)| *kind == "file") {
                matches.retain(|(kind, _)| *kind == "file");
            }
            if matches.is_empty() {
                return Err(AtlasError::Other(format!(
                    "overview export subject not found: {target}"
                )));
            }
            if matches.len() > 1 {
                let candidates = matches
                    .iter()
                    .map(|(kind, subject)| format!("{} ({kind})", subject_label(subject)))
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(AtlasError::Other(format!(
                    "overview export subject '{target}' is ambiguous; candidates: {candidates}"
                )));
            }
            subjects.push(matches.remove(0).1);
        }
        return Ok(subjects);
    }

    if request.changed_only || request.scope == OverviewExportScope::Changed {
        let target = match request.since.as_deref() {
            Some(since) => DiffTarget::BaseRef(since.to_owned()),
            None => DiffTarget::WorkingTree,
        };
        let changed = match changed_files(Utf8Path::new(repo_root), &target) {
            Ok(changed) => changed,
            Err(_) if request.since.is_none() => Vec::new(),
            Err(error) => {
                return Err(AtlasError::Other(format!(
                    "cannot read changed files: {error}"
                )));
            }
        };
        let mut changed_paths = changed
            .into_iter()
            .map(|change| change.path)
            .collect::<Vec<_>>();
        changed_paths.sort();
        changed_paths.dedup();
        return Ok(changed_paths
            .into_iter()
            .filter_map(|path| {
                let canonical = util::canonical_path(repo_root, &path).ok()?;
                files
                    .iter()
                    .find(|file| file.path == canonical)
                    .map(|file| util::file_subject(file, repo_root, commit_sha))
                    .or_else(|| {
                        by_file.get(&canonical).and_then(|indexes| {
                            indexes.first().map(|index| {
                                util::node_subject(&nodes[*index], repo_root, commit_sha)
                            })
                        })
                    })
            })
            .collect());
    }

    let subjects = match request.scope {
        OverviewExportScope::Project => {
            vec![util::project_subject(repo_root, commit_sha)]
        }
        OverviewExportScope::Packages => util::collect_packages(files, nodes)
            .into_iter()
            .map(|package| util::package_subject(&package, repo_root, commit_sha))
            .collect(),
        OverviewExportScope::Modules => util::collect_modules(files)
            .into_iter()
            .map(|module| util::module_subject(&module, repo_root, commit_sha))
            .collect(),
        OverviewExportScope::Files => files
            .iter()
            .map(|file| util::file_subject(file, repo_root, commit_sha))
            .collect(),
        OverviewExportScope::Symbols => nodes
            .iter()
            .filter(|node| {
                !matches!(
                    node.kind,
                    atlas_core::NodeKind::File
                        | atlas_core::NodeKind::Package
                        | atlas_core::NodeKind::Import
                )
            })
            .map(|node| util::node_subject(node, repo_root, commit_sha))
            .collect(),
        OverviewExportScope::Functions => nodes
            .iter()
            .filter(|node| node.kind == atlas_core::NodeKind::Function)
            .map(|node| util::node_subject(node, repo_root, commit_sha))
            .collect(),
        OverviewExportScope::Methods => nodes
            .iter()
            .filter(|node| node.kind == atlas_core::NodeKind::Method)
            .map(|node| util::node_subject(node, repo_root, commit_sha))
            .collect(),
        OverviewExportScope::Changed => Vec::new(),
    };
    Ok(subjects)
}

#[allow(clippy::too_many_arguments)]
fn build_packet(
    repo_root: &str,
    commit_sha: &str,
    graph: &atlas_session::OverviewGraphProvenance,
    freshness: &atlas_session::OverviewPacketFreshness,
    subject: &OverviewSubject,
    request: &OverviewExportRequest,
    nodes: &[Node],
    edges: &[atlas_core::model::Edge],
    by_file: &std::collections::HashMap<String, Vec<usize>>,
    by_qname: &std::collections::HashMap<String, usize>,
    max_tokens: usize,
) -> Result<OverviewAnalysisPacket> {
    let matched = util::matching_nodes(subject, nodes, by_file);
    let qnames = matched
        .iter()
        .map(|node| node.qualified_name.as_str())
        .collect::<BTreeSet<_>>();

    let callers = if request.include_callers {
        util::edge_callers(edges, &qnames)
    } else {
        Vec::new()
    };
    let callees = if request.include_callees {
        util::edge_callees(edges, &qnames)
    } else {
        Vec::new()
    };
    // Caller/callee arrays are symbol relationships. Drop file nodes and
    // expression-level call targets that do not resolve to a graph symbol.
    let resolvable_symbol = |qname: &str| {
        by_qname.get(qname).is_some_and(|index| {
            !matches!(
                nodes[*index].kind,
                atlas_core::NodeKind::File | atlas_core::NodeKind::Import
            )
        })
    };
    let callers = callers
        .into_iter()
        .filter(|qname| resolvable_symbol(qname))
        .collect::<Vec<_>>();
    let callees = callees
        .into_iter()
        .filter(|qname| resolvable_symbol(qname))
        .collect::<Vec<_>>();

    let mut related_files = BTreeSet::new();
    for qname in callers.iter().chain(callees.iter()) {
        if let Some(index) = by_qname.get(qname) {
            related_files.insert(nodes[*index].file_path.clone());
        }
    }

    let tests = if request.include_tests {
        util::tests(&matched, 20)
    } else {
        Vec::new()
    };

    let mut evidence_ids = matched
        .iter()
        .map(|node| format!("node:{}", node.id))
        .take(20)
        .collect::<Vec<_>>();
    if request.include_callers || request.include_callees {
        for edge in edges {
            if qnames.contains(edge.source_qn.as_str()) || qnames.contains(edge.target_qn.as_str())
            {
                evidence_ids.push(format!("edge:{}", edge.id));
                if evidence_ids.len() >= 40 {
                    break;
                }
            }
        }
    }

    let code_excerpt = if request.include_code_spans {
        excerpt_for(subject, &matched, repo_root)
    } else {
        None
    };

    let subject_identifier = subject_id(subject);
    let mut packet = OverviewAnalysisPacket {
        schema_version: OVERVIEW_SCHEMA_VERSION,
        packet_id: String::new(),
        repo_root: repo_root.to_owned(),
        commit_sha: commit_sha.to_owned(),
        graph: graph.clone(),
        freshness: freshness.clone(),
        subject: subject.clone(),
        subject_id: subject_identifier.clone(),
        evidence: OverviewPacketEvidence {
            node_count: matched.len(),
            edge_count: evidence_ids.len().saturating_sub(matched.len().min(20)),
            symbols: util::symbol_names(&matched, 20),
            files: util::distinct_files(&matched, 20),
        },
        code_excerpt,
        callers,
        callees,
        related_files: related_files.into_iter().collect(),
        tests,
        retrieval_hints: vec![format!("atlas overview search {}", subject_label(subject))],
        evidence_ids,
        safe_to_answer: freshness.safe_to_answer,
        omitted_subject_count: 0,
        budget: OverviewPacketBudget {
            max_tokens,
            budget_hit: false,
            omitted_bytes: 0,
        },
    };

    apply_packet_budget(&mut packet, max_tokens);
    packet.packet_id = packet_id(&subject_identifier, commit_sha, &packet.evidence_ids);
    Ok(packet)
}

fn apply_packet_budget(packet: &mut OverviewAnalysisPacket, max_tokens: usize) {
    let byte_limit = max_tokens.saturating_mul(4);
    let requested = serde_json::to_vec(packet)
        .map(|bytes| bytes.len())
        .unwrap_or(0);
    if requested <= byte_limit {
        return;
    }
    packet.code_excerpt = None;
    packet.evidence.symbols.truncate(5);
    packet.evidence.files.truncate(5);
    packet.callers.truncate(5);
    packet.callees.truncate(5);
    packet.related_files.truncate(5);
    packet.retrieval_hints.truncate(1);
    let emitted = serde_json::to_vec(packet)
        .map(|bytes| bytes.len())
        .unwrap_or(0);
    packet.budget.budget_hit = true;
    packet.budget.omitted_bytes = requested.saturating_sub(emitted);
}

fn excerpt_for(
    subject: &OverviewSubject,
    matched: &[&Node],
    repo_root: &str,
) -> Option<OverviewCodeExcerpt> {
    let (path, line_start, line_end) = match subject.kind {
        OverviewSubjectKind::File => {
            let path = subject.canonical_file_path.as_deref()?;
            let end = matched
                .iter()
                .map(|node| node.line_end)
                .max()
                .unwrap_or(200);
            (path.to_owned(), 1, end.min(200))
        }
        OverviewSubjectKind::Symbol
        | OverviewSubjectKind::Function
        | OverviewSubjectKind::Method => {
            let node = matched.first()?;
            (node.file_path.clone(), node.line_start, node.line_end)
        }
        _ => return None,
    };
    let text = util::read_excerpt(repo_root, &path, line_start, line_end, 2_048)?;
    Some(OverviewCodeExcerpt {
        path,
        line_start,
        line_end,
        text,
    })
}

fn subject_label(subject: &OverviewSubject) -> String {
    subject
        .qualified_name
        .clone()
        .or_else(|| subject.canonical_file_path.clone())
        .or_else(|| subject.package_name.clone())
        .or_else(|| subject.module_path.clone())
        .unwrap_or_else(|| subject.kind.as_str().to_owned())
}
