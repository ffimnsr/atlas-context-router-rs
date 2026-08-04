use super::*;

#[derive(Debug, Clone)]
pub(super) struct CallableFingerprint {
    pub(super) summary: InsightSymbolSummary,
    pub(super) arity: usize,
    pub(super) name_tokens: BTreeSet<String>,
    pub(super) signature_tokens: BTreeSet<String>,
    pub(super) body_shingles: BTreeSet<String>,
    pub(super) duplicate_shingles: BTreeSet<String>,
    pub(super) duplicate_signature: String,
    pub(super) duplicate_summary: String,
    pub(super) neighbor_names: BTreeSet<String>,
    pub(super) loc: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PersistedFingerprintCache {
    pub(super) version: u32,
    pub(super) files: BTreeMap<String, PersistedFingerprintFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PersistedFingerprintFile {
    pub(super) file_hash: String,
    pub(super) callables: BTreeMap<String, PersistedCallableFingerprint>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct PersistedCallableFingerprint {
    pub(super) arity: usize,
    pub(super) loc: usize,
    pub(super) name_tokens: Vec<String>,
    pub(super) signature_tokens: Vec<String>,
    pub(super) body_shingles: Vec<String>,
    pub(super) duplicate_shingles: Vec<String>,
    pub(super) duplicate_signature: String,
    pub(super) duplicate_summary: String,
}

impl PersistedFingerprintCache {
    fn empty() -> Self {
        Self {
            version: FINGERPRINT_CACHE_VERSION,
            files: BTreeMap::new(),
        }
    }
}

impl PersistedCallableFingerprint {
    fn from_runtime(fingerprint: &CallableFingerprint) -> Self {
        Self {
            arity: fingerprint.arity,
            loc: fingerprint.loc,
            name_tokens: fingerprint.name_tokens.iter().cloned().collect(),
            signature_tokens: fingerprint.signature_tokens.iter().cloned().collect(),
            body_shingles: fingerprint.body_shingles.iter().cloned().collect(),
            duplicate_shingles: fingerprint.duplicate_shingles.iter().cloned().collect(),
            duplicate_signature: fingerprint.duplicate_signature.clone(),
            duplicate_summary: fingerprint.duplicate_summary.clone(),
        }
    }

    fn to_runtime(
        &self,
        summary: InsightSymbolSummary,
        neighbor_names: BTreeSet<String>,
    ) -> CallableFingerprint {
        CallableFingerprint {
            summary,
            arity: self.arity,
            name_tokens: self.name_tokens.iter().cloned().collect(),
            signature_tokens: self.signature_tokens.iter().cloned().collect(),
            body_shingles: self.body_shingles.iter().cloned().collect(),
            duplicate_shingles: self.duplicate_shingles.iter().cloned().collect(),
            duplicate_signature: self.duplicate_signature.clone(),
            duplicate_summary: self.duplicate_summary.clone(),
            neighbor_names,
            loc: self.loc,
        }
    }
}

pub(super) fn callable_fingerprints(
    repo_root: &Path,
    snapshot: &GraphSnapshot,
    module_by_qname: &HashMap<String, String>,
) -> Result<Vec<CallableFingerprint>> {
    let node_by_qname = snapshot
        .nodes
        .iter()
        .map(|node| (node.qualified_name.clone(), node))
        .collect::<HashMap<_, _>>();
    let mut outbound = HashMap::<String, BTreeSet<String>>::new();
    for edge in &snapshot.edges {
        if !matches!(
            edge.kind,
            EdgeKind::Calls | EdgeKind::References | EdgeKind::Imports
        ) {
            continue;
        }
        let target_name = node_by_qname
            .get(&edge.target_qn)
            .map(|node| node.name.clone())
            .unwrap_or_else(|| {
                edge.target_qn
                    .rsplit("::")
                    .next()
                    .unwrap_or(&edge.target_qn)
                    .to_owned()
            });
        outbound
            .entry(edge.source_qn.clone())
            .or_default()
            .insert(target_name);
    }

    let callable_nodes_by_file = snapshot
        .nodes
        .iter()
        .filter(|node| is_callable_node(node))
        .fold(BTreeMap::<String, Vec<&Node>>::new(), |mut acc, node| {
            acc.entry(node.file_path.clone()).or_default().push(node);
            acc
        });
    let mut cache = load_persisted_fingerprint_cache(repo_root);
    let mut updated_cache_files = BTreeMap::new();
    let mut fingerprints = Vec::new();
    for (file_path, nodes) in callable_nodes_by_file {
        if let Some((cached, persisted)) = restore_cached_file_fingerprints(
            cache.files.get(&file_path),
            snapshot
                .file_hash_by_file
                .get(&file_path)
                .map(String::as_str),
            &nodes,
            module_by_qname,
            &mut outbound,
        ) {
            fingerprints.extend(cached);
            updated_cache_files.insert(file_path, persisted);
            continue;
        }

        let source = fs::read_to_string(repo_root.join(&file_path)).unwrap_or_default();
        let built = build_file_fingerprints(&source, &nodes, module_by_qname, &mut outbound);
        let file_hash = snapshot
            .file_hash_by_file
            .get(&file_path)
            .cloned()
            .unwrap_or_default();
        updated_cache_files.insert(file_path, persisted_fingerprint_file(&file_hash, &built));
        fingerprints.extend(built);
    }

    cache.version = FINGERPRINT_CACHE_VERSION;
    cache.files = updated_cache_files;
    persist_fingerprint_cache(repo_root, &cache);

    fingerprints.sort_by(|left, right| {
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
    Ok(fingerprints)
}

pub(super) fn restore_cached_file_fingerprints(
    cached: Option<&PersistedFingerprintFile>,
    file_hash: Option<&str>,
    nodes: &[&Node],
    module_by_qname: &HashMap<String, String>,
    outbound: &mut HashMap<String, BTreeSet<String>>,
) -> Option<(Vec<CallableFingerprint>, PersistedFingerprintFile)> {
    let cached = cached?;
    let file_hash = file_hash?;
    if file_hash.is_empty() || cached.file_hash != file_hash {
        return None;
    }
    let current_qnames = nodes
        .iter()
        .map(|node| node.qualified_name.as_str())
        .collect::<BTreeSet<_>>();
    let cached_qnames = cached
        .callables
        .keys()
        .map(|qname| qname.as_str())
        .collect::<BTreeSet<_>>();
    if current_qnames != cached_qnames {
        return None;
    }

    let mut fingerprints = Vec::with_capacity(nodes.len());
    for node in nodes {
        let persisted = cached.callables.get(&node.qualified_name)?;
        let module_id = module_by_qname
            .get(&node.qualified_name)
            .cloned()
            .unwrap_or_else(|| "module:<unknown>".to_owned());
        let summary = symbol_summary(node, module_id);
        let neighbor_names = outbound.remove(&node.qualified_name).unwrap_or_default();
        fingerprints.push(persisted.to_runtime(summary, neighbor_names));
    }
    Some((fingerprints, cached.clone()))
}

pub(super) fn build_file_fingerprints(
    source: &str,
    nodes: &[&Node],
    module_by_qname: &HashMap<String, String>,
    outbound: &mut HashMap<String, BTreeSet<String>>,
) -> Vec<CallableFingerprint> {
    nodes
        .iter()
        .map(|node| {
            let module_id = module_by_qname
                .get(&node.qualified_name)
                .cloned()
                .unwrap_or_else(|| "module:<unknown>".to_owned());
            let summary = symbol_summary(node, module_id);
            let name_tokens = tokenize_identifier(&node.name);
            let signature_tokens = signature_tokens(node);
            let source_excerpt = source_excerpt_from_text(source, node).unwrap_or_default();
            let body_tokens = tokenize_source(&source_excerpt);
            let body_shingles = shingles(&body_tokens, SIMILAR_SHINGLE_SIZE);
            let duplicate_tokens = normalize_duplicate_tokens(&source_excerpt);
            let duplicate_shingles = shingles(&duplicate_tokens, DUPLICATE_SHINGLE_SIZE);
            let duplicate_signature = duplicate_tokens.join(" ");
            let duplicate_summary = summarize_duplicate_pattern(&duplicate_tokens);
            let neighbor_names = outbound.remove(&node.qualified_name).unwrap_or_default();
            CallableFingerprint {
                arity: parse_arity(node.params.as_deref()),
                loc: node.line_end.saturating_sub(node.line_start) as usize + 1,
                summary,
                name_tokens,
                signature_tokens,
                body_shingles,
                duplicate_shingles,
                duplicate_signature,
                duplicate_summary,
                neighbor_names,
            }
        })
        .collect()
}

pub(super) fn persisted_fingerprint_file(
    file_hash: &str,
    fingerprints: &[CallableFingerprint],
) -> PersistedFingerprintFile {
    let callables = fingerprints
        .iter()
        .map(|fingerprint| {
            (
                fingerprint.summary.qualified_name.clone(),
                PersistedCallableFingerprint::from_runtime(fingerprint),
            )
        })
        .collect::<BTreeMap<_, _>>();
    PersistedFingerprintFile {
        file_hash: file_hash.to_owned(),
        callables,
    }
}

pub(super) fn load_persisted_fingerprint_cache(repo_root: &Path) -> PersistedFingerprintCache {
    let cache_path = fingerprint_cache_path(repo_root);
    let Ok(raw) = fs::read_to_string(cache_path) else {
        return PersistedFingerprintCache::empty();
    };
    let Ok(cache) = serde_json::from_str::<PersistedFingerprintCache>(&raw) else {
        return PersistedFingerprintCache::empty();
    };
    if cache.version == FINGERPRINT_CACHE_VERSION {
        cache
    } else {
        PersistedFingerprintCache::empty()
    }
}

pub(super) fn persist_fingerprint_cache(repo_root: &Path, cache: &PersistedFingerprintCache) {
    let cache_path = fingerprint_cache_path(repo_root);
    let Some(parent) = cache_path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(payload) = serde_json::to_vec_pretty(cache) else {
        return;
    };
    let tmp_path = cache_path.with_extension("json.tmp");
    if fs::write(&tmp_path, payload).is_err() {
        return;
    }
    let _ = fs::rename(tmp_path, cache_path);
}

pub(super) fn fingerprint_cache_path(repo_root: &Path) -> std::path::PathBuf {
    repo_root
        .join(atlas_engine::paths::ATLAS_DIR)
        .join(FINGERPRINT_CACHE_FILE)
}
