use super::*;

pub fn run_detect_changes(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let repo_root_path =
        find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
    let repo_root = repo_root_path.as_path();
    let db_path = db_path(cli, &repo);

    let (base, staged, selected_repo_id, all_repos) = match &cli.command {
        Command::DetectChanges {
            base,
            staged,
            repo_id,
            all_repos,
        } => (base.clone(), *staged, repo_id.clone(), *all_repos),
        _ => unreachable!(),
    };
    let diff_target = detect_changes_target(&base, staged);

    if all_repos || selected_repo_id.is_some() {
        let (registrations, excluded_manual_repo_count) =
            selected_impact_repos(Utf8Path::new(&repo), &selected_repo_id, all_repos)?;
        let mut repo_changes = Vec::new();
        let mut warnings = Vec::new();
        let mut processed_repo_count = 0usize;
        let mut failed_repo_count = 0usize;
        let mut skipped_repo_count = 0usize;
        let mut changed_file_count = 0usize;
        for registration in registrations {
            if !registration.root.exists() {
                failed_repo_count += 1;
                skipped_repo_count += 1;
                warnings.push(format!(
                    "skipped repo {}: repo root missing",
                    registration.display_alias
                ));
                repo_changes.push(serde_json::json!({
                    "repo_id": registration.repo_id,
                    "display_alias": registration.display_alias,
                    "root": registration.root,
                    "status": "skipped",
                    "error": "repo root missing",
                    "changes": [],
                }));
                continue;
            }
            match changed_files(registration.root.as_path(), &diff_target) {
                Ok(changes) => {
                    processed_repo_count += 1;
                    changed_file_count += changes.len();
                    repo_changes.push(serde_json::json!({
                        "repo_id": registration.repo_id,
                        "display_alias": registration.display_alias,
                        "root": registration.root,
                        "status": "ok",
                        "changed_file_count": changes.len(),
                        "changes": augment_changes_with_node_counts(&changes, None),
                    }));
                }
                Err(error) => {
                    failed_repo_count += 1;
                    skipped_repo_count += 1;
                    warnings.push(format!(
                        "skipped repo {}: {}",
                        registration.display_alias, error
                    ));
                    repo_changes.push(serde_json::json!({
                        "repo_id": registration.repo_id,
                        "display_alias": registration.display_alias,
                        "root": registration.root,
                        "status": "skipped",
                        "error": error.to_string(),
                        "changes": [],
                    }));
                }
            }
        }
        if cli.json {
            return print_json(
                "detect_changes",
                serde_json::json!({
                    "diff_target": {
                        "base": base,
                        "staged": staged,
                        "kind": if staged { "staged" } else if base.is_some() { "base_ref" } else { "working_tree" },
                    },
                    "repo_scope": {
                        "selected_repo_count": repo_changes.len(),
                        "processed_repo_count": processed_repo_count,
                        "failed_repo_count": failed_repo_count,
                        "skipped_repo_count": skipped_repo_count,
                        "excluded_manual_repo_count": excluded_manual_repo_count,
                    },
                    "summary": {
                        "changed_file_count": changed_file_count,
                        "warning_count": warnings.len(),
                    },
                    "warnings": warnings,
                    "repos": repo_changes,
                }),
            );
        }
        println!(
            "Detected changes across {} repo(s); processed={} failures={} skipped={} changed_files={}",
            repo_changes.len(),
            processed_repo_count,
            failed_repo_count,
            skipped_repo_count,
            changed_file_count,
        );
        if excluded_manual_repo_count > 0 {
            println!(
                "Phase-1 rollout: excluded {} manual repo(s) from --all-repos. Use --repo-id to target them explicitly.",
                excluded_manual_repo_count
            );
        }
        for warning in &warnings {
            println!("warning\t{warning}");
        }
        for repo_change in &repo_changes {
            println!("{}", repo_change);
        }
        return Ok(());
    }

    let changes = changed_files(repo_root, &diff_target).context("cannot detect changed files")?;

    // Try to open the DB for graph summary — tolerate failure (DB may not exist yet).
    let store_result = Store::open(&db_path);

    if cli.json {
        let entries = augment_changes_with_node_counts(&changes, store_result.as_ref().ok());
        print_json(
            "detect_changes",
            serde_json::json!({
                "diff_target": {
                    "base": base,
                    "staged": staged,
                    "kind": if staged { "staged" } else if base.is_some() { "base_ref" } else { "working_tree" },
                },
                // `changes` is the historical CLI key; `files` matches MCP.
                // Emit both so consumers never branch on surface.
                "files": entries.clone(),
                "changes": entries,
            }),
        )?;
    } else if changes.is_empty() {
        println!("No changed files detected.");
    } else {
        for cf in &changes {
            let node_info = store_result
                .as_ref()
                .ok()
                .and_then(|s| s.nodes_by_file(&cf.path).ok())
                .map(|ns| format!(" [{} nodes]", ns.len()))
                .unwrap_or_default();
            if let Some(old) = &cf.old_path {
                println!(
                    "{}  {old} -> {}{node_info}",
                    change_tag(cf.change_type),
                    cf.path
                );
            } else {
                println!("{}  {}{node_info}", change_tag(cf.change_type), cf.path);
            }
        }
        println!("\n{} file(s) changed.", changes.len());

        // Graph-level impact summary when DB is available.
        if let Ok(store) = &store_result {
            let policy = load_budget_policy(&repo)?;
            let non_deleted: Vec<&str> = changes
                .iter()
                .filter(|cf| cf.change_type != ChangeType::Deleted)
                .map(|cf| cf.path.as_str())
                .collect();
            if !non_deleted.is_empty()
                && let Ok(impact) = store.impact_radius(
                    &non_deleted,
                    5,
                    200,
                    policy.graph_traversal.edges.default_limit,
                )
            {
                println!("\nGraph impact summary:");
                println!("  Changed symbols : {}", impact.changed_nodes.len());
                println!("  Impacted nodes  : {}", impact.impacted_nodes.len());
                println!("  Impacted files  : {}", impact.impacted_files.len());
            }
        }
    }

    Ok(())
}
