use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use super::scope::should_auto_detect;

pub(super) const INSTRUCTIONS_MARKER: &str = "<!-- atlas MCP tools -->";
pub(super) const INSTRUCTIONS_END_MARKER: &str = "<!-- /atlas MCP tools -->";

pub(super) const INSTRUCTIONS_SECTION: &str = r#"<!-- atlas MCP tools -->
## MCP Tools: atlas

**IMPORTANT: This project has a code knowledge graph. ALWAYS use the
atlas MCP tools BEFORE using file-search or grep to explore the codebase.**
The graph is faster, cheaper (fewer tokens), and gives you structural context
(callers, dependents, test coverage) that file scanning cannot.

### When to use atlas tools first

- **Exploring code**: `query_graph` to find candidate symbols, then `symbol_neighbors`, `traverse_graph`, or `get_context` for callers/callees and usage relationships
- **Searching non-symbol content**: `search_files`, `search_content`, `search_templates`, and `search_text_assets` when graph lookup is the wrong tool
- **Understanding impact**: `get_impact_radius` for blast radius, `explain_change` for richer risk analysis
- **Code review**: `detect_changes` + `get_review_context`, or `get_minimal_context` when tokens matter
- **Finding relationships**: `symbol_neighbors` for immediate usage edges, `traverse_graph` for broader callers/callees, and `get_context` for intent-aware usage lookup
- **Repo health**: `list_graph_stats`, `status`, `doctor`, `db_check`, and `debug_graph` before trusting graph-backed answers
- **Session continuity**: `get_session_status`, `resume_session`, `search_saved_context`, and `save_context_artifact`

Do not treat `query_graph` as caller/callee search. Fall back to file tools **only** after graph relationship tools do not cover what you need.

### Tool list

| Tool | Use when |
| ---- | -------- |
| `list_graph_stats` | Overall graph metrics and language breakdown |
| `query_graph` | Search graph nodes by keyword, kind, or language; returns symbol matches, not usage edges |
| `batch_query_graph` | Run up to 20 query_graph searches in one call |
| `search_files` | Find config, template, SQL, Markdown, and other files by path or glob |
| `search_content` | Search file contents when you need text matches instead of graph symbols |
| `search_templates` | Discover template files by engine or extension |
| `search_text_assets` | Find SQL, config, env, and prompt files outside graph-symbol lookup |
| `status` | Check graph health and basic build state before trusting graph-backed answers |
| `doctor` | Run deeper repo, config, DB, and index health checks |
| `db_check` | Validate SQLite integrity and detect orphan or dangling graph records |
| `debug_graph` | Inspect node and edge breakdowns plus structural anomalies |
| `explain_query` | See how query_graph tokenizes and executes a search |
| `resolve_symbol` | Resolve a symbol or alias-qualified name to a canonical qualified_name |
| `analyze_safety` | Score refactor safety using callers, fan-out, and test adjacency |
| `analyze_remove` | Estimate removal impact with bounded evidence and warnings |
| `analyze_dead_code` | Find likely dead-code candidates with certainty and blockers |
| `analyze_dependency` | Check whether a symbol can be removed without remaining references |
| `get_impact_radius` | Understand blast radius from a changed file set |
| `get_review_context` | Build review bundle with symbols, neighbors, edges, and risk summary |
| `detect_changes` | Ask Atlas for changed files instead of shelling out to git |
| `build_or_update_graph` | Trigger full graph build or incremental update |
| `traverse_graph` | Walk callers, callees, and nearby nodes from known qualified name |
| `get_minimal_context` | Get lower-token review context with auto-detected changes |
| `explain_change` | Get deterministic risk analysis, change kinds, and test gaps |
| `get_context` | Build bounded context around symbol, file, or change-set |
| `get_session_status` | Inspect current session identity, event count, and resume state |
| `resume_session` | Restore prior session snapshot after reconnect or restart |
| `search_saved_context` | Search saved artifacts from earlier large outputs |
| `read_saved_context` | Retrieve full artifact content by source_id with optional paging |
| `save_context_artifact` | Persist large context payloads for later retrieval |
| `get_context_stats` | Inspect session and content-store stats |
| `purge_saved_context` | Remove saved artifacts by session or age |
| `symbol_neighbors` | Inspect immediate callers, callees, tests, and local graph neighborhood |
| `cross_file_links` | Find files coupled to a file through shared symbol references |
| `concept_clusters` | Group related files around seed files by coupling density |

### Workflow

1. Start with MCP graph tools to get structural context.
2. For usage questions, use `query_graph` to find the qualified name, then `symbol_neighbors`, `traverse_graph`, or `get_context`.
3. Use `detect_changes` to identify changed files.
4. Use `get_review_context` or `get_minimal_context` for review.
5. Use `get_impact_radius` or `explain_change` to assess change risk.

### Trust Metadata

- Every MCP tool response includes `atlas_provenance` with `repo_root`, `db_path`, `indexed_file_count`, and `last_indexed_at`.
- Some graph-backed tools also include `atlas_freshness` when working-tree edits may make graph results stale.
- If `atlas_provenance.repo_root` does not match current workspace, stop and verify session wiring.
- If `atlas_provenance.db_path` points at unexpected database, stop and call `status` or `doctor` before trusting results.
- If `atlas_freshness.stale` is true, prefer `build_or_update_graph` before making claims about current code.
- Treat missing optional `atlas_*` fields as "not applicable", not as tool failure.

### Path Identity Invariant

- Canonicalize repo paths before hashing, persistence, dedupe, cache keys, source IDs, chunk IDs, snapshot file references, or qualified-name prefixes.
- Apply same rule to future sidecar/cache/index keys, including in-memory parser caches and retrieval sidecars keyed by repo files.
- Use `atlas_repo::CanonicalRepoPath` or helper APIs built on it. Do not add local path-normalization helpers for path-derived identity.
- If `doctor` or `db_check` reports `noncanonical_path_rows`, prefer rebuild from clean canonical inputs instead of in-place row rewrites.
<!-- /atlas MCP tools -->
"#;

pub(super) fn instruction_targets(platform: &str, repo_root: &Path) -> Vec<&'static str> {
    let mut targets = Vec::new();

    let wants_agents = match platform {
        "copilot" | "codex" => true,
        "all" => should_auto_detect("copilot", repo_root) || should_auto_detect("codex", repo_root),
        _ => false,
    };
    let wants_claude = match platform {
        "claude" => true,
        "all" => should_auto_detect("claude", repo_root),
        _ => false,
    };

    if wants_agents {
        targets.push("AGENTS.md");
    }
    if wants_claude {
        targets.push("CLAUDE.md");
    }

    targets
}

pub fn inject_instructions(
    repo_root: &Path,
    filenames: &[&str],
    dry_run: bool,
) -> Result<Vec<String>> {
    let mut updated = Vec::new();

    for filename in filenames {
        let path = repo_root.join(filename);
        let existing = if path.exists() {
            fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?
        } else {
            String::new()
        };

        let next_content = upsert_instructions_section(&existing);
        if next_content == existing {
            continue;
        }

        if dry_run {
            if existing.is_empty() {
                println!("  [dry-run] would create {filename}");
            } else if existing.contains(INSTRUCTIONS_MARKER) {
                println!("  [dry-run] would refresh atlas instructions in {filename}");
            } else {
                println!("  [dry-run] would append atlas instructions to {filename}");
            }
            updated.push(filename.to_string());
            continue;
        }

        fs::write(&path, next_content)
            .with_context(|| format!("cannot write {}", path.display()))?;
        updated.push(filename.to_string());
    }

    Ok(updated)
}

fn upsert_instructions_section(existing: &str) -> String {
    if let Some((start, end)) = instruction_section_range(existing) {
        let mut updated = String::new();
        let prefix = &existing[..start];
        updated.push_str(prefix);
        if !prefix.is_empty() && !prefix.ends_with('\n') {
            updated.push_str("\n\n");
        }
        updated.push_str(INSTRUCTIONS_SECTION);
        let suffix = &existing[end..];
        if !suffix.is_empty() {
            if !updated.ends_with('\n') && !suffix.starts_with('\n') {
                updated.push('\n');
            }
            updated.push_str(suffix);
        }
        return updated;
    }

    let separator = if existing.is_empty() {
        ""
    } else if existing.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };

    format!("{existing}{separator}{INSTRUCTIONS_SECTION}")
}

fn instruction_section_range(existing: &str) -> Option<(usize, usize)> {
    let start = existing.find(INSTRUCTIONS_MARKER)?;
    let mut end = existing[start..]
        .find(INSTRUCTIONS_END_MARKER)
        .map(|offset| start + offset + INSTRUCTIONS_END_MARKER.len())
        .unwrap_or(existing.len());
    if existing[end..].starts_with("\r\n") {
        end += 2;
    } else if existing[end..].starts_with('\n') {
        end += 1;
    }
    Some((start, end))
}
