ALTER TABLE graph_build_state
    ADD COLUMN files_accepted INTEGER NOT NULL DEFAULT 0;

ALTER TABLE graph_build_state
    ADD COLUMN files_skipped_by_byte_budget INTEGER NOT NULL DEFAULT 0;

ALTER TABLE graph_build_state
    ADD COLUMN bytes_accepted INTEGER NOT NULL DEFAULT 0;

ALTER TABLE graph_build_state
    ADD COLUMN bytes_skipped INTEGER NOT NULL DEFAULT 0;

ALTER TABLE graph_build_state
    ADD COLUMN budget_stop_reason TEXT;
