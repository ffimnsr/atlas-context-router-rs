use super::*;

pub fn run_explain_change(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let mut adapter = CliAdapter::open(&repo);
    if let Some(ref mut a) = adapter {
        a.before_command("explain-change");
    }

    let result = (|| -> Result<()> {
        let repo_root_path =
            find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
        let repo_root = repo_root_path.as_path();
        let db_path = db_path(cli, &repo);

        let (base, staged, explicit_files, max_depth, max_nodes, allow_stale, allow_partial) =
            match &cli.command {
                Command::ExplainChange {
                    base,
                    staged,
                    files,
                    max_depth,
                    max_nodes,
                    allow_stale,
                    allow_partial,
                } => (
                    base.clone(),
                    *staged,
                    files.clone(),
                    *max_depth,
                    *max_nodes as usize,
                    *allow_stale,
                    *allow_partial,
                ),
                _ => unreachable!(),
            };

        let changes = if !explicit_files.is_empty() {
            normalize_explicit_files(repo_root, &explicit_files)?
                .into_iter()
                .map(|path| atlas_core::model::ChangedFile {
                    path,
                    change_type: ChangeType::Modified,
                    old_path: None,
                })
                .collect()
        } else {
            changed_files(repo_root, &detect_changes_target(&base, staged))
                .context("cannot detect changed files")?
        };

        let target_files: Vec<String> = changes
            .iter()
            .filter(|change| change.change_type != ChangeType::Deleted)
            .map(|change| change.path.clone())
            .collect();

        if target_files.is_empty() {
            let empty = empty_explain_change_summary();
            if cli.json {
                print_json("explain_change", explain_change_wire_payload(&empty))?;
            } else {
                println!("No changed files detected.");
            }
            return Ok(());
        }

        let store = match Store::open(&db_path) {
            Ok(s) => s,
            Err(e) => {
                let readiness = derive_graph_readiness_open_failed(&repo, &db_path, &e.to_string());
                check_graph_readiness(
                    &readiness,
                    GraphToolRequirement::Analysis,
                    readiness_overrides(allow_stale, allow_partial),
                    "explain_change",
                    cli,
                )?;
                return Err(e).with_context(|| format!("cannot open database at {db_path}"));
            }
        };

        let readiness = derive_graph_readiness(&store, &repo, &db_path);
        if let Some(warning) = check_graph_readiness(
            &readiness,
            GraphToolRequirement::Analysis,
            readiness_overrides(allow_stale, allow_partial),
            "explain_change",
            cli,
        )? {
            eprintln!("Warning: {warning}");
        }

        let policy = load_budget_policy(&repo)?;
        let summary = build_explain_change_summary(
            &store,
            &changes,
            &target_files,
            max_depth,
            max_nodes,
            &policy,
        )?;

        if cli.json {
            print_json("explain_change", explain_change_wire_payload(&summary))?;
        } else {
            println!("Risk level      : {}", summary.risk_level);
            println!("Changed files   : {}", summary.changed_file_count);
            println!("Changed symbols : {}", summary.changed_symbol_count);
            println!(
                "Diff summary    : +{} ~{} -{} r{}",
                summary.diff_summary.counts.added,
                summary.diff_summary.counts.modified,
                summary.diff_summary.counts.deleted,
                summary.diff_summary.counts.renamed
            );
            println!(
                "Change kinds    : api {} | signature {} | internal {}",
                summary.changed_by_kind.api_change,
                summary.changed_by_kind.signature_change,
                summary.changed_by_kind.internal_change
            );
            println!("Impacted files  : {}", summary.impacted_file_count);
            println!("Impacted nodes  : {}", summary.impacted_node_count);

            if !summary.changed_symbols.is_empty() {
                println!("\nChanged symbols:");
                for symbol in summary.changed_symbols.iter().take(20) {
                    println!(
                        "  [{}] {} {} ({}:{})",
                        symbol.change_kind, symbol.kind, symbol.qn, symbol.file, symbol.line
                    );
                }
            }

            if !summary.boundary_violations.is_empty() {
                println!("\nBoundary violations:");
                for violation in &summary.boundary_violations {
                    println!("  [{}] {}", violation.kind, violation.description);
                }
            }

            if !summary.impacted_components.is_empty() {
                println!("\nImpacted components:");
                for component in summary.impacted_components.iter().take(8) {
                    println!(
                        "  [{}] {} | changed {} | impacted {} | files {}",
                        component.kind,
                        component.label,
                        component.changed_node_count,
                        component.impacted_node_count,
                        component.file_count
                    );
                }
            }

            if !summary.call_chains.is_empty() {
                println!("\nCall chains:");
                for chain in summary.call_chains.iter().take(5) {
                    println!("  {}", chain.summary);
                }
            }

            if !summary.ripple_effects.is_empty() {
                println!("\nRipple effects:");
                for ripple in &summary.ripple_effects {
                    println!("  {ripple}");
                }
            }

            if summary.test_impact.affected_test_count > 0 {
                println!(
                    "\nAffected tests  : {}",
                    summary.test_impact.affected_test_count
                );
            }
            if summary.test_impact.uncovered_symbol_count > 0 {
                println!("Changed symbols without test coverage:");
                for symbol in &summary.test_impact.uncovered_symbols {
                    println!("  {symbol}");
                }
            }

            println!("\nSummary: {}", summary.summary);
        }

        Ok(())
    })();

    if let Some(ref mut a) = adapter {
        a.after_command("explain-change", result.is_ok());
    }
    result
}
