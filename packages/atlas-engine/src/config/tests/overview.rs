use super::*;

#[test]
fn overview_config_defaults_are_safe_and_non_llm() {
    let config = Config::default();
    assert!(config.overview.enabled);
    assert!(!config.overview.auto_export_on_commit);
    assert!(config.overview.auto_mark_stale_on_commit);
    assert!(config.overview.external_analyzer_command.is_none());
    assert!(!config.overview.ingest_after_external_command);
    assert_eq!(config.overview.max_subjects_per_run, 50);
    assert_eq!(config.overview.max_packet_bytes, 1_048_576);
    config.overview.validate().unwrap();
}

#[test]
fn overview_config_rejects_unsafe_limits_and_env_commands() {
    let mut config = Config::default();
    config.overview.max_subjects_per_run = 0;
    assert!(config.overview.validate().is_err());

    let mut config = Config::default();
    config.overview.max_packet_bytes = 1;
    assert!(config.overview.validate().is_err());

    let mut config = Config::default();
    config.overview.external_analyzer_command = Some("   ".to_owned());
    assert!(config.overview.validate().is_err());

    let mut config = Config::default();
    config.overview.external_analyzer_command = Some("$ATLAS_ANALYZER".to_owned());
    assert!(config.overview.validate().is_err());

    let mut config = Config::default();
    config.overview.external_analyzer_command = Some("python3 analyzer.py".to_owned());
    assert!(config.overview.validate().is_ok());
}
