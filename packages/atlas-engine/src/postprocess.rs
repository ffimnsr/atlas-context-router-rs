use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::Context;
use atlas_core::{
    ChangeType, GraphStats, PostprocessExecutionMode, PostprocessRunState, PostprocessRunSummary,
    PostprocessStageStatus, PostprocessStageSummary,
};
use atlas_repo::{DiffTarget, changed_files, find_repo_root};
use atlas_store_sqlite::Store;
use camino::Utf8Path;

pub const POSTPROCESS_STAGE_FLOWS: &str = "flows";
pub const POSTPROCESS_STAGE_COMMUNITIES: &str = "communities";
pub const POSTPROCESS_STAGE_ARCHITECTURE_METRICS: &str = "architecture_metrics";
pub const POSTPROCESS_STAGE_QUERY_HINTS: &str = "query_hints";
pub const POSTPROCESS_STAGE_LARGE_FUNCTION_SUMMARIES: &str = "large_function_summaries";

const LARGE_FUNCTION_MIN_LINES: usize = 40;
const LARGE_FUNCTION_LIMIT: usize = 25;
const QUERY_HINT_LIMIT: usize = 5;

#[derive(Debug, Clone)]
pub struct PostprocessOptions {
    pub changed_only: bool,
    pub stage: Option<String>,
    pub dry_run: bool,
}

impl PostprocessOptions {
    pub fn mode(&self) -> PostprocessExecutionMode {
        if self.changed_only {
            PostprocessExecutionMode::ChangedOnly
        } else {
            PostprocessExecutionMode::Full
        }
    }
}

pub fn supported_postprocess_stages() -> Vec<String> {
    vec![
        POSTPROCESS_STAGE_FLOWS.to_string(),
        POSTPROCESS_STAGE_COMMUNITIES.to_string(),
        POSTPROCESS_STAGE_ARCHITECTURE_METRICS.to_string(),
        POSTPROCESS_STAGE_QUERY_HINTS.to_string(),
        POSTPROCESS_STAGE_LARGE_FUNCTION_SUMMARIES.to_string(),
    ]
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or_default()
}

fn qn_file_path(qualified_name: &str) -> &str {
    qualified_name
        .split_once("::")
        .map(|(path, _)| path)
        .unwrap_or(qualified_name)
}

fn graph_built(stats: &GraphStats) -> bool {
    stats.file_count > 0 || stats.node_count > 0 || stats.edge_count > 0
}

fn unknown_stage_summary(
    repo_root: &str,
    options: &PostprocessOptions,
    started_at_ms: i64,
) -> PostprocessRunSummary {
    let stage = options.stage.clone().unwrap_or_default();
    PostprocessRunSummary {
        repo_root: repo_root.to_string(),
        ok: false,
        noop: false,
        noop_reason: None,
        error_code: "unknown_stage".to_string(),
        message: format!("unknown postprocess stage '{stage}'"),
        suggestions: vec![
            "use one of the documented stage names".to_string(),
            "omit --stage to run the full postprocess pipeline".to_string(),
        ],
        graph_built: true,
        state: PostprocessRunState::Failed,
        requested_mode: options.mode(),
        stage_filter: options.stage.clone(),
        dry_run: options.dry_run,
        changed_files: Vec::new(),
        started_at_ms,
        finished_at_ms: now_ms(),
        total_elapsed_ms: 0,
        stages: Vec::new(),
        supported_stages: supported_postprocess_stages(),
    }
}

fn no_graph_summary(
    repo_root: &str,
    options: &PostprocessOptions,
    started_at_ms: i64,
) -> PostprocessRunSummary {
    PostprocessRunSummary {
        repo_root: repo_root.to_string(),
        ok: true,
        noop: true,
        noop_reason: Some("graph_not_built".to_string()),
        error_code: "none".to_string(),
        message: "graph not built; nothing to postprocess".to_string(),
        suggestions: vec![
            "run atlas build or atlas update first".to_string(),
            "re-run postprocess after graph state is current".to_string(),
        ],
        graph_built: false,
        state: PostprocessRunState::Succeeded,
        requested_mode: options.mode(),
        stage_filter: options.stage.clone(),
        dry_run: options.dry_run,
        changed_files: Vec::new(),
        started_at_ms,
        finished_at_ms: now_ms(),
        total_elapsed_ms: 0,
        stages: Vec::new(),
        supported_stages: supported_postprocess_stages(),
    }
}

fn resolve_changed_files(repo_root: &Utf8Path) -> anyhow::Result<Vec<String>> {
    let changes = changed_files(repo_root, &DiffTarget::WorkingTree)
        .context("cannot detect changed files")?;
    Ok(changes
        .into_iter()
        .filter(|change| change.change_type != ChangeType::Deleted)
        .map(|change| change.path)
        .collect())
}

fn scope_file_count(
    mode: PostprocessExecutionMode,
    stats: &GraphStats,
    changed_files: &[String],
) -> usize {
    match mode {
        PostprocessExecutionMode::Full => stats.file_count as usize,
        PostprocessExecutionMode::ChangedOnly => changed_files.len(),
    }
}

fn build_flows_stage(
    store: &Store,
    mode: PostprocessExecutionMode,
    stats: &GraphStats,
    changed_files: &[String],
) -> anyhow::Result<PostprocessStageSummary> {
    let started = Instant::now();
    let scope_count = scope_file_count(mode, stats, changed_files);
    let flows = store.list_flows().context("cannot list flows")?;
    let touched_flow_count =
        if mode == PostprocessExecutionMode::ChangedOnly && !changed_files.is_empty() {
            flows
                .iter()
                .filter(|flow| {
                    store
                        .get_flow_members(flow.id)
                        .map(|members| {
                            members.iter().any(|member| {
                                let path = qn_file_path(&member.node_qualified_name);
                                changed_files.iter().any(|file| file == path)
                            })
                        })
                        .unwrap_or(false)
                })
                .count()
        } else {
            flows.len()
        };
    let membership_count = flows
        .iter()
        .map(|flow| {
            store
                .get_flow_members(flow.id)
                .map(|members| members.len())
                .unwrap_or_default()
        })
        .sum::<usize>();
    Ok(PostprocessStageSummary {
        stage: POSTPROCESS_STAGE_FLOWS.to_string(),
        status: PostprocessStageStatus::Completed,
        mode,
        affected_file_count: scope_count,
        item_count: touched_flow_count,
        elapsed_ms: started.elapsed().as_millis() as u64,
        error_code: None,
        message: Some("flow summaries refreshed".to_string()),
        details: serde_json::json!({
            "flow_count": flows.len(),
            "memberships": membership_count,
            "flow_count_in_scope": touched_flow_count,
        }),
    })
}

fn build_communities_stage(
    store: &Store,
    mode: PostprocessExecutionMode,
    stats: &GraphStats,
    changed_files: &[String],
) -> anyhow::Result<PostprocessStageSummary> {
    let started = Instant::now();
    let scope_count = scope_file_count(mode, stats, changed_files);
    let communities = store
        .list_communities()
        .context("cannot list communities")?;
    let touched_community_count =
        if mode == PostprocessExecutionMode::ChangedOnly && !changed_files.is_empty() {
            communities
                .iter()
                .filter(|community| {
                    store
                        .get_community_nodes(community.id)
                        .map(|members| {
                            members.iter().any(|member| {
                                let path = qn_file_path(&member.node_qualified_name);
                                changed_files.iter().any(|file| file == path)
                            })
                        })
                        .unwrap_or(false)
                })
                .count()
        } else {
            communities.len()
        };
    let membership_count = communities
        .iter()
        .map(|community| {
            store
                .get_community_nodes(community.id)
                .map(|members| members.len())
                .unwrap_or_default()
        })
        .sum::<usize>();
    Ok(PostprocessStageSummary {
        stage: POSTPROCESS_STAGE_COMMUNITIES.to_string(),
        status: PostprocessStageStatus::Completed,
        mode,
        affected_file_count: scope_count,
        item_count: touched_community_count,
        elapsed_ms: started.elapsed().as_millis() as u64,
        error_code: None,
        message: Some("community summaries refreshed".to_string()),
        details: serde_json::json!({
            "community_count": communities.len(),
            "community_count_in_scope": touched_community_count,
            "memberships": membership_count,
        }),
    })
}

fn build_architecture_metrics_stage(
    mode: PostprocessExecutionMode,
    stats: &GraphStats,
    changed_files: &[String],
) -> PostprocessStageSummary {
    let started = Instant::now();
    PostprocessStageSummary {
        stage: POSTPROCESS_STAGE_ARCHITECTURE_METRICS.to_string(),
        status: PostprocessStageStatus::Completed,
        mode,
        affected_file_count: scope_file_count(mode, stats, changed_files),
        item_count: 1,
        elapsed_ms: started.elapsed().as_millis() as u64,
        error_code: None,
        message: Some("architecture metric bundle refreshed".to_string()),
        details: serde_json::json!({
            "file_count": stats.file_count,
            "node_count": stats.node_count,
            "edge_count": stats.edge_count,
            "languages": stats.languages,
            "nodes_by_kind": stats.nodes_by_kind,
        }),
    }
}

fn build_query_hints_stage(
    store: &Store,
    mode: PostprocessExecutionMode,
    stats: &GraphStats,
    changed_files: &[String],
) -> anyhow::Result<PostprocessStageSummary> {
    let started = Instant::now();
    let recent_files = store
        .recently_indexed_files(QUERY_HINT_LIMIT)
        .context("cannot load recently indexed files")?;
    let top_kind = stats
        .nodes_by_kind
        .iter()
        .max_by_key(|(_, count)| *count)
        .map(|(kind, count)| serde_json::json!({ "kind": kind, "count": count }))
        .unwrap_or(serde_json::Value::Null);

    Ok(PostprocessStageSummary {
        stage: POSTPROCESS_STAGE_QUERY_HINTS.to_string(),
        status: PostprocessStageStatus::Completed,
        mode,
        affected_file_count: scope_file_count(mode, stats, changed_files),
        item_count: recent_files.len(),
        elapsed_ms: started.elapsed().as_millis() as u64,
        error_code: None,
        message: Some("query hints refreshed".to_string()),
        details: serde_json::json!({
            "recent_files": recent_files,
            "top_kind": top_kind,
            "language_count": stats.languages.len(),
        }),
    })
}

fn build_large_function_stage(
    store: &Store,
    mode: PostprocessExecutionMode,
    stats: &GraphStats,
    changed_files: &[String],
) -> anyhow::Result<PostprocessStageSummary> {
    let started = Instant::now();
    let files = if mode == PostprocessExecutionMode::ChangedOnly {
        Some(changed_files)
    } else {
        None
    };
    let nodes = store
        .find_large_functions(files, LARGE_FUNCTION_MIN_LINES, LARGE_FUNCTION_LIMIT)
        .context("cannot summarize large functions")?;
    let top = nodes
        .iter()
        .take(QUERY_HINT_LIMIT)
        .map(|node| {
            serde_json::json!({
                "qn": node.qualified_name,
                "file_path": node.file_path,
                "line_count": node.line_end.saturating_sub(node.line_start).saturating_add(1),
            })
        })
        .collect::<Vec<_>>();

    Ok(PostprocessStageSummary {
        stage: POSTPROCESS_STAGE_LARGE_FUNCTION_SUMMARIES.to_string(),
        status: PostprocessStageStatus::Completed,
        mode,
        affected_file_count: scope_file_count(mode, stats, changed_files),
        item_count: nodes.len(),
        elapsed_ms: started.elapsed().as_millis() as u64,
        error_code: None,
        message: Some("large function summaries refreshed".to_string()),
        details: serde_json::json!({
            "min_lines": LARGE_FUNCTION_MIN_LINES,
            "top_large_functions": top,
        }),
    })
}

pub fn postprocess_graph(
    repo_root: &Utf8Path,
    db_path: &str,
    options: &PostprocessOptions,
) -> anyhow::Result<PostprocessRunSummary> {
    let started_at_ms = now_ms();
    let mode = options.mode();
    let supported_stages = supported_postprocess_stages();
    if let Some(stage) = options.stage.as_deref()
        && !supported_stages.iter().any(|candidate| candidate == stage)
    {
        return Ok(unknown_stage_summary(
            repo_root.as_str(),
            options,
            started_at_ms,
        ));
    }

    let store =
        Store::open(db_path).with_context(|| format!("cannot open database at {db_path}"))?;
    let stats = store.stats().context("cannot read graph stats")?;
    if !graph_built(&stats) {
        return Ok(no_graph_summary(repo_root.as_str(), options, started_at_ms));
    }

    let git_repo_root = find_repo_root(repo_root).context("cannot find git repo root")?;
    let changed_files = if mode == PostprocessExecutionMode::ChangedOnly {
        resolve_changed_files(git_repo_root.as_path())?
    } else {
        Vec::new()
    };

    if !options.dry_run {
        store
            .begin_postprocess(
                repo_root.as_str(),
                mode,
                options.stage.as_deref(),
                changed_files.len(),
            )
            .context("cannot record postprocess start")?;
    }

    let selected_stages = if let Some(stage) = options.stage.clone() {
        vec![stage]
    } else {
        supported_stages.clone()
    };

    let mut stages = Vec::new();
    let run_started = Instant::now();
    for stage in selected_stages {
        let stage_result = match stage.as_str() {
            POSTPROCESS_STAGE_FLOWS => build_flows_stage(&store, mode, &stats, &changed_files),
            POSTPROCESS_STAGE_COMMUNITIES => {
                build_communities_stage(&store, mode, &stats, &changed_files)
            }
            POSTPROCESS_STAGE_ARCHITECTURE_METRICS => Ok(build_architecture_metrics_stage(
                mode,
                &stats,
                &changed_files,
            )),
            POSTPROCESS_STAGE_QUERY_HINTS => {
                build_query_hints_stage(&store, mode, &stats, &changed_files)
            }
            POSTPROCESS_STAGE_LARGE_FUNCTION_SUMMARIES => {
                build_large_function_stage(&store, mode, &stats, &changed_files)
            }
            _ => unreachable!(),
        };

        match stage_result {
            Ok(summary) => stages.push(summary),
            Err(error) => {
                stages.push(PostprocessStageSummary {
                    stage,
                    status: PostprocessStageStatus::Failed,
                    mode,
                    affected_file_count: scope_file_count(mode, &stats, &changed_files),
                    item_count: 0,
                    elapsed_ms: 0,
                    error_code: Some("stage_failed".to_string()),
                    message: Some(error.to_string()),
                    details: serde_json::Value::Null,
                });

                let summary = PostprocessRunSummary {
                    repo_root: repo_root.as_str().to_string(),
                    ok: false,
                    noop: false,
                    noop_reason: None,
                    error_code: "stage_failed".to_string(),
                    message: stages
                        .last()
                        .and_then(|item| item.message.clone())
                        .unwrap_or_else(|| "postprocess stage failed".to_string()),
                    suggestions: vec![
                        "inspect the failed stage details".to_string(),
                        "rerun with --stage for narrower scope".to_string(),
                    ],
                    graph_built: true,
                    state: PostprocessRunState::Failed,
                    requested_mode: mode,
                    stage_filter: options.stage.clone(),
                    dry_run: options.dry_run,
                    changed_files,
                    started_at_ms,
                    finished_at_ms: now_ms(),
                    total_elapsed_ms: run_started.elapsed().as_millis() as u64,
                    stages,
                    supported_stages,
                };
                if !options.dry_run {
                    store
                        .finish_postprocess(&summary)
                        .context("cannot record failed postprocess run")?;
                }
                return Ok(summary);
            }
        }
    }

    let summary = PostprocessRunSummary {
        repo_root: repo_root.as_str().to_string(),
        ok: true,
        noop: false,
        noop_reason: None,
        error_code: "none".to_string(),
        message: if options.dry_run {
            "postprocess dry run complete".to_string()
        } else {
            "postprocess complete".to_string()
        },
        suggestions: vec![],
        graph_built: true,
        state: PostprocessRunState::Succeeded,
        requested_mode: mode,
        stage_filter: options.stage.clone(),
        dry_run: options.dry_run,
        changed_files,
        started_at_ms,
        finished_at_ms: now_ms(),
        total_elapsed_ms: run_started.elapsed().as_millis() as u64,
        stages,
        supported_stages,
    };
    if !options.dry_run {
        store
            .finish_postprocess(&summary)
            .context("cannot record postprocess completion")?;
    }
    Ok(summary)
}
