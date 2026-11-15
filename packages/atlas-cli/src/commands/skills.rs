//! `atlas skills` command dispatch.
//!
//! Skills are versioned guide content embedded in the binary, so agents can load
//! current Atlas instructions at runtime instead of relying on a copy frozen
//! into `AGENTS.md` at install time.
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
                            .map(|skill| json!({ "name": skill.name, "summary": skill.summary }))
                            .collect::<Vec<_>>(),
                    }),
                )
            } else {
                for skill in skills::all() {
                    println!("{:10}  {}", skill.name, skill.summary);
                }
                println!("\nRead one with `atlas skills get <name>`.");
                Ok(())
            }
        }
        SkillsCommand::Get { name } => {
            let Some(skill) = skills::get(name) else {
                bail!("unknown skill '{name}'; available: {}", skills::names());
            };
            if cli.json {
                print_json(
                    "skills.get",
                    json!({
                        "name": skill.name,
                        "summary": skill.summary,
                        "content": skill.content,
                    }),
                )
            } else {
                print!("{}", skill.content);
                if !skill.content.ends_with('\n') {
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
                },
            },
        };

        let error = run_skills(&cli).unwrap_err().to_string();
        assert!(error.contains("unknown skill 'missing'"));
        assert!(error.contains("core"));
    }
}
