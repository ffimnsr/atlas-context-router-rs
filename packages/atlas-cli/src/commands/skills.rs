//! `atlas skills` command dispatch.
//!
//! Skills are versioned guide content embedded in the binary, so agents can load
//! current Atlas instructions at runtime instead of relying on a copy frozen
//! into `AGENTS.md` at install time. The `cli` and `mcp-tools` references are
//! rendered from the live binary and MCP registry, so they cannot go stale.
//!
//! Intentionally CLI-only: the managed `AGENTS.md` pointer tells agents to run
//! `atlas skills get core`, so an MCP mirror would duplicate guide content
//! without adding reach.

use anyhow::{Result, bail};
use serde_json::json;

use crate::cli::{Cli, Command, SkillsCommand};
use crate::skills;

use super::print_json;

pub fn run_skills(cli: &Cli) -> Result<()> {
    let Command::Skills { subcommand } = &cli.command else {
        bail!("skills command required");
    };

    match subcommand {
        SkillsCommand::List => {
            if cli.json {
                print_json(
                    "skills.list",
                    json!({
                        "skills": skills::all()
                            .iter()
                            .map(|skill| json!({
                                "name": skill.name,
                                "summary": skill.summary,
                                "references": skill.references.iter().map(|reference| json!({
                                    "name": reference.name,
                                    "summary": reference.summary,
                                })).collect::<Vec<_>>(),
                            }))
                            .collect::<Vec<_>>(),
                    }),
                )
            } else {
                for skill in skills::all() {
                    println!("{}  {}", skill.name, skill.summary);
                    println!("  references: {}", skills::reference_names(skill));
                }
                println!(
                    "\nRead a guide with `atlas skills get <name>`.\n\
                     Read one reference with `atlas skills get <name> --ref <reference>`.\n\
                     Read everything with `atlas skills get <name> --full`."
                );
                Ok(())
            }
        }
        SkillsCommand::Get {
            name,
            reference,
            full,
        } => {
            let Some(skill) = skills::get(name) else {
                bail!("unknown skill '{name}'; available: {}", skills::names());
            };

            let (kind, content) = if let Some(reference_name) = reference {
                let Some(content) = skills::render_reference(reference_name) else {
                    bail!(
                        "unknown reference '{reference_name}' for skill '{name}'; available: {}",
                        skills::reference_names(skill)
                    );
                };
                ("reference", content)
            } else {
                ("guide", skills::render_skill(skill, *full))
            };

            if cli.json {
                print_json(
                    "skills.get",
                    json!({
                        "name": skill.name,
                        "summary": skill.summary,
                        "kind": kind,
                        "reference": reference,
                        "full": full,
                        "content": content,
                    }),
                )
            } else {
                print!("{content}");
                if !content.ends_with('\n') {
                    println!();
                }
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_cli(reference: Option<&str>, full: bool) -> Cli {
        Cli {
            repo: None,
            db: None,
            verbose: false,
            json: false,
            command: Command::Skills {
                subcommand: SkillsCommand::Get {
                    name: "core".to_owned(),
                    reference: reference.map(str::to_owned),
                    full,
                },
            },
        }
    }

    #[test]
    fn unknown_skill_error_lists_available_names() {
        let cli = Cli {
            repo: None,
            db: None,
            verbose: false,
            json: false,
            command: Command::Skills {
                subcommand: SkillsCommand::Get {
                    name: "missing".to_owned(),
                    reference: None,
                    full: false,
                },
            },
        };

        let error = run_skills(&cli).unwrap_err().to_string();
        assert!(error.contains("unknown skill 'missing'"));
        assert!(error.contains("core"));
    }

    #[test]
    fn unknown_reference_error_lists_available_references() {
        let error = run_skills(&get_cli(Some("missing"), false))
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown reference 'missing'"));
        assert!(error.contains("workflows"));
        assert!(error.contains("mcp-tools"));
    }
}
