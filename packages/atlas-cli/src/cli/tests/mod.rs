use super::*;
use clap::Parser;

mod core;
mod global;
mod history;
mod impact_review;
mod insights;
mod install;
mod memory;
mod query;
mod update;

fn parse(args: &[&str]) -> Cli {
    Cli::try_parse_from(args).expect("parse should succeed")
}
