use super::super::*;
use super::parse;
use clap::Parser;

#[test]
fn parse_query_text_only() {
    let cli = parse(&["atlas", "query", "ReplaceFileGraph"]);
    if let Command::Query {
        text,
        kind,
        language,
        include_files,
        limit,
        ..
    } = cli.command
    {
        assert_eq!(text, "ReplaceFileGraph");
        assert!(kind.is_none());
        assert!(language.is_none());
        assert!(!include_files);
        assert_eq!(limit, 20);
    } else {
        panic!("expected Query command");
    }
}
#[test]
fn parse_man_command() {
    let cli = parse(&["atlas", "man", "mcp", "query_graph"]);
    if let Command::Man {
        namespace,
        tool_name,
    } = cli.command
    {
        assert_eq!(namespace, "mcp");
        assert_eq!(tool_name, "query_graph");
    } else {
        panic!("expected Man command");
    }
}
#[test]
fn parse_man_missing_tool_name_fails() {
    assert!(Cli::try_parse_from(["atlas", "man", "mcp"]).is_err());
}
#[test]
fn parse_docs_section_heading_selector() {
    let cli = parse(&[
        "atlas",
        "docs-section",
        "README.md",
        "--heading",
        "document.overview.install",
        "--max-bytes",
        "2048",
    ]);
    if let Command::DocsSection {
        path,
        heading,
        line,
        max_bytes,
    } = cli.command
    {
        assert_eq!(path, "README.md");
        assert_eq!(heading.as_deref(), Some("document.overview.install"));
        assert_eq!(line, None);
        assert_eq!(max_bytes, 2048);
    } else {
        panic!("expected DocsSection command");
    }
}
#[test]
fn parse_docs_section_line_selector() {
    let cli = parse(&["atlas", "docs-section", "README.md", "--line", "7"]);
    if let Command::DocsSection {
        path,
        heading,
        line,
        max_bytes,
    } = cli.command
    {
        assert_eq!(path, "README.md");
        assert_eq!(heading, None);
        assert_eq!(line, Some(7));
        assert_eq!(max_bytes, 16_384);
    } else {
        panic!("expected DocsSection command");
    }
}
#[test]
fn parse_query_with_kind_and_language_filters() {
    let cli = parse(&[
        "atlas",
        "query",
        "foo",
        "--kind",
        "function",
        "--language",
        "rust",
    ]);
    if let Command::Query {
        text,
        kind,
        language,
        ..
    } = cli.command
    {
        assert_eq!(text, "foo");
        assert_eq!(kind.as_deref(), Some("function"));
        assert_eq!(language.as_deref(), Some("rust"));
    } else {
        panic!("expected Query command");
    }
}
#[test]
fn parse_query_expand_flag() {
    let cli = parse(&["atlas", "query", "foo", "--expand", "--expand-hops", "3"]);
    if let Command::Query {
        expand,
        expand_hops,
        fuzzy,
        ..
    } = cli.command
    {
        assert!(expand);
        assert_eq!(expand_hops, 3);
        assert!(!fuzzy);
    } else {
        panic!("expected Query command");
    }
}
#[test]
fn parse_query_fuzzy_flag() {
    let cli = parse(&["atlas", "query", "greter", "--fuzzy"]);
    if let Command::Query { fuzzy, .. } = cli.command {
        assert!(fuzzy);
    } else {
        panic!("expected Query command");
    }
}
#[test]
fn parse_query_include_files_flag() {
    let cli = parse(&["atlas", "query", "guide", "--include-files"]);
    if let Command::Query { include_files, .. } = cli.command {
        assert!(include_files);
    } else {
        panic!("expected Query command");
    }
}
#[test]
fn parse_query_all_repos_flag() {
    let cli = parse(&["atlas", "query", "guide", "--all-repos"]);
    if let Command::Query {
        all_repos, repo_id, ..
    } = cli.command
    {
        assert!(all_repos);
        assert!(repo_id.is_none());
    } else {
        panic!("expected Query command");
    }
}
