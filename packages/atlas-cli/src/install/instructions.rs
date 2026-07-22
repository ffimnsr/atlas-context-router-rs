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

ALWAYS use TOON output everytime, and JSON when you expect there's floating numbers.

### When to use atlas tools first

- **Exploring code**: `query_graph` to find candidate symbols, then `symbol_neighbors`, `traverse_graph`, or `get_context` for callers/callees and usage relationships
- **Companion content lookup**: after graph tools surface structural context, use `search_files`, `search_content`, `search_templates`, or `search_text_assets` when changed files or graph evidence points to non-code assets (docs, config, SQL, templates, prompts). Do not search content before graph resolution for symbol questions.
- **Mixed graph/content context**: pass non-code asset paths into `get_context` via `files` to merge graph and content evidence under one bounded selection, ranking, and truncation policy.
- **Understanding impact**: `get_impact_radius` for blast radius, `explain_change` for richer risk analysis
- **Code review**: `detect_changes` + `get_review_context`, or `get_minimal_context` when tokens matter; follow up with `search_text_assets` or `search_templates` when changed files include config/templates/SQL/prompts
- **Finding relationships**: `symbol_neighbors` for immediate usage edges, `traverse_graph` for broader callers/callees, and `get_context` for intent-aware usage lookup
- **Repo health**: `list_graph_stats`, `status`, `doctor`, `db_check`, and `debug_graph` before trusting graph-backed answers
- **Session continuity**: `get_session_status`, `resume_session`, `search_saved_context`, `search_decisions`, and `save_context_artifact`

Do not treat `query_graph` as caller/callee search. Fall back to file tools **only** after graph relationship tools do not cover what you need.

### Tool discovery

- `tool_list`: list current visible exported MCP tools at runtime instead of hardcoding long tool tables in agent instructions
- `tool_search`: search tools by name/title/description with explicit score factors and typo-tolerant fuzzy name matching when exact tool name is unclear
- `tool_help`: read runtime docs for one exact exported MCP tool name
- `man`: legacy namespace-aware manual alias; use when caller already speaks `namespace/tool_name`

### Core workflow surfaces

- `query_graph`, then `symbol_neighbors`, `traverse_graph`, or `get_context` for symbol lookup and relationships
- `detect_changes`, `get_review_context`, `get_minimal_context`, `get_impact_radius`, and `explain_change` for change review and blast radius
- `status`, `doctor`, `db_check`, `debug_graph`, and `list_graph_stats` for repo and graph health
- `search_files`, `search_content`, `read_file_excerpt`, `get_docs_section`, `read_file_around_match`, `search_templates`, and `search_text_assets` for non-code or known-file companion lookup
- `get_session_status`, `resume_session`, `search_saved_context`, `search_decisions`, and `save_context_artifact` for continuity

### Workflow

1. Start with MCP graph tools to get structural context.
2. For usage questions, use `query_graph` to find the qualified name, then `symbol_neighbors`, `traverse_graph`, or `get_context`.
3. Use `detect_changes` to identify changed files.
4. Use `get_review_context` or `get_minimal_context` for review.
5. Use `get_impact_radius` or `explain_change` to assess change risk.
6. When changed files include docs, config, templates, prompts, or SQL, call `search_text_assets` or `search_templates` as companion lookup after graph tools run. Pass asset paths into `get_context` via `files` to merge graph and content under one bounded budget.
7. When graph evidence shows edges to non-code files (config, SQL, template, prompt), use `search_text_assets` or `search_content` as companion lookup. Do not search content before graph resolution for symbol questions.

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
