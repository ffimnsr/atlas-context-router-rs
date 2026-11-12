//! Frame rendering for the read-only memory TUI (ICM-H2).
//!
//! Rendering is pure: `frame_lines` takes the collected model and returns
//! bounded text lines, so the surface can be asserted in tests with no
//! terminal and no storage writes.

use serde_json::Value;

use super::model::{TuiFilters, TuiModel, TuiSection};

const SECONDARY_INDENT: &str = "    ";

/// Render one frame: header tabs, section rows, footer hints.
pub fn frame_lines(
    model: &TuiModel,
    section: TuiSection,
    offset: usize,
    limit: usize,
    width: usize,
) -> Vec<String> {
    let width = width.max(40);
    let mut lines = Vec::new();
    lines.push(clip(
        &format!("Atlas memory · read-only · {}", model.repo_root),
        width,
    ));
    lines.push(clip(
        &TuiSection::ALL
            .iter()
            .map(|entry| {
                if *entry == section {
                    format!("[{}]", entry.title())
                } else {
                    format!(" {} ", entry.title())
                }
            })
            .collect::<Vec<_>>()
            .join("|"),
        width,
    ));
    lines.push("─".repeat(width.min(120)));

    let rows = model.rows(section);
    let limit = limit.max(1);
    if rows.is_empty() {
        lines.push(format!("  (no {} entries)", section.title().to_lowercase()));
    } else {
        let start = offset.min(rows.len().saturating_sub(1));
        for row in rows.iter().skip(start).take(limit) {
            lines.push(clip(&format!("• {}", row.primary), width));
            if !row.secondary.is_empty() {
                lines.push(clip(&format!("{SECONDARY_INDENT}{}", row.secondary), width));
            }
        }
        let shown = rows.len().min(start + limit);
        lines.push(format!("  [{shown}/{} rows]", rows.len()));
    }

    if !model.warnings.is_empty() {
        lines.push(String::new());
        for warning in model.warnings.iter().take(3) {
            lines.push(clip(&format!("! {warning}"), width));
        }
    }

    lines.push(String::new());
    lines.push(clip(
        "q quit · tab/h/l section · j/k scroll · space/b page · g/G top/bottom",
        width,
    ));
    lines
}

/// Stable JSON projection of the read-only model for `--json`.
pub fn model_json(model: &TuiModel, filters: &TuiFilters) -> Value {
    let sections = TuiSection::ALL
        .iter()
        .map(|section| {
            serde_json::json!({
                "section": section.title().to_lowercase(),
                "rows": model
                    .rows(*section)
                    .iter()
                    .map(|row| serde_json::json!({
                        "primary": row.primary,
                        "secondary": row.secondary,
                    }))
                    .collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();

    serde_json::json!({
        "mode": "read_only",
        "repo_root": model.repo_root,
        "filters": {
            "topic": filters.memory.topic,
            "scope": filters.memory.scope.map(|scope| scope.as_str().to_owned()),
            "importance": filters.memory.importance.map(|importance| importance.as_str().to_owned()),
            "frontend": filters.frontend,
            "subject_kind": filters.subject_kind,
            "freshness": filters.freshness,
            "package": filters.package,
            "module": filters.module,
            "file": filters.file,
            "source_kind": filters.source_kind,
            "older_than": filters.overview_older_than,
            "newer_than": filters.overview_newer_than,
            "limit": filters.limit,
        },
        "sections": sections,
        "row_count": model.total_rows(),
        "warnings": model.warnings,
    })
}

fn clip(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let mut clipped = text
        .chars()
        .take(width.saturating_sub(1))
        .collect::<String>();
    clipped.push('…');
    clipped
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::memory_tui::model::TuiRow;

    fn sample_model() -> TuiModel {
        let mut model = TuiModel::new("/repo");
        model.insert(
            TuiSection::Memories,
            vec![
                TuiRow::new("postgres · pool sizing", "critical · id m1"),
                TuiRow::new("hooks · policy", "normal · id m2"),
            ],
        );
        model
    }

    #[test]
    fn frame_lists_active_section_rows_and_hints() {
        let lines = frame_lines(&sample_model(), TuiSection::Memories, 0, 10, 80);
        let text = lines.join("\n");
        assert!(text.contains("[Memories]"));
        assert!(text.contains("postgres · pool sizing"));
        assert!(text.contains("critical · id m1"));
        assert!(text.contains("q quit"));
    }

    #[test]
    fn frame_reports_empty_sections_without_panicking() {
        let lines = frame_lines(&sample_model(), TuiSection::Artifacts, 0, 10, 80);
        let text = lines.join("\n");
        assert!(text.contains("(no artifacts entries)"));
    }

    #[test]
    fn frame_clips_long_rows_to_width() {
        let mut model = TuiModel::new("/repo");
        model.insert(
            TuiSection::Memories,
            vec![TuiRow::new("x".repeat(200), String::new())],
        );
        let lines = frame_lines(&model, TuiSection::Memories, 0, 10, 40);
        assert!(lines.iter().all(|line| line.chars().count() <= 40));
    }

    #[test]
    fn offset_past_the_end_still_renders_last_page() {
        let lines = frame_lines(&sample_model(), TuiSection::Memories, 99, 1, 80);
        let text = lines.join("\n");
        assert!(text.contains("hooks · policy"));
    }
}
