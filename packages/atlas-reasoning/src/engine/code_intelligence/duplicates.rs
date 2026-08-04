use super::*;

impl<'s> InsightsEngine<'s> {
    pub fn find_duplicates(
        &self,
        repo_root: impl AsRef<Path>,
        request: DuplicateDetectionRequest,
    ) -> Result<DuplicateDetectionAnalysis> {
        let store = self.store().ok_or_else(|| {
            AtlasError::Other(
                "duplicate detection requires a store-backed insights engine".to_owned(),
            )
        })?;
        let snapshot = self.load_graph_snapshot(store, repo_root.as_ref())?;
        let rust_complexity = load_rust_complexity(repo_root.as_ref(), &snapshot.nodes)?;
        let node_metrics = build_node_metrics(self, &snapshot, &rust_complexity);
        let module_by_qname = node_metrics
            .iter()
            .map(|metric| (metric.node.qualified_name.clone(), metric.module_id.clone()))
            .collect::<HashMap<_, _>>();
        let mut fingerprints =
            callable_fingerprints(repo_root.as_ref(), &snapshot, &module_by_qname)?;
        let file_scope = normalize_paths(request.files.as_deref())?
            .into_iter()
            .collect::<BTreeSet<_>>();
        fingerprints.retain(|item| {
            (request.include_tests || !item.summary.file_path.starts_with("tests/"))
                && (file_scope.is_empty() || file_scope.contains(&item.summary.file_path))
        });

        let thresholds = duplicate_thresholds(self.config());
        let min_score = request.min_score.unwrap_or(thresholds.low);
        let limit = request.limit.unwrap_or(self.config().max_findings);
        let suppressions = duplicate_suppressions(self.config(), &request);
        let groups = detect_duplicate_groups(&fingerprints, min_score, limit)
            .into_iter()
            .filter(|group| !duplicate_group_suppressed(group, &suppressions))
            .collect::<Vec<_>>();
        let findings = groups
            .iter()
            .map(|group| duplicate_group_to_finding(group, &thresholds))
            .collect::<Vec<_>>();
        let report = self.pattern_report(findings);
        let retained_ids = retained_finding_ids(&report);
        let groups = groups
            .into_iter()
            .filter(|group| retained_ids.contains(&duplicate_group_id(group)))
            .collect::<Vec<_>>();

        Ok(DuplicateDetectionAnalysis {
            request,
            thresholds,
            report,
            groups,
        })
    }
}

pub(super) fn detect_duplicate_groups(
    fingerprints: &[CallableFingerprint],
    min_score: f64,
    limit: usize,
) -> Vec<DuplicateGroup> {
    let mut exact_groups = BTreeMap::<String, Vec<&CallableFingerprint>>::new();
    for item in fingerprints {
        if item.duplicate_signature.split_whitespace().count() < 6 {
            continue;
        }
        exact_groups
            .entry(item.duplicate_signature.clone())
            .or_default()
            .push(item);
    }

    let mut groups = Vec::new();
    let mut consumed = HashSet::<String>::new();
    for members in exact_groups.values() {
        if members.len() < 2 {
            continue;
        }
        let group = build_duplicate_group("exact_normalized", 1.0, members);
        for member in members {
            consumed.insert(member.summary.qualified_name.clone());
        }
        groups.push(group);
    }

    let candidates = fingerprints
        .iter()
        .filter(|item| !consumed.contains(&item.summary.qualified_name))
        .collect::<Vec<_>>();
    let mut visited = HashSet::<String>::new();
    for (index, seed) in candidates.iter().enumerate() {
        if visited.contains(&seed.summary.qualified_name) {
            continue;
        }
        let mut cluster = vec![*seed];
        let mut cluster_score = 0.0_f64;
        visited.insert(seed.summary.qualified_name.clone());
        for other in candidates.iter().skip(index + 1) {
            if visited.contains(&other.summary.qualified_name)
                || other.summary.language != seed.summary.language
                || other.summary.node_kind != seed.summary.node_kind
            {
                continue;
            }
            if overlap_ratio(seed.loc, other.loc) < 0.55 {
                continue;
            }
            let score = jaccard(&seed.duplicate_shingles, &other.duplicate_shingles);
            if score < min_score {
                continue;
            }
            cluster.push(*other);
            cluster_score = cluster_score.max(score);
            visited.insert(other.summary.qualified_name.clone());
        }
        if cluster.len() >= 2 {
            groups.push(build_duplicate_group(
                "near_duplicate",
                cluster_score.max(min_score),
                &cluster,
            ));
        }
    }

    groups.sort_by(|left, right| {
        right
            .confidence
            .total_cmp(&left.confidence)
            .then_with(|| {
                right
                    .duplicated_token_count
                    .cmp(&left.duplicated_token_count)
            })
            .then_with(|| left.group_id.cmp(&right.group_id))
    });
    if groups.len() > limit {
        groups.truncate(limit);
    }
    groups
}

pub(super) fn build_duplicate_group(
    duplicate_kind: &str,
    confidence: f64,
    members: &[&CallableFingerprint],
) -> DuplicateGroup {
    let mut sorted_members = members
        .iter()
        .map(|item| (*item).clone())
        .collect::<Vec<_>>();
    sorted_members.sort_by(|left, right| {
        left.summary
            .file_path
            .cmp(&right.summary.file_path)
            .then_with(|| left.summary.line_start.cmp(&right.summary.line_start))
            .then_with(|| {
                left.summary
                    .qualified_name
                    .cmp(&right.summary.qualified_name)
            })
    });
    let first = &sorted_members[0];
    let files = sorted_members
        .iter()
        .map(|item| item.summary.file_path.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let duplicated_line_count = sorted_members
        .iter()
        .map(|item| item.loc)
        .min()
        .unwrap_or_default();
    let duplicated_token_count = sorted_members
        .iter()
        .map(|item| item.duplicate_signature.split_whitespace().count())
        .min()
        .unwrap_or_default();
    let member_summaries = sorted_members
        .iter()
        .map(|item| DuplicateMember {
            source_span: SourceSpan {
                file_path: item.summary.file_path.clone(),
                line_start: item.summary.line_start,
                line_end: item.summary.line_end,
            },
            symbol: item.summary.clone(),
            normalized_token_count: item.duplicate_signature.split_whitespace().count(),
        })
        .collect::<Vec<_>>();
    let suggested_extraction_target = common_parent_qname(&sorted_members)
        .or_else(|| common_parent_path(&files))
        .or_else(|| files.first().cloned());
    DuplicateGroup {
        group_id: format!(
            "duplicate:{}:{}:{}:{}",
            duplicate_kind,
            first.summary.file_path,
            first.summary.line_start,
            sorted_members.len()
        ),
        duplicate_kind: duplicate_kind.to_owned(),
        confidence,
        normalized_pattern_summary: first.duplicate_summary.clone(),
        duplicated_line_count,
        duplicated_token_count,
        member_count: sorted_members.len(),
        files,
        members: member_summaries,
        suggested_extraction_target,
    }
}
