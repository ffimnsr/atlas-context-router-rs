use anyhow::{Context, Result};
use atlas_repo::{CanonicalRepoPath, find_repo_root};
use atlas_review::{DocsSectionLookup, DocsSectionSelector, lookup_docs_section};
use atlas_store_sqlite::Store;
use camino::Utf8Path;

use crate::cli::{Cli, Command};

use super::{db_path, print_json, resolve_repo};

pub fn run_docs_section(cli: &Cli) -> Result<()> {
    let Command::DocsSection {
        path,
        heading,
        line,
        max_bytes,
    } = &cli.command
    else {
        anyhow::bail!("docs-section command required");
    };

    if usize::from(heading.is_some()) + usize::from(line.is_some()) != 1 {
        anyhow::bail!("provide exactly one selector: heading or line");
    }

    let repo = resolve_repo(cli)?;
    let repo_root = find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
    let canonical = CanonicalRepoPath::from_cli_argument(repo_root.as_path(), Utf8Path::new(path))
        .with_context(|| format!("invalid explicit file path '{path}'"))?;
    let store = Store::open(&db_path(cli, &repo))?;
    let selector = if let Some(heading) = heading {
        DocsSectionSelector::Heading(heading.clone())
    } else {
        DocsSectionSelector::Line(line.expect("validated line selector"))
    };
    let result = lookup_docs_section(
        &store,
        repo_root.as_path(),
        canonical.as_str(),
        selector,
        *max_bytes,
    )?;

    if cli.json {
        print_json("docs_section", serde_json::to_value(&result)?)
    } else {
        println!("{}", render_docs_section_text(&result));
        Ok(())
    }
}

fn render_docs_section_text(result: &DocsSectionLookup) -> String {
    if !result.resolved {
        let mut lines = vec![format!(
            "Ambiguous heading selector '{}' in {}.",
            result.query.as_deref().unwrap_or_default(),
            result.file
        )];
        for candidate in &result.candidates {
            lines.push(format!(
                "- {} [{}] lines {}-{}",
                candidate.heading_path, candidate.title, candidate.start_line, candidate.end_line
            ));
        }
        return lines.join("\n");
    }

    let mut lines = vec![format!(
        "{} ({}) lines {}-{}",
        result.heading_path.as_deref().unwrap_or_default(),
        result.file,
        result.start_line.unwrap_or_default(),
        result.end_line.unwrap_or_default()
    )];
    if result.truncated {
        lines.push(format!(
            "truncated: omitted {} bytes",
            result.omitted_byte_count
        ));
    }
    lines.push(String::new());
    lines.push(result.content.clone().unwrap_or_default());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_review::{DocsSectionCandidate, DocsSectionLine};

    #[test]
    fn render_ambiguous_heading_candidates() {
        let text = render_docs_section_text(&DocsSectionLookup {
            file: "README.md".to_owned(),
            selector_kind: "heading".to_owned(),
            resolved: false,
            query: Some("install".to_owned()),
            title: None,
            heading_path: None,
            heading_slug: None,
            heading_level: None,
            start_line: None,
            end_line: None,
            line_count: None,
            file_hash: None,
            content: None,
            lines: Vec::<DocsSectionLine>::new(),
            truncated: false,
            omitted_byte_count: 0,
            candidates: vec![DocsSectionCandidate {
                title: "Install".to_owned(),
                heading_path: "document.one.install".to_owned(),
                heading_slug: "install".to_owned(),
                heading_level: 2,
                start_line: 2,
                end_line: 3,
            }],
            atlas_result_kind: "docs_section",
        });

        assert!(text.contains("Ambiguous heading selector 'install'"));
        assert!(text.contains("document.one.install"));
    }
}
