use super::*;

pub fn run_review_context(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let mut adapter = CliAdapter::open(&repo);
    if let Some(ref mut a) = adapter {
        a.before_command("review-context");
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
                    "review_context",
                    cli,
                )?;
                return Err(e).with_context(|| format!("cannot open database at {db_path}"));
            }
        };

        let (
            base,
            explicit_files,
            max_depth,
            max_nodes,
            format,
            allow_stale,
            allow_partial,
            include_overviews,
            include_stale_overviews,
            overview_limit,
            overview_max_bytes,
            exact_dir,
            exclude_subpaths,
            telemetry,
            include_ranking_evidence,
        ) = match &cli.command {
            Command::ReviewContext {
                base,
                files,
                max_depth,
                max_nodes,
                format,
                allow_stale,
                allow_partial,
                include_overviews,
                include_stale_overviews,
                overview_limit,
                overview_max_bytes,
                exact_dir,
                exclude_subpaths,
                telemetry,
                include_ranking_evidence,
            } => (
                base.clone(),
                files.clone(),
                *max_depth,
                *max_nodes as usize,
                *format,
                *allow_stale,
                *allow_partial,
                *include_overviews,
                *include_stale_overviews,
                *overview_limit,
                *overview_max_bytes,
                exact_dir.clone(),
                exclude_subpaths.clone(),
                *telemetry,
                *include_ranking_evidence,
            ),
            _ => unreachable!(),
        };

        let readiness = derive_graph_readiness(&store, &repo, &db_path);
        if let Some(warning) = check_graph_readiness(
            &readiness,
            GraphToolRequirement::Analysis,
            readiness_overrides(allow_stale, allow_partial),
            "review_context",
            cli,
        )? {
            eprintln!("Warning: {warning}");
        }

        if cli.json && format == ReviewContextFormat::Markdown {
            anyhow::bail!("--format markdown cannot be combined with --json");
        }

        let target_files: Vec<String> = if !explicit_files.is_empty() {
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

        if target_files.is_empty() {
            if cli.json {
                // Emit the same `context_result` envelope as the non-empty path
                // (D10). Previously this branch returned `{files, review_context}`
                // while `--files` returned the context engine payload, forcing
                // consumers to branch on shape.
                print_json(
                    "review_context",
                    serde_json::json!({
                        "request": {
                            "intent": "review",
                            "target": { "kind": "changed_files", "paths": [] },
                            "max_nodes": max_nodes,
                            "depth": max_depth,
                        },
                        "nodes": [],
                        "edges": [],
                        "files": [],
                        "truncation": {},
                        "ambiguity": serde_json::Value::Null,
                        "workflow": serde_json::Value::Null,
                        "saved_context_sources": [],
                        "budget_status": "ok",
                        "budget_hit": false,
                        "budget_name": serde_json::Value::Null,
                        "budget_limit": serde_json::Value::Null,
                        "budget_observed": serde_json::Value::Null,
                        "partial": false,
                        "safe_to_answer": true,
                    }),
                )?;
            } else if format == ReviewContextFormat::Markdown {
                println!("## Atlas Review Context\n\nNo changed files detected.");
            } else {
                println!("No changed files detected.");
            }
            return Ok(());
        }

        let workflow_request = ContextRequest {
            intent: ContextIntent::Review,
            target: ContextTarget::ChangedFiles {
                paths: target_files.clone(),
            },
            max_nodes: Some(max_nodes),
            depth: Some(max_depth),
            exact_dir: exact_dir
                .as_deref()
                .map(|path| {
                    atlas_repo::CanonicalRepoPath::from_repo_relative(path)
                        .map(|resolved| resolved.as_str().to_owned())
                        .with_context(|| format!("invalid directory scope '{path}'"))
                })
                .transpose()?,
            exclude_subpaths: exclude_subpaths
                .iter()
                .map(|path| {
                    atlas_repo::CanonicalRepoPath::from_repo_relative(path)
                        .map(|resolved| resolved.as_str().to_owned())
                        .with_context(|| format!("invalid directory scope '{path}'"))
                })
                .collect::<Result<Vec<_>>>()?,
            include_overviews,
            include_telemetry: telemetry,
            include_ranking_evidence,
            overview_limit,
            overview_max_bytes,
            overview_freshness: include_stale_overviews.then(|| {
                vec![
                    "fresh".to_owned(),
                    "possibly_stale".to_owned(),
                    "stale".to_owned(),
                ]
            }),
            ..ContextRequest::default()
        };
        let token_counter = load_token_counter(repo_root.as_str())?;
        let overview_session = if include_overviews {
            let session_db = atlas_engine::paths::session_db_path(&db_path);
            Some(
                atlas_session::SessionStore::open(&session_db)
                    .with_context(|| format!("cannot open session store at {session_db}"))?,
            )
        } else {
            None
        };
        let escalation = {
            let config =
                atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root.as_str()))
                    .unwrap_or_default();
            (
                config.context.escalation.enabled,
                config.context.escalation.max_rounds,
            )
        };
        let mut engine = ContextEngine::new(&store)
            .with_budget_policy(load_budget_policy(repo_root.as_str())?)
            .with_token_counter(token_counter.counter)
            .with_token_fallback(token_counter.fallback_used, token_counter.fallback_reason)
            .with_escalation(escalation.0, escalation.1);
        if let Some(session) = overview_session.as_ref() {
            engine = engine.with_overview_store(repo_root.as_str(), session);
        }
        let workflow_result = engine
            .build(&workflow_request)
            .context("context engine failed")?;

        if cli.json {
            let mut value = serde_json::to_value(&workflow_result)?;
            if let Some(object) = value.as_object_mut()
                && include_ranking_evidence
            {
                object.insert(
                    "context_ranking_evidence_legend".to_owned(),
                    atlas_core::context_ranking_evidence_legend(),
                );
            }
            print_json("review_context", value)?;
            return Ok(());
        }

        match format {
            ReviewContextFormat::Text => print_review_context_text(&workflow_result, &target_files),
            ReviewContextFormat::Markdown => {
                print!(
                    "{}",
                    render_review_context_markdown(&workflow_result, &target_files)
                );
            }
        }

        Ok(())
    })();

    if let Some(ref mut a) = adapter {
        a.after_command("review-context", result.is_ok());
    }
    result
}
