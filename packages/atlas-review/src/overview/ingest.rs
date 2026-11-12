//! ICM-G4 — external analysis ingest with freshness validation.

use atlas_adapters::{
    ArtifactIdentity, RedactionRules, generate_source_id, redact_text_with_rules,
};
use atlas_contentstore::{ContentStore, OutputRouting, SourceMeta};
use atlas_core::model::Node;
use atlas_core::{AtlasError, Result};
use atlas_session::{
    NewOverviewRecord, OVERVIEW_DESCRIPTION_PREVIEW_MAX_CHARS, OVERVIEW_INLINE_BODY_BUDGET_BYTES,
    OVERVIEW_SCHEMA_VERSION, OverviewAnalysisInput, OverviewFreshness, OverviewSourceKind,
    OverviewSubject, OverviewSubjectKind, SessionStore, packet_id, subject_id, truncate_chars,
};
use atlas_store_sqlite::Store;

use super::types::OverviewIngestResult;
use super::util;

/// Ingest one externally produced analysis record.
///
/// Validation order is intentional: schema version, packet id, subject fields,
/// then text requirements. Stale/orphaned states are explicit, never silently
/// fresh.
pub fn ingest_overview_analysis(
    repo_root: &str,
    store: &Store,
    session: &mut SessionStore,
    content: &mut ContentStore,
    rules: &RedactionRules,
    input: &OverviewAnalysisInput,
) -> Result<OverviewIngestResult> {
    if input.schema_version != OVERVIEW_SCHEMA_VERSION {
        return Err(AtlasError::Other(format!(
            "overview ingest schema_version must be {OVERVIEW_SCHEMA_VERSION}, got {}",
            input.schema_version
        )));
    }

    let mut subject = input.subject.clone();
    if subject.repo_root.trim().is_empty() {
        subject.repo_root = repo_root.to_owned();
    } else if subject.repo_root != repo_root {
        return Err(AtlasError::Other(format!(
            "overview ingest subject repo_root '{}' does not match active repo '{}'",
            subject.repo_root, repo_root
        )));
    }
    let subject = subject.validated()?;
    atlas_session::validate_confidence(input.confidence)?;

    if input.summary.trim().is_empty() {
        return Err(AtlasError::Other(
            "overview ingest requires a non-empty summary".to_owned(),
        ));
    }
    let has_detail = !input.description.trim().is_empty()
        || !input.responsibilities.is_empty()
        || !input.flow.is_empty()
        || !input.inputs.is_empty()
        || !input.outputs.is_empty()
        || !input.invariants.is_empty()
        || !input.gotchas.is_empty()
        || !input.examples.is_empty();
    if !has_detail {
        return Err(AtlasError::Other(
            "overview ingest requires description or at least one non-empty detail array"
                .to_owned(),
        ));
    }

    let subject_identifier = subject_id(&subject);
    let packet_validated = validate_packet(session, input, &subject_identifier)?;

    let nodes = store.list_all_nodes()?;
    let (by_qname, by_file) = util::index_nodes(&nodes);
    let (subject_resolved, span_overlaps) = resolve_subject(&subject, &nodes, &by_qname, &by_file);

    let head = util::current_commit_sha(repo_root);
    let freshness = if !subject_resolved {
        OverviewFreshness::Orphaned
    } else if input.packet_id.is_none() && input.source_kind != OverviewSourceKind::ExternalLlm {
        OverviewFreshness::Unverified
    } else if subject.commit_sha != head
        || (matches!(
            subject.kind,
            OverviewSubjectKind::Symbol
                | OverviewSubjectKind::Function
                | OverviewSubjectKind::Method
        ) && !span_overlaps)
    {
        OverviewFreshness::PossiblyStale
    } else {
        OverviewFreshness::Fresh
    };

    let (redacted, redacted_fields) = redact_input(input, rules)?;
    let was_redacted = !redacted_fields.is_empty();
    let mut metadata = input.metadata.clone();
    if let serde_json::Value::Object(map) = &mut metadata {
        map.insert(
            "freshness_reason".to_owned(),
            serde_json::Value::String(freshness_reason(freshness, subject_resolved, span_overlaps)),
        );
        if !redacted_fields.is_empty() {
            map.insert(
                "redacted_fields".to_owned(),
                serde_json::Value::Array(
                    redacted_fields
                        .into_iter()
                        .map(serde_json::Value::String)
                        .collect(),
                ),
            );
        }
        if let Some(pid) = input.packet_id.as_deref() {
            map.insert(
                "packet_id".to_owned(),
                serde_json::Value::String(pid.to_owned()),
            );
        }
    }

    let (description_preview, body_routed, source_id) = route_description(
        repo_root,
        content,
        &subject_identifier,
        &redacted.description,
        &redacted.examples,
        input.source_ids.first().cloned(),
    )?;

    let graph_last_indexed_at = store.stats().ok().and_then(|stats| stats.last_indexed_at);
    let record = session.insert_overview_record(&NewOverviewRecord {
        subject,
        title: redacted.title,
        summary: redacted.summary,
        description_preview,
        responsibilities: redacted.responsibilities,
        flow: redacted.flow,
        inputs: redacted.inputs,
        outputs: redacted.outputs,
        invariants: redacted.invariants,
        gotchas: redacted.gotchas,
        examples: redacted.examples,
        tags: redacted.tags,
        source_kind: input.source_kind,
        analyzer_name: redacted.analyzer_name,
        analyzer_version: redacted.analyzer_version,
        model_name: redacted.model_name,
        graph_last_indexed_at,
        freshness: Some(freshness),
        confidence: input.confidence,
        source_id,
        supersedes_id: None,
        metadata,
    })?;

    Ok(OverviewIngestResult {
        record,
        subject_resolved,
        packet_validated,
        body_routed,
        redacted: was_redacted,
        status: freshness.as_str().to_owned(),
    })
}

fn validate_packet(
    session: &SessionStore,
    input: &OverviewAnalysisInput,
    subject_identifier: &str,
) -> Result<bool> {
    let Some(provided) = input.packet_id.as_deref() else {
        return Ok(false);
    };
    let expected = packet_id(
        subject_identifier,
        &input.subject.commit_sha,
        &input.evidence_ids,
    );
    if provided == expected {
        return Ok(true);
    }
    // Exported packets are also recorded server-side. Accept a stored packet
    // only when its subject and commit match; otherwise fail clearly.
    if let Some((stored_subject, stored_commit, _repo)) = session.get_overview_packet(provided)? {
        if stored_subject != subject_identifier {
            return Err(AtlasError::Other(format!(
                "overview packet_id '{provided}' was exported for subject {stored_subject}, not {subject_identifier}"
            )));
        }
        if stored_commit != input.subject.commit_sha {
            return Err(AtlasError::Other(format!(
                "overview packet_id '{provided}' was exported for commit {stored_commit}, not {}",
                input.subject.commit_sha
            )));
        }
        return Ok(true);
    }
    Err(AtlasError::Other(format!(
        "overview packet_id mismatch: expected '{expected}' for subject {subject_identifier} and commit {}; echo the exported evidence_ids or omit packet_id",
        input.subject.commit_sha
    )))
}

fn resolve_subject(
    subject: &OverviewSubject,
    nodes: &[Node],
    by_qname: &std::collections::HashMap<String, usize>,
    by_file: &std::collections::HashMap<String, Vec<usize>>,
) -> (bool, bool) {
    match subject.kind {
        OverviewSubjectKind::Project => (true, true),
        OverviewSubjectKind::File => {
            let path = subject.canonical_file_path.as_deref().unwrap_or_default();
            (
                by_file.get(path).is_some_and(|indexes| !indexes.is_empty()),
                true,
            )
        }
        OverviewSubjectKind::Package => {
            let package = subject.package_name.as_deref().unwrap_or_default();
            (
                nodes
                    .iter()
                    .any(|node| util::node_package(node).as_deref() == Some(package)),
                true,
            )
        }
        OverviewSubjectKind::Module => {
            let module = subject.module_path.as_deref().unwrap_or_default();
            (
                nodes
                    .iter()
                    .any(|node| util::module_path_for_file(&node.file_path) == module),
                true,
            )
        }
        OverviewSubjectKind::Symbol
        | OverviewSubjectKind::Function
        | OverviewSubjectKind::Method => {
            let qname = subject.qualified_name.as_deref().unwrap_or_default();
            let path = subject.canonical_file_path.as_deref().unwrap_or_default();
            let Some(index) = by_qname.get(qname) else {
                return (false, false);
            };
            let node = &nodes[*index];
            if node.file_path != path {
                return (false, false);
            }
            let overlaps = span_overlaps(subject, node);
            (true, overlaps)
        }
    }
}

fn span_overlaps(subject: &OverviewSubject, node: &Node) -> bool {
    let start = subject.line_start.unwrap_or(0);
    let end = subject.line_end.unwrap_or(0);
    start <= node.line_end && end >= node.line_start
}

fn freshness_reason(freshness: OverviewFreshness, resolved: bool, span_overlaps: bool) -> String {
    match freshness {
        OverviewFreshness::Fresh => "subject_resolved_and_commit_matches".to_owned(),
        OverviewFreshness::PossiblyStale if !resolved => "subject_unresolved".to_owned(),
        OverviewFreshness::PossiblyStale if !span_overlaps => {
            "line_span_no_longer_overlaps_subject".to_owned()
        }
        OverviewFreshness::PossiblyStale => "commit_sha_differs".to_owned(),
        OverviewFreshness::Orphaned => "subject_no_longer_resolves".to_owned(),
        OverviewFreshness::Unverified => "manual_or_imported_without_packet_id".to_owned(),
        OverviewFreshness::Stale => "direct_change_detected".to_owned(),
    }
}

struct RedactedInput {
    title: String,
    summary: String,
    description: String,
    responsibilities: Vec<String>,
    flow: Vec<String>,
    inputs: Vec<String>,
    outputs: Vec<String>,
    invariants: Vec<String>,
    gotchas: Vec<String>,
    examples: Vec<String>,
    tags: Vec<String>,
    analyzer_name: Option<String>,
    analyzer_version: Option<String>,
    model_name: Option<String>,
}

fn redact_input(
    input: &OverviewAnalysisInput,
    rules: &RedactionRules,
) -> Result<(RedactedInput, Vec<String>)> {
    let mut fields = Vec::new();
    let apply = |field: &str, value: &str, used: &mut Vec<String>| -> String {
        let redacted = redact_text_with_rules(value, rules);
        if redacted != value {
            used.push(field.to_owned());
        }
        redacted
    };
    let mut used = Vec::new();
    let redacted = RedactedInput {
        title: apply("title", &input.title, &mut used),
        summary: apply("summary", &input.summary, &mut used),
        description: apply("description", &input.description, &mut used),
        responsibilities: redact_list(
            "responsibilities",
            &input.responsibilities,
            rules,
            &mut used,
        ),
        flow: redact_list("flow", &input.flow, rules, &mut used),
        inputs: redact_list("inputs", &input.inputs, rules, &mut used),
        outputs: redact_list("outputs", &input.outputs, rules, &mut used),
        invariants: redact_list("invariants", &input.invariants, rules, &mut used),
        gotchas: redact_list("gotchas", &input.gotchas, rules, &mut used),
        examples: redact_list("examples", &input.examples, rules, &mut used),
        tags: redact_list("tags", &input.tags, rules, &mut used),
        analyzer_name: input
            .analyzer_name
            .as_deref()
            .map(|value| apply("analyzer_name", value, &mut used)),
        analyzer_version: input
            .analyzer_version
            .as_deref()
            .map(|value| apply("analyzer_version", value, &mut used)),
        model_name: input
            .model_name
            .as_deref()
            .map(|value| apply("model_name", value, &mut used)),
    };
    fields.extend(used);
    fields.sort();
    fields.dedup();
    Ok((redacted, fields))
}

fn redact_list(
    field: &str,
    values: &[String],
    rules: &RedactionRules,
    used: &mut Vec<String>,
) -> Vec<String> {
    values
        .iter()
        .map(|value| {
            let redacted = redact_text_with_rules(value, rules);
            if redacted != *value {
                used.push(field.to_owned());
            }
            redacted
        })
        .collect()
}

fn route_description(
    repo_root: &str,
    content: &mut ContentStore,
    subject_identifier: &str,
    description: &str,
    examples: &[String],
    input_source_id: Option<String>,
) -> Result<(String, bool, Option<String>)> {
    let (preview, _) = truncate_chars(description, OVERVIEW_DESCRIPTION_PREVIEW_MAX_CHARS);
    let examples_over_budget =
        examples.iter().map(String::len).sum::<usize>() > OVERVIEW_INLINE_BODY_BUDGET_BYTES;
    if description.len() <= OVERVIEW_INLINE_BODY_BUDGET_BYTES && !examples_over_budget {
        return Ok((preview, false, input_source_id));
    }

    let mut body = description.to_owned();
    if !examples.is_empty() {
        body.push_str("\n\n## Examples\n\n");
        for example in examples {
            body.push_str(example);
            body.push('\n');
        }
    }

    let identity = ArtifactIdentity::artifact_label(format!("overview:{subject_identifier}"));
    let source_id = generate_source_id(&identity, &body);
    let meta = SourceMeta {
        id: source_id.clone(),
        session_id: None,
        agent_id: None,
        source_type: "overview_body".to_owned(),
        label: format!("Overview body for {subject_identifier}"),
        repo_root: Some(repo_root.to_owned()),
        repo_roots: vec![repo_root.to_owned()],
        repo_id: None,
        repo_ids: vec![],
        identity_kind: identity.kind_str().to_owned(),
        identity_value: identity.value().to_owned(),
    };
    let routing = content.route_output(meta, &body, "text/markdown")?;
    let routed_source_id = match routing {
        OutputRouting::Raw(_) => None,
        OutputRouting::Preview { source_id, .. } | OutputRouting::Pointer { source_id } => {
            Some(source_id)
        }
    };
    Ok((
        preview,
        routed_source_id.is_some(),
        routed_source_id.or(input_source_id),
    ))
}
