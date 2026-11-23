use super::*;

pub fn run_impact(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let mut adapter = CliAdapter::open(&repo);
    if let Some(ref mut a) = adapter {
        a.before_command("impact");
    }

    let result = (|| -> Result<()> {
        let repo_root_path =
            find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
        let repo_root = repo_root_path.as_path();
        let db_path = db_path(cli, &repo);

        let store = match Store::open(&db_path) {
            Ok(s) => s,
            Err(e) => {
                let readiness = derive_graph_readiness_open_failed(&repo, &db_path, &e.to_string());
                check_graph_readiness(
                    &readiness,
                    GraphToolRequirement::Analysis,
                    readiness_overrides(false, false),
                    "impact",
                    cli,
                )?;
                return Err(e).with_context(|| format!("cannot open database at {db_path}"));
            }
        };
        let policy = load_budget_policy(&repo)?;

        let (
            base,
            explicit_files,
            max_depth,
            max_nodes,
            allow_stale,
            allow_partial,
            repo_id,
            all_repos,
        ) = match &cli.command {
            Command::Impact {
                base,
                files,
                max_depth,
                max_nodes,
                allow_stale,
                allow_partial,
                repo_id,
                all_repos,
            } => (
                base.clone(),
                files.clone(),
                *max_depth,
                *max_nodes as usize,
                *allow_stale,
                *allow_partial,
                repo_id.clone(),
                *all_repos,
            ),
            _ => unreachable!(),
        };

        let readiness = derive_graph_readiness(&store, &repo, &db_path);
        if let Some(warning) = check_graph_readiness(
            &readiness,
            GraphToolRequirement::Analysis,
            readiness_overrides(allow_stale, allow_partial),
            "impact",
            cli,
        )? {
            eprintln!("Warning: {warning}");
        }

        let (selected_repos, excluded_manual_repo_count) =
            selected_impact_repos(repo_root, &repo_id, all_repos)?;
        let mut repo_warnings = Vec::new();
        let mut processed_repo_count = 0usize;
        let mut failed_repo_count = 0usize;
        let mut skipped_repo_count = 0usize;
        let target_files: Vec<(String, String)> = if selected_repos.is_empty() {
            let files = if !explicit_files.is_empty() {
                normalize_explicit_files(repo_root, &explicit_files)?
            } else {
                let diff_target = if let Some(base_ref) = &base {
                    DiffTarget::BaseRef(base_ref.clone())
                } else {
                    DiffTarget::WorkingTree
                };
                changed_files(repo_root, &diff_target)
                    .context("cannot detect changed files")?
                    .into_iter()
                    .filter(|cf| cf.change_type != ChangeType::Deleted)
                    .map(|cf| cf.path)
                    .collect()
            };
            let root_repo_id = stable_repo_id(repo_root);
            files
                .into_iter()
                .map(|path| (root_repo_id.clone(), path))
                .collect()
        } else {
            let diff_target = if let Some(base_ref) = &base {
                DiffTarget::BaseRef(base_ref.clone())
            } else {
                DiffTarget::WorkingTree
            };
            let mut combined = Vec::new();
            for registration in &selected_repos {
                if !registration.root.exists() {
                    failed_repo_count += 1;
                    skipped_repo_count += 1;
                    repo_warnings.push(format!(
                        "skipped repo {}: repo root missing",
                        registration.display_alias
                    ));
                    continue;
                }
                let repo_files = if !explicit_files.is_empty() {
                    match normalize_explicit_files(registration.root.as_path(), &explicit_files) {
                        Ok(files) => files,
                        Err(error) => {
                            failed_repo_count += 1;
                            skipped_repo_count += 1;
                            repo_warnings.push(format!(
                                "skipped repo {}: {}",
                                registration.display_alias, error
                            ));
                            continue;
                        }
                    }
                } else {
                    match changed_files(registration.root.as_path(), &diff_target) {
                        Ok(changes) => changes
                            .into_iter()
                            .filter(|cf| cf.change_type != ChangeType::Deleted)
                            .map(|cf| cf.path)
                            .collect(),
                        Err(error) => {
                            failed_repo_count += 1;
                            skipped_repo_count += 1;
                            repo_warnings.push(format!(
                                "skipped repo {}: {}",
                                registration.display_alias, error
                            ));
                            continue;
                        }
                    }
                };
                if repo_files.is_empty() {
                    skipped_repo_count += 1;
                    continue;
                }
                processed_repo_count += 1;
                combined.extend(
                    repo_files
                        .into_iter()
                        .map(|path| (registration.repo_id.clone(), path)),
                );
            }
            combined
        };

        if target_files.is_empty() {
            if cli.json {
                print_json(
                    "impact",
                    serde_json::json!({
                        // Same `files` element shape as the non-empty path so
                        // empty and populated payloads share one schema.
                        "files": target_files.iter().map(|(repo_id, path)| serde_json::json!({
                            "repo_id": repo_id,
                            "repo_alias": repo_id,
                            "path": path,
                        })).collect::<Vec<_>>(),
                        "repo_scope": {
                            "selected_repo_count": selected_repos.len(),
                            "processed_repo_count": processed_repo_count,
                            "failed_repo_count": failed_repo_count,
                            "skipped_repo_count": skipped_repo_count,
                            "excluded_manual_repo_count": excluded_manual_repo_count,
                        },
                        "warnings": repo_warnings,
                        "analysis": ImpactResult {
                            changed_nodes: vec![],
                            impacted_nodes: vec![],
                            impacted_files: vec![],
                            relevant_edges: vec![],
                            seed_budgets: vec![],
                            traversal_budget: None,
                            budget: BudgetReport::not_applicable(),
                        }
                    }),
                )?;
            } else {
                println!("No changed files detected.");
                if excluded_manual_repo_count > 0 {
                    println!(
                        "Phase-1 rollout: excluded {} manual repo(s) from --all-repos. Use --repo-id to target them explicitly.",
                        excluded_manual_repo_count
                    );
                }
                for warning in &repo_warnings {
                    println!("warning\t{warning}");
                }
            }
            return Ok(());
        }

        let mut seed_qnames = Vec::new();
        for (seed_repo_id, file_path) in &target_files {
            seed_qnames.extend(impact_seed_qnames_for_repo(
                &store,
                seed_repo_id,
                std::slice::from_ref(file_path),
            )?);
        }
        seed_qnames.sort();
        seed_qnames.dedup();

        let t0 = std::time::Instant::now();
        let result = store
            .traverse_from_qnames(
                &seed_qnames.iter().map(String::as_str).collect::<Vec<_>>(),
                max_depth,
                max_nodes,
                policy.graph_traversal.edges.default_limit,
            )
            .context("impact radius query failed")?;
        let latency_ms = t0.elapsed().as_millis();

        let advanced = advanced_impact(result);
        let repo_aliases: std::collections::BTreeMap<String, String> = selected_repos
            .iter()
            .map(|entry| (entry.repo_id.clone(), entry.display_alias.clone()))
            .collect();

        if cli.json {
            print_json(
                "impact",
                serde_json::json!({
                    "files": target_files.iter().map(|(repo_id, path)| serde_json::json!({
                        "repo_id": repo_id,
                        "repo_alias": repo_aliases.get(repo_id).cloned().unwrap_or_else(|| repo_id.clone()),
                        "path": path,
                    })).collect::<Vec<_>>(),
                    "repo_scope": {
                        "selected_repo_count": selected_repos.len(),
                        "processed_repo_count": processed_repo_count,
                        "failed_repo_count": failed_repo_count,
                        "skipped_repo_count": skipped_repo_count,
                        "excluded_manual_repo_count": excluded_manual_repo_count,
                    },
                    "warnings": repo_warnings,
                    "analysis": advanced,
                }),
            )?;
        } else {
            if !selected_repos.is_empty() {
                println!(
                    "Repo scope    : selected={} processed={} failures={} skipped={}",
                    selected_repos.len(),
                    processed_repo_count,
                    failed_repo_count,
                    skipped_repo_count,
                );
            }
            println!("Changed files : {}", target_files.len());
            println!("Changed nodes : {}", advanced.base.changed_nodes.len());
            println!("Impacted nodes: {}", advanced.base.impacted_nodes.len());
            println!("Impacted files: {}", advanced.base.impacted_files.len());
            println!("Relevant edges: {}", advanced.base.relevant_edges.len());
            println!("Risk level    : {}", advanced.risk_level);
            println!("Latency       : {latency_ms}ms");
            if !advanced.base.impacted_files.is_empty() {
                println!("\nImpacted files:");
                for f in &advanced.base.impacted_files {
                    println!("  {f}");
                }
            }
            if !advanced.scored_nodes.is_empty() {
                println!("\nTop impacted nodes (by score):");
                for sn in advanced.scored_nodes.iter().take(20) {
                    let ck = sn
                        .change_kind
                        .map(|c| format!(" [{c}]"))
                        .unwrap_or_default();
                    let repo_suffix = node_repo_id(&sn.node)
                        .map(|repo_id| {
                            let label = repo_aliases
                                .get(repo_id)
                                .cloned()
                                .unwrap_or_else(|| repo_id.to_owned());
                            format!(" [repo {label}]")
                        })
                        .unwrap_or_default();
                    println!(
                        "  {:>6.2}  {} {}{}{}",
                        sn.impact_score,
                        sn.node.kind.as_str(),
                        sn.node.qualified_name,
                        ck,
                        repo_suffix
                    );
                }
            }
            if !advanced.test_impact.affected_tests.is_empty() {
                println!(
                    "\nAffected tests: {}",
                    advanced.test_impact.affected_tests.len()
                );
            }
            if !advanced.test_impact.uncovered_changed_nodes.is_empty() {
                println!("\nChanged nodes with no test coverage:");
                for n in &advanced.test_impact.uncovered_changed_nodes {
                    println!("  {} {}", n.kind.as_str(), n.qualified_name);
                }
            }
            if !advanced.boundary_violations.is_empty() {
                println!("\nBoundary violations:");
                for v in &advanced.boundary_violations {
                    println!("  [{}] {}", v.kind, v.description);
                }
            }
            if excluded_manual_repo_count > 0 {
                println!(
                    "\nPhase-1 rollout: excluded {} manual repo(s) from --all-repos. Use --repo-id to target them explicitly.",
                    excluded_manual_repo_count
                );
            }
            for warning in &repo_warnings {
                println!("warning\t{warning}");
            }
        }

        Ok(())
    })();

    if let Some(ref mut a) = adapter {
        a.after_command("impact", result.is_ok());
    }
    result
}
