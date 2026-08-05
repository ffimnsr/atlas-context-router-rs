ALTER TABLE graph_build_state
    ADD COLUMN recovery_mode TEXT;

ALTER TABLE graph_build_state
    ADD COLUMN quarantine_path TEXT;
