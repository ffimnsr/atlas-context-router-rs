//! ICM-H2 — read-only memory TUI.
//!
//! The first version never mutates storage: it collects bounded snapshots from
//! the session store and routed artifact index, renders them in fixed sections,
//! and supports navigation only. Non-interactive runs (JSON output, pipes,
//! tests) render one frame instead of entering the key loop, so the surface
//! stays smoke-testable without a terminal.

use anyhow::{Result, bail};

use atlas_session::SessionStore;

use crate::commands::print_json;

mod model;
mod render;

#[cfg(test)]
mod tests;

pub use model::{TuiFilters, TuiModel, TuiSection, collect_model};
pub use render::{frame_lines, model_json};

/// Entry point for `atlas memory tui`.
pub fn run_memory_tui(
    repo: &str,
    store: &SessionStore,
    json: bool,
    filters: TuiFilters,
) -> Result<()> {
    let model = collect_model(repo, store, &filters)?;

    if json {
        return print_json("memory.tui", model_json(&model, &filters));
    }

    let interactive = std::io::IsTerminal::is_terminal(&std::io::stdin())
        && std::io::IsTerminal::is_terminal(&std::io::stdout());
    if !interactive {
        for line in frame_lines(&model, TuiSection::Memories, 0, filters.limit, 100) {
            println!("{line}");
        }
        return Ok(());
    }

    run_loop(&model, &filters)
}

/// Interactive read-only key loop (q/Esc quit, tab/h/l section, j/k scroll).
fn run_loop(model: &TuiModel, filters: &TuiFilters) -> Result<()> {
    let term = console::Term::stdout();
    let mut section = TuiSection::Memories;
    let mut offset = 0_usize;

    term.hide_cursor().ok();
    let result = (|| -> Result<()> {
        loop {
            let (rows, width) = term.size();
            let rows = rows as usize;
            let width = width as usize;
            let page = rows.saturating_sub(4).max(1);
            let limit = filters.limit.max(1).min(page);
            let lines = frame_lines(model, section, offset, limit, width);
            term.clear_screen().ok();
            term.move_cursor_to(0, 0).ok();
            term.write_str(&lines.join("\n")).ok();
            term.flush().ok();

            let key = match term.read_key() {
                Ok(key) => key,
                Err(error) => {
                    if error.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    }
                    bail!("cannot read terminal input: {error}");
                }
            };

            match key {
                console::Key::Char('q') | console::Key::Escape | console::Key::CtrlC => break,
                console::Key::Tab | console::Key::ArrowRight => {
                    section = section.next();
                    offset = 0;
                }
                console::Key::Char('l') => {
                    section = section.next();
                    offset = 0;
                }
                console::Key::BackTab | console::Key::ArrowLeft => {
                    section = section.prev();
                    offset = 0;
                }
                console::Key::Char('h') => {
                    section = section.prev();
                    offset = 0;
                }
                console::Key::Char('j') | console::Key::ArrowDown => {
                    offset = (offset + 1).min(scroll_max(model, section, limit));
                }
                console::Key::Char('k') | console::Key::ArrowUp => {
                    offset = offset.saturating_sub(1);
                }
                console::Key::PageDown | console::Key::Char(' ') => {
                    offset = (offset + page).min(scroll_max(model, section, limit));
                }
                console::Key::PageUp => offset = offset.saturating_sub(page),
                console::Key::Char('g') | console::Key::Home => offset = 0,
                console::Key::Char('G') | console::Key::End => {
                    offset = scroll_max(model, section, limit);
                }
                _ => {}
            }
        }
        Ok(())
    })();

    term.show_cursor().ok();
    result
}

fn scroll_max(model: &TuiModel, section: TuiSection, limit: usize) -> usize {
    model.rows(section).len().saturating_sub(limit)
}
