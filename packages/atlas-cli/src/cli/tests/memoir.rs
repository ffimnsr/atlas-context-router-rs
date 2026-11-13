use super::super::*;
use super::parse;

#[test]
fn parse_memoir_create_command() {
    let cli = parse(&[
        "atlas",
        "memoir",
        "create",
        "Architecture",
        "--description",
        "system map",
        "--scope",
        "global",
    ]);
    assert!(matches!(
        cli.command,
        Command::Memoir {
            subcommand:
                MemoirCommand::Create {
                    ref name,
                    ref description,
                    ref scope,
                }
        } if name == "Architecture"
            && description.as_deref() == Some("system map")
            && scope.as_deref() == Some("global")
    ));
}

#[test]
fn parse_memoir_add_concept_command() {
    let cli = parse(&[
        "atlas",
        "memoir",
        "add-concept",
        "Architecture",
        "parser",
        "source parser",
        "--kind",
        "component",
        "--source-id",
        "artifact-1",
    ]);
    assert!(matches!(
        cli.command,
        Command::Memoir {
            subcommand:
                MemoirCommand::AddConcept {
                    ref graph,
                    ref name,
                    ref description,
                    ref kind,
                    ref source_id,
                    ..
                }
        } if graph == "Architecture"
            && name == "parser"
            && description.as_deref() == Some("source parser")
            && kind.as_deref() == Some("component")
            && source_id.as_deref() == Some("artifact-1")
    ));
}

#[test]
fn parse_memoir_link_command() {
    let cli = parse(&[
        "atlas",
        "memoir",
        "link",
        "Architecture",
        "parser",
        "lexer",
        "--relation",
        "depends_on",
        "--confidence",
        "0.5",
        "--source-id",
        "artifact-2",
        "--allow-missing-concepts",
    ]);
    assert!(matches!(
        cli.command,
        Command::Memoir {
            subcommand:
                MemoirCommand::Link {
                    ref graph,
                    ref source,
                    ref target,
                    ref relation,
                    confidence,
                    ref source_id,
                    allow_missing_concepts: true,
                    ..
                }
        } if graph == "Architecture"
            && source == "parser"
            && target == "lexer"
            && relation.as_deref() == Some("depends_on")
            && confidence == 0.5
            && source_id.as_deref() == Some("artifact-2")
    ));
}

#[test]
fn parse_memoir_inspect_command_defaults_depth_to_one() {
    let cli = parse(&["atlas", "memoir", "inspect", "parser"]);
    assert!(matches!(
        cli.command,
        Command::Memoir {
            subcommand: MemoirCommand::Inspect {
                ref targets,
                ref graph,
                depth: 1,
                ref relation,
                ..
            }
        } if targets == &["parser".to_owned()] && graph.is_none() && relation.is_none()
    ));
}

#[test]
fn parse_memoir_link_accepts_positional_relation() {
    let cli = parse(&[
        "atlas",
        "memoir",
        "link",
        "Architecture",
        "parser",
        "lexer",
        "depends_on",
    ]);
    assert!(matches!(
        cli.command,
        Command::Memoir {
            subcommand: MemoirCommand::Link {
                ref relation_positional,
                ..
            }
        } if relation_positional.as_deref() == Some("depends_on")
    ));
}

#[test]
fn parse_memoir_inspect_accepts_graph_positionally() {
    let cli = parse(&[
        "atlas",
        "memoir",
        "inspect",
        "Architecture",
        "HealthDiagnostics",
    ]);
    assert!(matches!(
        cli.command,
        Command::Memoir {
            subcommand: MemoirCommand::Inspect {
                ref targets,
                depth: 1,
                ..
            }
        } if targets == &["Architecture".to_owned(), "HealthDiagnostics".to_owned()]
    ));
}
