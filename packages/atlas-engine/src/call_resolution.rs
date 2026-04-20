use std::collections::{HashMap, HashSet};
use std::fs;

use anyhow::Result;
use atlas_core::{Edge, Node, NodeKind};
use atlas_store_sqlite::Store;
use camino::{Utf8Path, Utf8PathBuf};

pub fn reconcile_call_targets(
    store: &mut Store,
    repo_root: &Utf8Path,
    file_paths: &[String],
) -> Result<usize> {
    let mut touched_files = 0usize;
    let mut seen = HashSet::new();
    let mut candidate_cache: HashMap<(String, String), Vec<Node>> = HashMap::new();
    let mut owner_cache: HashMap<String, Option<String>> = HashMap::new();
    let config = load_resolver_config(repo_root);

    for path in file_paths {
        if !seen.insert(path.clone()) {
            continue;
        }

        let nodes = store.nodes_by_file(path)?;
        if nodes.is_empty() {
            continue;
        }
        let language = nodes
            .iter()
            .find(|node| node.kind == NodeKind::File)
            .or_else(|| nodes.first())
            .map(|node| node.language.clone())
            .unwrap_or_default();
        if language.is_empty() {
            continue;
        }

        let import_bindings = collect_import_bindings(&nodes);
        let mut edges = store.edges_by_file(path)?;
        let mut changed = false;
        let resolution_ctx = ResolutionContext {
            store,
            repo_root,
            path,
            language: &language,
            config: &config,
        };

        for edge in &mut edges {
            if edge.kind != atlas_core::EdgeKind::Calls {
                continue;
            }
            let Some(meta) = call_meta(edge) else {
                continue;
            };
            if edge.confidence_tier.as_deref() == Some("same_file") {
                continue;
            }

            let resolved = if meta.receiver_text.is_some() {
                resolve_import_target(
                    &resolution_ctx,
                    &meta,
                    &import_bindings,
                    &mut candidate_cache,
                )
                .or_else(|| {
                    resolve_same_package_target(
                        store,
                        path,
                        &language,
                        &meta.callee_name,
                        &mut candidate_cache,
                        &mut owner_cache,
                    )
                })
            } else {
                resolve_import_target(
                    &resolution_ctx,
                    &meta,
                    &import_bindings,
                    &mut candidate_cache,
                )
                .or_else(|| {
                    resolve_same_package_target(
                        store,
                        path,
                        &language,
                        &meta.callee_name,
                        &mut candidate_cache,
                        &mut owner_cache,
                    )
                })
            };

            let Some((target_qn, tier, confidence)) = resolved else {
                continue;
            };
            if edge.target_qn == target_qn && edge.confidence_tier.as_deref() == Some(tier) {
                continue;
            }

            edge.target_qn = target_qn;
            edge.confidence_tier = Some(tier.to_owned());
            edge.confidence = confidence;
            changed = true;
        }

        if changed {
            store.rewrite_file_edges(path, &edges)?;
            touched_files += 1;
        }
    }

    Ok(touched_files)
}

#[derive(Clone)]
struct CallMeta {
    callee_name: String,
    receiver_text: Option<String>,
}

#[derive(Clone)]
struct ImportBinding {
    source: String,
    local: String,
    imported: Option<String>,
    kind: String,
    relative_level: usize,
}

struct ResolutionContext<'a> {
    store: &'a Store,
    repo_root: &'a Utf8Path,
    path: &'a str,
    language: &'a str,
    config: &'a ResolverConfig,
}

#[derive(Default)]
struct ResolverConfig {
    tsconfigs: Vec<TsConfigEntry>,
    go_module: Option<String>,
}

struct TsConfigEntry {
    dir: Utf8PathBuf,
    config: TsConfig,
}

struct TsConfig {
    base_url: Utf8PathBuf,
    base_url_explicit: bool,
    paths: Vec<TsPathAlias>,
}

struct TsPathAlias {
    pattern: String,
    targets: Vec<String>,
}

fn load_resolver_config(repo_root: &Utf8Path) -> ResolverConfig {
    ResolverConfig {
        tsconfigs: load_tsconfigs(repo_root),
        go_module: load_go_module(repo_root),
    }
}

fn load_tsconfigs(repo_root: &Utf8Path) -> Vec<TsConfigEntry> {
    let mut entries = Vec::new();
    collect_tsconfigs(repo_root, Utf8Path::new(""), &mut entries);
    entries.sort_by(|left, right| {
        right
            .dir
            .components()
            .count()
            .cmp(&left.dir.components().count())
    });
    entries
}

fn collect_tsconfigs(repo_root: &Utf8Path, rel_dir: &Utf8Path, entries: &mut Vec<TsConfigEntry>) {
    let abs_dir = repo_root.join(rel_dir);
    let Ok(read_dir) = fs::read_dir(abs_dir.as_std_path()) else {
        return;
    };

    for entry in read_dir.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let child_rel = rel_dir.join(name.as_ref());

        if entry.file_type().ok().is_some_and(|kind| kind.is_dir()) {
            if matches!(name.as_ref(), ".git" | "target" | ".atlas") {
                continue;
            }
            collect_tsconfigs(repo_root, &child_rel, entries);
            continue;
        }

        if !matches!(name.as_ref(), "tsconfig.json" | "jsconfig.json") {
            continue;
        }

        if let Some(config) = parse_tsconfig(repo_root, &child_rel) {
            let dir_path = child_rel.parent().unwrap_or_else(|| Utf8Path::new(""));
            entries.push(TsConfigEntry {
                dir: dir_path.to_owned(),
                config,
            });
        }
    }
}

fn parse_tsconfig(repo_root: &Utf8Path, rel_path: &Utf8Path) -> Option<TsConfig> {
    let mut visited = HashSet::new();
    parse_tsconfig_recursive(repo_root, rel_path, &mut visited)
}

fn parse_tsconfig_recursive(
    repo_root: &Utf8Path,
    rel_path: &Utf8Path,
    visited: &mut HashSet<String>,
) -> Option<TsConfig> {
    let normalized = normalize_relative_path(rel_path.to_owned());
    if !visited.insert(normalized.to_string()) {
        return None;
    }

    let contents = fs::read_to_string(repo_root.join(&normalized).as_std_path()).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&contents).ok()?;
    let config_dir = normalized.parent().unwrap_or_else(|| Utf8Path::new(""));

    let mut config = value
        .get("extends")
        .and_then(|extends| extends.as_str())
        .and_then(|extends| resolve_tsconfig_extends_path(repo_root, &normalized, extends))
        .and_then(|parent_path| parse_tsconfig_recursive(repo_root, &parent_path, visited))
        .unwrap_or_else(|| TsConfig {
            base_url: config_dir.to_owned(),
            base_url_explicit: false,
            paths: Vec::new(),
        });

    let options = value
        .get("compilerOptions")
        .and_then(|compiler_options| compiler_options.as_object());

    if let Some(base_url) = options
        .and_then(|compiler_options| compiler_options.get("baseUrl"))
        .and_then(|base_url| base_url.as_str())
    {
        config.base_url = normalize_relative_path(config_dir.join(base_url));
        config.base_url_explicit = true;
    }

    if let Some(paths_obj) = options
        .and_then(|compiler_options| compiler_options.get("paths"))
        .and_then(|paths| paths.as_object())
    {
        let mut paths = Vec::new();
        for (pattern, replacements) in paths_obj {
            let Some(items) = replacements.as_array() else {
                continue;
            };
            let targets: Vec<String> = items
                .iter()
                .filter_map(|item| item.as_str())
                .map(|item| normalize_relative_path(config.base_url.join(item)).to_string())
                .collect();
            if !targets.is_empty() {
                paths.push(TsPathAlias {
                    pattern: pattern.clone(),
                    targets,
                });
            }
        }
        if !paths.is_empty() {
            config.paths = paths;
        }
    }

    (config.base_url_explicit || !config.paths.is_empty()).then_some(config)
}

fn resolve_tsconfig_extends_path(
    repo_root: &Utf8Path,
    current_path: &Utf8Path,
    extends: &str,
) -> Option<Utf8PathBuf> {
    if extends.is_empty() {
        return None;
    }

    if extends.starts_with('.') || extends.starts_with('/') {
        let current_dir = current_path.parent().unwrap_or_else(|| Utf8Path::new(""));
        let mut candidate = normalize_relative_path(current_dir.join(extends));
        if candidate.extension().is_none() {
            candidate.set_extension("json");
        }
        return Some(candidate);
    }

    let current_dir = current_path.parent().unwrap_or_else(|| Utf8Path::new(""));
    for ancestor in current_dir
        .ancestors()
        .chain(std::iter::once(Utf8Path::new("")))
    {
        let node_modules_dir = if ancestor.as_str().is_empty() {
            Utf8PathBuf::from("node_modules")
        } else {
            ancestor.join("node_modules")
        };
        for mut candidate in package_tsconfig_candidates(&node_modules_dir, extends) {
            candidate = normalize_relative_path(candidate);
            if repo_root.join(&candidate).exists() {
                return Some(candidate);
            }
        }
    }

    None
}

fn load_go_module(repo_root: &Utf8Path) -> Option<String> {
    let path = repo_root.join("go.mod");
    let contents = fs::read_to_string(path.as_std_path()).ok()?;
    contents.lines().find_map(|line| {
        line.trim()
            .strip_prefix("module ")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    })
}

fn call_meta(edge: &Edge) -> Option<CallMeta> {
    let extra = edge.extra_json.as_object()?;
    let callee_name = extra.get("callee_name")?.as_str()?.to_owned();
    let receiver_text = extra
        .get("receiver_text")
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .filter(|value| !value.is_empty());
    Some(CallMeta {
        callee_name,
        receiver_text,
    })
}

fn collect_import_bindings(nodes: &[Node]) -> Vec<ImportBinding> {
    let mut bindings = Vec::new();
    for node in nodes {
        if node.kind != NodeKind::Import {
            continue;
        }
        let Some(extra) = node.extra_json.as_object() else {
            continue;
        };
        let Some(source) = extra.get("source").and_then(|value| value.as_str()) else {
            continue;
        };
        let Some(items) = extra.get("bindings").and_then(|value| value.as_array()) else {
            continue;
        };
        for item in items {
            let Some(item_obj) = item.as_object() else {
                continue;
            };
            let Some(local) = item_obj.get("local").and_then(|value| value.as_str()) else {
                continue;
            };
            let imported = item_obj
                .get("imported")
                .and_then(|value| value.as_str())
                .map(str::to_owned);
            let kind = item_obj
                .get("kind")
                .and_then(|value| value.as_str())
                .unwrap_or("module")
                .to_owned();
            let relative_level = extra
                .get("relative_level")
                .and_then(|value| value.as_u64())
                .unwrap_or(0) as usize;
            bindings.push(ImportBinding {
                source: source.to_owned(),
                local: local.to_owned(),
                imported,
                kind,
                relative_level,
            });
        }
    }
    bindings
}

fn resolve_same_package_target(
    store: &Store,
    path: &str,
    language: &str,
    callee_name: &str,
    candidate_cache: &mut HashMap<(String, String), Vec<Node>>,
    owner_cache: &mut HashMap<String, Option<String>>,
) -> Option<(String, &'static str, f32)> {
    let candidates = callable_candidates(store, language, callee_name, candidate_cache).ok()?;
    let current_owner = cached_owner_id(store, path, owner_cache);

    if let Some(current_owner) = current_owner {
        let same_owner_matches: Vec<Node> = candidates
            .iter()
            .filter(|node| node.file_path != path)
            .filter(|node| {
                cached_owner_id(store, &node.file_path, owner_cache).as_deref()
                    == Some(current_owner.as_str())
            })
            .cloned()
            .collect();
        if let Some(node) = unique_node(same_owner_matches.clone()) {
            return Some((node.qualified_name, "same_package", 0.65));
        }

        let current_dir = Utf8Path::new(path)
            .parent()
            .unwrap_or_else(|| Utf8Path::new(""));
        let same_dir_matches: Vec<Node> = same_owner_matches
            .into_iter()
            .filter(|node| same_dir(current_dir, &node.file_path))
            .collect();
        return unique_node(same_dir_matches)
            .map(|node| (node.qualified_name, "same_package", 0.65));
    }

    let current_dir = Utf8Path::new(path)
        .parent()
        .unwrap_or_else(|| Utf8Path::new(""));
    let matches: Vec<Node> = candidates
        .into_iter()
        .filter(|node| node.file_path != path)
        .filter(|node| same_dir(current_dir, &node.file_path))
        .collect();
    unique_node(matches).map(|node| (node.qualified_name, "same_package", 0.65))
}

fn cached_owner_id(
    store: &Store,
    path: &str,
    owner_cache: &mut HashMap<String, Option<String>>,
) -> Option<String> {
    if let Some(owner_id) = owner_cache.get(path) {
        return owner_id.clone();
    }
    let owner_id = store.file_owner_id(path).ok().flatten();
    owner_cache.insert(path.to_owned(), owner_id.clone());
    owner_id
}

fn resolve_import_target(
    ctx: &ResolutionContext<'_>,
    meta: &CallMeta,
    bindings: &[ImportBinding],
    candidate_cache: &mut HashMap<(String, String), Vec<Node>>,
) -> Option<(String, &'static str, f32)> {
    let matching_bindings: Vec<&ImportBinding> = if let Some(receiver) = &meta.receiver_text {
        bindings
            .iter()
            .filter(|binding| binding.local == *receiver)
            .collect()
    } else {
        bindings
            .iter()
            .filter(|binding| binding.local == meta.callee_name || binding.kind == "wildcard")
            .collect()
    };

    for binding in matching_bindings {
        let imported_name = imported_symbol_name(binding, meta)?;
        let resolution_options = import_resolution_options(ctx, binding, imported_name)?;
        for (candidate_files, candidate_name) in resolution_options {
            let candidates = callable_candidates(
                ctx.store,
                ctx.language,
                candidate_name.as_str(),
                candidate_cache,
            )
            .ok()?;
            let matches: Vec<Node> = candidates
                .into_iter()
                .filter(|node| candidate_files.contains(node.file_path.as_str()))
                .collect();
            if let Some(node) = unique_node(matches) {
                return Some((node.qualified_name, "imports", 0.75));
            }
        }
    }

    None
}

fn import_resolution_options(
    ctx: &ResolutionContext<'_>,
    binding: &ImportBinding,
    imported_name: &str,
) -> Option<Vec<(HashSet<String>, String)>> {
    let candidate_files =
        resolve_import_files(ctx.config, ctx.repo_root, ctx.path, ctx.language, binding)?;
    let mut options = vec![(candidate_files.clone(), imported_name.to_owned())];

    if matches!(ctx.language, "javascript" | "typescript") {
        let mut visited = HashSet::new();
        options.extend(follow_js_ts_reexports(
            ctx.config,
            ctx.repo_root,
            ctx.language,
            &candidate_files,
            imported_name,
            0,
            &mut visited,
        ));
    }

    Some(options)
}

fn imported_symbol_name<'a>(binding: &'a ImportBinding, meta: &'a CallMeta) -> Option<&'a str> {
    if meta.receiver_text.is_some() {
        return Some(meta.callee_name.as_str());
    }
    match binding.kind.as_str() {
        "wildcard" => Some(meta.callee_name.as_str()),
        "named" | "from" => binding.imported.as_deref().or(Some(binding.local.as_str())),
        "module" => None,
        "namespace" => None,
        "package" => None,
        "default" => None,
        _ => binding.imported.as_deref().or(Some(binding.local.as_str())),
    }
}

fn resolve_import_files(
    config: &ResolverConfig,
    repo_root: &Utf8Path,
    path: &str,
    language: &str,
    binding: &ImportBinding,
) -> Option<HashSet<String>> {
    match language {
        "javascript" | "typescript" => {
            resolve_js_ts_import_files(config, repo_root, path, language, &binding.source)
        }
        "python" => resolve_python_import_files(repo_root, path, binding),
        "go" => resolve_go_import_files(config, repo_root, binding),
        _ => None,
    }
}

fn resolve_js_ts_import_files(
    config: &ResolverConfig,
    repo_root: &Utf8Path,
    path: &str,
    language: &str,
    source: &str,
) -> Option<HashSet<String>> {
    if !source.starts_with('.') {
        return resolve_tsconfig_import_files(
            find_nearest_tsconfig(&config.tsconfigs, path)?,
            repo_root,
            source,
            language,
        );
    }
    let current_dir = Utf8Path::new(path).parent()?;
    let base = current_dir.join(source);
    existing_repo_paths(repo_root, js_ts_candidates_for_base(&base, language))
}

fn resolve_python_import_files(
    repo_root: &Utf8Path,
    path: &str,
    binding: &ImportBinding,
) -> Option<HashSet<String>> {
    let current_dir = Utf8Path::new(path)
        .parent()
        .unwrap_or_else(|| Utf8Path::new(""));
    let base_dir = if binding.relative_level == 0 {
        Utf8PathBuf::new()
    } else {
        let mut base = current_dir.to_owned();
        for _ in 1..binding.relative_level {
            base.pop();
        }
        base
    };

    let candidates =
        python_import_candidates(&base_dir, &binding.source, binding.imported.as_deref());
    existing_repo_paths(repo_root, candidates)
}

fn resolve_go_import_files(
    config: &ResolverConfig,
    repo_root: &Utf8Path,
    binding: &ImportBinding,
) -> Option<HashSet<String>> {
    let module_path = config.go_module.as_deref()?;
    let suffix = if binding.source == module_path {
        ""
    } else {
        binding.source.strip_prefix(&format!("{module_path}/"))?
    };

    let dir = if suffix.is_empty() {
        Utf8PathBuf::new()
    } else {
        Utf8PathBuf::from(suffix)
    };
    let abs_dir = repo_root.join(&dir);
    if !abs_dir.is_dir() {
        return None;
    }

    let mut files = HashSet::new();
    for entry in fs::read_dir(abs_dir.as_std_path()).ok()? {
        let entry = entry.ok()?;
        let candidate = dir.join(entry.file_name().to_string_lossy().as_ref());
        if candidate.extension() == Some("go") {
            files.insert(candidate.to_string());
        }
    }

    if files.is_empty() { None } else { Some(files) }
}

fn resolve_tsconfig_import_files(
    config: &TsConfig,
    repo_root: &Utf8Path,
    source: &str,
    language: &str,
) -> Option<HashSet<String>> {
    let mut candidates = Vec::new();
    let mut matched = false;

    for alias in &config.paths {
        let Some(wildcard) = match_alias_pattern(&alias.pattern, source) else {
            continue;
        };
        matched = true;
        for target in &alias.targets {
            let replaced = apply_alias_target(target, wildcard);
            candidates.extend(js_ts_candidates_for_base(
                Utf8Path::new(&replaced),
                language,
            ));
        }
    }

    if !matched && config.base_url_explicit {
        let base = join_with_base_url(&config.base_url, source);
        candidates.extend(js_ts_candidates_for_base(&base, language));
    }

    existing_repo_paths(repo_root, candidates)
}

fn find_nearest_tsconfig<'a>(entries: &'a [TsConfigEntry], path: &str) -> Option<&'a TsConfig> {
    let current_dir = Utf8Path::new(path)
        .parent()
        .unwrap_or_else(|| Utf8Path::new(""));
    entries
        .iter()
        .find(|entry| current_dir.starts_with(&entry.dir))
        .map(|entry| &entry.config)
}

fn js_ts_candidates_for_base(base: &Utf8Path, language: &str) -> Vec<Utf8PathBuf> {
    if base.extension().is_some() {
        return vec![base.to_owned()];
    }

    let mut candidates = Vec::new();
    for ext in candidate_extensions(language) {
        candidates.push(Utf8PathBuf::from(format!("{}.{}", base, ext)));
        candidates.push(base.join(format!("index.{ext}")));
    }
    candidates
}

fn candidate_extensions(language: &str) -> &'static [&'static str] {
    if language == "typescript" {
        &["ts", "tsx", "js", "jsx", "mjs", "cjs"]
    } else {
        &["js", "jsx", "mjs", "cjs", "ts", "tsx"]
    }
}

fn python_import_candidates(
    base_dir: &Utf8Path,
    source: &str,
    imported: Option<&str>,
) -> Vec<Utf8PathBuf> {
    let normalized_source = source.trim_start_matches('.');
    let mut candidates = Vec::new();
    if normalized_source.is_empty() {
        candidates.push(base_dir.join("__init__.py"));
        if let Some(name) = imported {
            let module_path = name.replace('.', "/");
            candidates.push(base_dir.join(format!("{module_path}.py")));
            candidates.push(base_dir.join(format!("{module_path}/__init__.py")));
        }
        return candidates;
    }

    let module_path = normalized_source.replace('.', "/");
    candidates.push(base_dir.join(format!("{module_path}.py")));
    candidates.push(base_dir.join(format!("{module_path}/__init__.py")));
    candidates.push(base_dir.join(format!("{module_path}/__init__.py")));
    if let Some(imported_name) = imported
        && !matches!(imported_name, "" | "*")
    {
        let imported_module_path = imported_name.replace('.', "/");
        candidates.push(base_dir.join(format!("{module_path}/__init__.py")));
        candidates.push(base_dir.join(format!("{module_path}/{imported_module_path}.py")));
        candidates.push(base_dir.join(format!("{module_path}/{imported_module_path}/__init__.py")));
    }
    candidates
}

fn match_alias_pattern<'a>(pattern: &'a str, source: &'a str) -> Option<&'a str> {
    if let Some((prefix, suffix)) = pattern.split_once('*') {
        if source.starts_with(prefix)
            && source.ends_with(suffix)
            && source.len() >= prefix.len() + suffix.len()
        {
            return Some(&source[prefix.len()..source.len() - suffix.len()]);
        }
        return None;
    }

    (pattern == source).then_some("")
}

fn apply_alias_target(target: &str, wildcard: &str) -> String {
    if let Some((prefix, suffix)) = target.split_once('*') {
        return format!("{prefix}{wildcard}{suffix}");
    }
    target.to_owned()
}

fn join_with_base_url(base_url: &Utf8Path, path: &str) -> Utf8PathBuf {
    if base_url.as_str().is_empty() || base_url == Utf8Path::new(".") {
        Utf8PathBuf::from(path)
    } else {
        base_url.join(path)
    }
}

fn existing_repo_paths(
    repo_root: &Utf8Path,
    candidates: Vec<Utf8PathBuf>,
) -> Option<HashSet<String>> {
    let mut paths = HashSet::new();
    for candidate in candidates {
        let normalized = normalize_relative_path(candidate);
        if repo_root.join(&normalized).exists() {
            paths.insert(normalized.to_string());
        }
    }
    if paths.is_empty() { None } else { Some(paths) }
}

fn package_tsconfig_candidates(base_dir: &Utf8Path, extends: &str) -> Vec<Utf8PathBuf> {
    let mut candidates = Vec::new();
    let base = base_dir.join(extends);
    candidates.push(base.clone());
    if base.extension().is_none() {
        candidates.push(Utf8PathBuf::from(format!("{base}.json")));
    }
    candidates.push(base.join("tsconfig.json"));
    candidates
}

fn follow_js_ts_reexports(
    config: &ResolverConfig,
    repo_root: &Utf8Path,
    language: &str,
    files: &HashSet<String>,
    imported_name: &str,
    depth: usize,
    visited: &mut HashSet<(String, String)>,
) -> Vec<(HashSet<String>, String)> {
    if depth >= 4 {
        return Vec::new();
    }

    let mut results = Vec::new();
    for file in files {
        if !visited.insert((file.clone(), imported_name.to_owned())) {
            continue;
        }
        let abs_path = repo_root.join(file);
        let Ok(source) = fs::read_to_string(abs_path.as_std_path()) else {
            continue;
        };
        for (reexport_source, next_name) in js_ts_reexports_for_symbol(&source, imported_name) {
            let binding = ImportBinding {
                source: reexport_source,
                local: next_name.clone(),
                imported: Some(next_name.clone()),
                kind: "named".to_owned(),
                relative_level: 0,
            };
            let Some(next_files) =
                resolve_import_files(config, repo_root, file, language, &binding)
            else {
                continue;
            };
            results.push((next_files.clone(), next_name.clone()));
            results.extend(follow_js_ts_reexports(
                config,
                repo_root,
                language,
                &next_files,
                &next_name,
                depth + 1,
                visited,
            ));
        }
    }
    results
}

fn js_ts_reexports_for_symbol(source: &str, symbol: &str) -> Vec<(String, String)> {
    let mut results = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("export ") || !trimmed.contains(" from ") {
            continue;
        }
        let Some(module_source) = quoted_module_source(trimmed) else {
            continue;
        };

        if trimmed.starts_with("export * from ") {
            results.push((module_source.to_owned(), symbol.to_owned()));
            continue;
        }

        let Some(open) = trimmed.find('{') else {
            continue;
        };
        let Some(close) = trimmed[open + 1..].find('}') else {
            continue;
        };
        let clause = &trimmed[open + 1..open + 1 + close];
        for spec in clause
            .split(',')
            .map(str::trim)
            .filter(|spec| !spec.is_empty())
        {
            let (local, exported) = if let Some((left, right)) = spec.split_once(" as ") {
                (left.trim(), right.trim())
            } else {
                (spec, spec)
            };
            if exported == symbol {
                results.push((module_source.to_owned(), local.to_owned()));
            }
        }
    }
    results
}

fn quoted_module_source(input: &str) -> Option<&str> {
    let start = input.find(['"', '\''])?;
    let quote = input.as_bytes()[start] as char;
    let rest = &input[start + 1..];
    let end = rest.find(quote)?;
    Some(&rest[..end])
}

fn callable_candidates(
    store: &Store,
    language: &str,
    callee_name: &str,
    candidate_cache: &mut HashMap<(String, String), Vec<Node>>,
) -> Result<Vec<Node>> {
    let key = (language.to_owned(), callee_name.to_owned());
    if let Some(nodes) = candidate_cache.get(&key) {
        return Ok(nodes.clone());
    }
    let nodes = store.callable_nodes_by_name(language, callee_name)?;
    candidate_cache.insert(key, nodes.clone());
    Ok(nodes)
}

fn same_dir(current_dir: &Utf8Path, candidate_path: &str) -> bool {
    Utf8Path::new(candidate_path)
        .parent()
        .map(|parent| parent == current_dir)
        .unwrap_or(false)
}

fn unique_node(mut nodes: Vec<Node>) -> Option<Node> {
    if nodes.len() != 1 {
        return None;
    }
    nodes.pop()
}

fn normalize_relative_path(path: Utf8PathBuf) -> Utf8PathBuf {
    let mut normalized = Utf8PathBuf::new();
    for component in path.components() {
        match component.as_str() {
            "." => {}
            ".." => {
                normalized.pop();
            }
            part => normalized.push(part),
        }
    }
    normalized
}
