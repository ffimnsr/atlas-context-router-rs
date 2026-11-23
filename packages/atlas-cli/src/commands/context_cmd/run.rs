use super::*;

pub fn run_context(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let mut adapter = CliAdapter::open(&repo);
    if let Some(ref mut a) = adapter {
        a.before_command("context");
    }
    let db_path = db_path(cli, &repo);

    let result = (|| -> Result<()> {
        let (
            query,
            file,
            files,
            intent_override,
            max_nodes,
            max_edges,
            max_files,
            depth,
            code_spans,
            tests,
            imports,
            neighbors,
            semantic,
            agent_id,
            merge_agent_partitions,
            token_budget,
            include_overviews,
            include_stale_overviews,
            overview_limit,
            overview_max_bytes,
            overview_subject_kinds,
            exact_dir,
            exclude_subpaths,
            telemetry,
            include_ranking_evidence,
        ) = match &cli.command {
            Command::Context {
                query,
                file,
                files,
                intent,
                max_nodes,
                max_edges,
                max_files,
                depth,
                code_spans,
                tests,
                imports,
                neighbors,
                semantic,
                agent_id,
                merge_agent_partitions,
                token_budget,
                include_overviews,
                include_stale_overviews,
                overview_limit,
                overview_max_bytes,
                overview_subject_kinds,
                exact_dir,
                exclude_subpaths,
                telemetry,
                include_ranking_evidence,
            } => (
                query.clone(),
                file.clone(),
                files.clone(),
                intent.clone(),
                *max_nodes,
                *max_edges,
                *max_files,
                *depth,
                *code_spans,
                *tests,
                *imports,
                *neighbors,
                *semantic,
                agent_id.clone(),
                *merge_agent_partitions,
                *token_budget,
                *include_overviews,
                *include_stale_overviews,
                *overview_limit,
                *overview_max_bytes,
                overview_subject_kinds.clone(),
                exact_dir.clone(),
                exclude_subpaths.clone(),
                *telemetry,
                *include_ranking_evidence,
            ),
            _ => unreachable!(),
        };

        // Build the base request: parse from free-text query or structured flags.
        let mut request = if !files.is_empty() {
            // Explicit changed-file list → review/impact. Accept repo-relative,
            // repo-dir-prefixed, and absolute-under-root forms.
            let repo_root_path =
                find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
            let normalized_files = files
                .iter()
                .map(|path| {
                    atlas_repo::normalize_repo_file_path(repo_root_path.as_path(), path)
                        .map(|resolved| resolved.canonical)
                        .with_context(|| format!("invalid file path '{path}'"))
                })
                .collect::<Result<Vec<_>>>()?;
            let intent = parse_intent_override(intent_override.as_deref(), ContextIntent::Review);
            ContextRequest {
                intent,
                target: ContextTarget::ChangedFiles {
                    paths: normalized_files,
                },
                ..ContextRequest::default()
            }
        } else if let Some(path) = file {
            // Explicit file target.
            let repo_root_path =
                find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
            let path = atlas_repo::normalize_repo_file_path(repo_root_path.as_path(), &path)
                .with_context(|| format!("invalid file path '{path}'"))?
                .canonical;
            let intent = parse_intent_override(intent_override.as_deref(), ContextIntent::File);
            ContextRequest {
                intent,
                target: ContextTarget::FilePath { path },
                ..ContextRequest::default()
            }
        } else if let Some(q) = query {
            // Free-text or symbol/qualified name — route through query parser.
            let mut parsed = query_parser::parse_query(&q);
            if let Some(intent) = intent_override.as_deref().and_then(parse_intent_str) {
                parsed.intent = intent;
            }
            parsed
        } else {
            anyhow::bail!("provide a TARGET query, --file <path>, or --files <paths...>");
        };

        // Apply explicit limit/depth overrides.
        if max_nodes.is_some() {
            request.max_nodes = max_nodes;
        }
        if max_edges.is_some() {
            request.max_edges = max_edges;
        }
        if max_files.is_some() {
            request.max_files = max_files;
        }
        if depth.is_some() {
            request.depth = depth;
        }
        if code_spans {
            request.include_code_spans = true;
        }
        if tests {
            request.include_tests = true;
        }
        if imports {
            request.include_imports = true;
        }
        if neighbors {
            request.include_neighbors = true;
        }
        if token_budget.is_some() {
            request.token_budget = token_budget;
        }
        request.agent_id = agent_id.clone();
        request.merge_agent_partitions = merge_agent_partitions;
        request.include_overviews = include_overviews;
        request.include_telemetry = telemetry;
        request.include_ranking_evidence = include_ranking_evidence;
        request.overview_limit = overview_limit;
        request.overview_max_bytes = overview_max_bytes;
        if exact_dir.is_some() || !exclude_subpaths.is_empty() {
            let normalize = |path: &str| {
                atlas_repo::CanonicalRepoPath::from_repo_relative(path)
                    .map(|resolved| resolved.as_str().to_owned())
                    .with_context(|| format!("invalid directory scope '{path}'"))
            };
            request.exact_dir = exact_dir.as_deref().map(normalize).transpose()?;
            request.exclude_subpaths = exclude_subpaths
                .iter()
                .map(|path| normalize(path))
                .collect::<Result<Vec<_>>>()?;
        }
        if include_stale_overviews {
            request.overview_freshness = Some(vec![
                "fresh".to_owned(),
                "possibly_stale".to_owned(),
                "stale".to_owned(),
            ]);
        }
        if let Some(kinds) = overview_subject_kinds {
            request.overview_subject_kinds = Some(
                kinds
                    .split(',')
                    .map(|value| value.trim().to_owned())
                    .filter(|value| !value.is_empty())
                    .collect(),
            );
        }

        let store = match Store::open(&db_path) {
            Err(e) => {
                let readiness = derive_graph_readiness_open_failed(&repo, &db_path, &e.to_string());
                check_graph_readiness(
                    &readiness,
                    GraphToolRequirement::Analysis,
                    readiness_overrides(false, false),
                    "context",
                    cli,
                )?;
                return Err(e).with_context(|| format!("cannot open database at {db_path}"));
            }
            Ok(s) => s,
        };
        let readiness = derive_graph_readiness(&store, &repo, &db_path);
        if let Some(warning) = check_graph_readiness(
            &readiness,
            GraphToolRequirement::Analysis,
            readiness_overrides(false, false),
            "context",
            cli,
        )? {
            eprintln!("Warning: {warning}");
        }

        // --semantic: when the target is a free-text / symbol query, run a
        // graph-aware semantic search first to find the best matching qualified
        // name.  If a session is active, prior-context files from recent
        // events are used to boost relevance (context_boosted_search).
        // The context engine then builds around the resolved qualified name
        // instead of doing a fuzzier name lookup internally.
        if semantic && let ContextTarget::SymbolName { ref name } = request.target {
            let sq = SearchQuery {
                text: name.clone(),
                limit: 5,
                graph_expand: true,
                graph_max_hops: 1,
                ..Default::default()
            };
            let (ctx_files, ctx_symbols) = context_session_hints(&repo, "cli");
            let hits =
                search::semantic::context_boosted_search(&store, &sq, &ctx_files, &ctx_symbols)
                    .unwrap_or_default();
            if let Some(top) = hits.into_iter().next() {
                request.target = ContextTarget::QualifiedName {
                    qname: top.node.qualified_name,
                };
            }
        }

        let token_counter = load_token_counter(&repo)?;
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
            let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(&repo))
                .unwrap_or_default();
            (
                config.context.escalation.enabled,
                config.context.escalation.max_rounds,
                config.context.experience.max_edges,
            )
        };
        let mut engine = ContextEngine::new(&store)
            .with_budget_policy(load_budget_policy(&repo)?)
            .with_token_counter(token_counter.counter)
            .with_token_fallback(token_counter.fallback_used, token_counter.fallback_reason)
            .with_escalation(escalation.0, escalation.1);
        if let Some(session) = overview_session.as_ref() {
            engine = engine.with_overview_store(&repo, session);
        }
        let experience_session =
            atlas_session::SessionStore::open(&atlas_engine::paths::session_db_path(&db_path)).ok();
        if let Some(session) = experience_session.as_ref() {
            engine = engine.with_experience_store(&repo, session, escalation.2);
        }
        let result = engine.build(&request).context("context engine failed")?;
        let linked_decisions = search_linked_decisions(&repo, &request);

        if cli.json {
            let mut value = serde_json::to_value(&result)?;
            if let Some(object) = value.as_object_mut() {
                if include_ranking_evidence {
                    object.insert(
                        "context_ranking_evidence_legend".to_owned(),
                        atlas_core::context_ranking_evidence_legend(),
                    );
                }
                object.insert(
                    "agent_scope".to_owned(),
                    serde_json::json!({
                        "agent_id": agent_id,
                        "merge_agent_partitions": merge_agent_partitions,
                    }),
                );
                if !linked_decisions.is_empty() {
                    object.insert(
                        "linked_decisions".to_owned(),
                        serde_json::to_value(&linked_decisions)?,
                    );
                }
            }
            print_json("context", value)?;
        } else {
            println!(
                "{}",
                format_context_output_with_decisions(&result, &linked_decisions)
            );
            if agent_id.is_some() || merge_agent_partitions {
                println!(
                    "Agent scope: {}",
                    if merge_agent_partitions {
                        "merged"
                    } else {
                        agent_id.as_deref().unwrap_or("default")
                    }
                );
            }
        }

        if !linked_decisions.is_empty()
            && let Some(ref mut a) = adapter
        {
            let lookup_query = decision_lookup_query(&request).unwrap_or_default();
            let evidence = linked_decisions
                .iter()
                .take(3)
                .map(|hit| {
                    serde_json::json!({
                        "decision_id": hit.decision.decision_id,
                        "summary": hit.decision.summary,
                        "relevance_score": hit.relevance_score,
                    })
                })
                .collect::<Vec<_>>();
            let source_ids = linked_decisions
                .iter()
                .flat_map(|hit| hit.decision.source_ids.iter().cloned())
                .take(5)
                .collect::<Vec<_>>();
            a.record(extract_decision_event_with_details(
                &format!("reuse prior decision for context: {lookup_query}"),
                Some("stored decision memory matched current context request"),
                serde_json::json!({
                    "query": lookup_query,
                    "conclusion": "prior decision reused for context request",
                    "source_ids": source_ids,
                    "evidence": evidence,
                }),
            ));
        }

        Ok(())
    })();

    if result.is_ok()
        && let Some(ref mut a) = adapter
    {
        a.record(extract_context_event("cli:context", 0));
    }
    if let Some(ref mut a) = adapter {
        a.after_command("context", result.is_ok());
    }
    result
}
