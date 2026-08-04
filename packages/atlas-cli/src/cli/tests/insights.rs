use super::super::*;
use super::parse;

#[test]
fn parse_analyze_dead_code_with_subpath() {
    let cli = parse(&["atlas", "analyze", "dead-code", "--subpath", "src"]);
    if let Command::Analyze { subcommand, .. } = cli.command {
        match subcommand {
            AnalyzeCommand::DeadCode { subpath, limit, .. } => {
                assert_eq!(subpath.as_deref(), Some("src"));
                assert_eq!(limit, 100);
            }
            _ => panic!("expected analyze dead-code"),
        }
    } else {
        panic!("expected Analyze command");
    }
}
#[test]
fn parse_insights_large_functions_with_thresholds() {
    let cli = parse(&[
        "atlas",
        "insights",
        "large-functions",
        "--files",
        "src/lib.rs",
        "src/api.rs",
        "--threshold",
        "120",
        "--complexity-threshold",
        "18",
        "--cognitive-threshold",
        "24",
        "--nesting-threshold",
        "5",
        "--mode",
        "complex",
        "--limit",
        "7",
        "--include-tests",
    ]);
    if let Command::Insights {
        subcommand,
        allow_stale,
        allow_partial,
    } = cli.command
    {
        assert!(!allow_stale);
        assert!(!allow_partial);
        match subcommand {
            InsightsCommand::LargeFunctions {
                files,
                threshold,
                complexity_threshold,
                cognitive_threshold,
                nesting_threshold,
                mode,
                limit,
                include_tests,
            } => {
                assert_eq!(files, vec!["src/lib.rs", "src/api.rs"]);
                assert_eq!(threshold, Some(120));
                assert_eq!(complexity_threshold, Some(18));
                assert_eq!(cognitive_threshold, Some(24));
                assert_eq!(nesting_threshold, Some(5));
                assert_eq!(
                    mode,
                    crate::cli::subcommands::InsightsLargeFunctionMode::Complex
                );
                assert_eq!(limit, Some(7));
                assert!(include_tests);
            }
            _ => panic!("expected insights large-functions"),
        }
    } else {
        panic!("expected Insights command");
    }
}
#[test]
fn parse_insights_architecture_with_limit() {
    let cli = parse(&["atlas", "insights", "architecture", "--limit", "9"]);
    if let Command::Insights { subcommand, .. } = cli.command {
        match subcommand {
            InsightsCommand::Architecture { limit } => assert_eq!(limit, Some(9)),
            _ => panic!("expected insights architecture"),
        }
    } else {
        panic!("expected Insights command");
    }
}
#[test]
fn parse_insights_risk_symbol() {
    let cli = parse(&["atlas", "insights", "risk", "src/lib.rs::fn::helper"]);
    if let Command::Insights { subcommand, .. } = cli.command {
        match subcommand {
            InsightsCommand::Risk { symbol } => assert_eq!(symbol, "src/lib.rs::fn::helper"),
            _ => panic!("expected insights risk"),
        }
    } else {
        panic!("expected Insights command");
    }
}
#[test]
fn parse_insights_complex_functions_with_thresholds() {
    let cli = parse(&[
        "atlas",
        "insights",
        "complex-functions",
        "--files",
        "src/lib.rs",
        "--complexity-threshold",
        "18",
        "--cognitive-threshold",
        "24",
        "--nesting-threshold",
        "5",
        "--limit",
        "7",
        "--include-tests",
    ]);
    if let Command::Insights { subcommand, .. } = cli.command {
        match subcommand {
            InsightsCommand::ComplexFunctions {
                files,
                complexity_threshold,
                cognitive_threshold,
                nesting_threshold,
                limit,
                include_tests,
            } => {
                assert_eq!(files, vec!["src/lib.rs"]);
                assert_eq!(complexity_threshold, Some(18));
                assert_eq!(cognitive_threshold, Some(24));
                assert_eq!(nesting_threshold, Some(5));
                assert_eq!(limit, Some(7));
                assert!(include_tests);
            }
            _ => panic!("expected insights complex-functions"),
        }
    } else {
        panic!("expected Insights command");
    }
}
#[test]
fn parse_insights_similar_functions_with_thresholds() {
    let cli = parse(&[
        "atlas",
        "insights",
        "similar-functions",
        "src/lib.rs::fn::helper",
        "--min-score",
        "0.61",
        "--limit",
        "6",
        "--include-same-file",
    ]);
    if let Command::Insights { subcommand, .. } = cli.command {
        match subcommand {
            InsightsCommand::SimilarFunctions {
                symbol,
                min_score,
                limit,
                include_same_file,
            } => {
                assert_eq!(symbol, "src/lib.rs::fn::helper");
                assert_eq!(min_score, Some(0.61));
                assert_eq!(limit, Some(6));
                assert!(include_same_file);
            }
            _ => panic!("expected insights similar-functions"),
        }
    } else {
        panic!("expected Insights command");
    }
}
#[test]
fn parse_insights_duplicates_with_thresholds() {
    let cli = parse(&[
        "atlas",
        "insights",
        "duplicates",
        "--files",
        "src/lib.rs",
        "src/api.rs",
        "--min-score",
        "0.77",
        "--limit",
        "5",
        "--include-tests",
        "--suppress",
        "src/generated",
    ]);
    if let Command::Insights { subcommand, .. } = cli.command {
        match subcommand {
            InsightsCommand::Duplicates {
                files,
                min_score,
                limit,
                include_tests,
                suppressions,
            } => {
                assert_eq!(files, vec!["src/lib.rs", "src/api.rs"]);
                assert_eq!(min_score, Some(0.77));
                assert_eq!(limit, Some(5));
                assert!(include_tests);
                assert_eq!(suppressions, vec!["src/generated"]);
            }
            _ => panic!("expected insights duplicates"),
        }
    } else {
        panic!("expected Insights command");
    }
}
#[test]
fn parse_insights_infer_modules_with_limit() {
    let cli = parse(&["atlas", "insights", "infer-modules", "--limit", "11"]);
    if let Command::Insights { subcommand, .. } = cli.command {
        match subcommand {
            InsightsCommand::InferModules { limit } => assert_eq!(limit, Some(11)),
            _ => panic!("expected insights infer-modules"),
        }
    } else {
        panic!("expected Insights command");
    }
}
#[test]
fn parse_insights_label_components_with_filters() {
    let cli = parse(&[
        "atlas",
        "insights",
        "label-components",
        "--files",
        "src/lib.rs",
        "docs/README.md",
        "--symbol",
        "src/lib.rs::fn::helper",
        "src/api.rs::fn::handle",
        "--limit",
        "8",
    ]);
    if let Command::Insights { subcommand, .. } = cli.command {
        match subcommand {
            InsightsCommand::LabelComponents {
                files,
                symbols,
                limit,
            } => {
                assert_eq!(files, vec!["src/lib.rs", "docs/README.md"]);
                assert_eq!(
                    symbols,
                    vec!["src/lib.rs::fn::helper", "src/api.rs::fn::handle"]
                );
                assert_eq!(limit, Some(8));
            }
            _ => panic!("expected insights label-components"),
        }
    } else {
        panic!("expected Insights command");
    }
}
#[test]
fn parse_refactor_rename_with_named_flags() {
    let cli = parse(&[
        "atlas",
        "refactor",
        "rename",
        "--symbol",
        "src/lib.rs::fn::helper",
        "--to",
        "helper_renamed",
        "--dry-run",
    ]);
    if let Command::Refactor { subcommand, .. } = cli.command {
        match subcommand {
            RefactorCommand::Rename {
                symbol,
                to,
                dry_run,
                legacy_symbol,
                legacy_to,
            } => {
                assert_eq!(symbol.as_deref(), Some("src/lib.rs::fn::helper"));
                assert_eq!(to.as_deref(), Some("helper_renamed"));
                assert!(dry_run);
                assert!(legacy_symbol.is_none());
                assert!(legacy_to.is_none());
            }
            _ => panic!("expected refactor rename"),
        }
    } else {
        panic!("expected Refactor command");
    }
}
#[test]
fn parse_refactor_rename_legacy_positionals() {
    let cli = parse(&[
        "atlas",
        "refactor",
        "rename",
        "src/lib.rs::fn::helper",
        "helper_renamed",
        "--dry-run",
    ]);
    if let Command::Refactor { subcommand, .. } = cli.command {
        match subcommand {
            RefactorCommand::Rename {
                symbol,
                to,
                dry_run,
                legacy_symbol,
                legacy_to,
            } => {
                assert!(symbol.is_none());
                assert!(to.is_none());
                assert!(dry_run);
                assert_eq!(legacy_symbol.as_deref(), Some("src/lib.rs::fn::helper"));
                assert_eq!(legacy_to.as_deref(), Some("helper_renamed"));
            }
            _ => panic!("expected refactor rename"),
        }
    } else {
        panic!("expected Refactor command");
    }
}
#[test]
fn parse_hidden_hook_command() {
    let cli = parse(&["atlas", "hook", "session-start"]);
    match cli.command {
        Command::Hook { event } => assert_eq!(event, "session-start"),
        _ => panic!("expected Hook command"),
    }
}
