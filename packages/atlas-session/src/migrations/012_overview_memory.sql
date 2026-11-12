-- ICM-G2 — Code overview memory, stored beside shared memory in the session
-- database. Overview bodies and records never enter worldtree.db and are never
-- merged into code graph `nodes`/`edges`.
--
-- Identity rules:
-- - `subject_id` is a versioned hash over canonical subject fields (see
--   `overview::subject_id`). File-backed subjects must already carry a
--   canonical repo-relative path before persistence.
-- - An "active" record is one that no newer record supersedes. Re-ingest
--   creates a new record pointing at the previous active record through
--   `supersedes_id`; old records are retained for audit.
-- - The partial unique index allows one active record per
--   `(repo_root, subject_id, commit_sha, source_kind)` while letting superseded
--   rows keep the same natural key.

CREATE TABLE IF NOT EXISTS overview_records (
    id                     TEXT PRIMARY KEY,
    repo_root              TEXT NOT NULL,
    subject_id             TEXT NOT NULL,
    subject_kind           TEXT NOT NULL,
    package_name           TEXT,
    module_path            TEXT,
    canonical_file_path    TEXT,
    qualified_name         TEXT,
    node_kind              TEXT,
    line_start             INTEGER,
    line_end               INTEGER,
    content_hash           TEXT,
    title                  TEXT NOT NULL DEFAULT '',
    summary                TEXT NOT NULL DEFAULT '',
    description_preview    TEXT NOT NULL DEFAULT '',
    responsibilities_json  TEXT NOT NULL DEFAULT '[]',
    flow_json              TEXT NOT NULL DEFAULT '[]',
    inputs_json            TEXT NOT NULL DEFAULT '[]',
    outputs_json           TEXT NOT NULL DEFAULT '[]',
    invariants_json        TEXT NOT NULL DEFAULT '[]',
    gotchas_json           TEXT NOT NULL DEFAULT '[]',
    examples_json          TEXT NOT NULL DEFAULT '[]',
    tags_json              TEXT NOT NULL DEFAULT '[]',
    source_kind            TEXT NOT NULL,
    analyzer_name          TEXT,
    analyzer_version       TEXT,
    model_name             TEXT,
    commit_sha             TEXT NOT NULL,
    graph_last_indexed_at  TEXT,
    freshness              TEXT NOT NULL,
    confidence             REAL NOT NULL DEFAULT 0.5,
    source_id              TEXT,
    supersedes_id          TEXT REFERENCES overview_records(id),
    created_at             TEXT NOT NULL,
    updated_at             TEXT NOT NULL,
    metadata_json          TEXT NOT NULL DEFAULT '{}',
    CHECK (subject_kind IN ('project', 'package', 'module', 'file', 'symbol', 'function', 'method')),
    CHECK (freshness IN ('fresh', 'possibly_stale', 'stale', 'orphaned', 'unverified')),
    CHECK (source_kind IN ('external_llm', 'manual', 'imported')),
    CHECK (confidence >= 0.0 AND confidence <= 1.0)
);

CREATE INDEX IF NOT EXISTS idx_overview_records_repo
    ON overview_records(repo_root);

CREATE INDEX IF NOT EXISTS idx_overview_records_subject
    ON overview_records(repo_root, subject_id);

CREATE INDEX IF NOT EXISTS idx_overview_records_subject_kind
    ON overview_records(repo_root, subject_kind);

CREATE INDEX IF NOT EXISTS idx_overview_records_file
    ON overview_records(repo_root, canonical_file_path);

CREATE INDEX IF NOT EXISTS idx_overview_records_qname
    ON overview_records(repo_root, qualified_name);

CREATE INDEX IF NOT EXISTS idx_overview_records_commit
    ON overview_records(repo_root, commit_sha);

CREATE INDEX IF NOT EXISTS idx_overview_records_freshness
    ON overview_records(repo_root, freshness);

CREATE INDEX IF NOT EXISTS idx_overview_records_source_kind
    ON overview_records(repo_root, source_kind);

CREATE INDEX IF NOT EXISTS idx_overview_records_updated
    ON overview_records(repo_root, updated_at DESC);

CREATE UNIQUE INDEX IF NOT EXISTS idx_overview_records_active_key
    ON overview_records(repo_root, subject_id, commit_sha, source_kind)
    WHERE supersedes_id IS NULL;

-- Standalone FTS index. The service layer keeps it in sync explicitly so the
-- session database does not need trigger support across migrations.
CREATE VIRTUAL TABLE IF NOT EXISTS overview_record_fts USING fts5(
    overview_id UNINDEXED,
    title,
    summary,
    description_preview,
    responsibilities_json,
    flow_json,
    invariants_json,
    gotchas_json,
    tags_json
);

-- Bounded packet references let `overview ingest` validate that a caller is
-- returning analysis for a packet Atlas actually exported. Only packet
-- metadata is stored here; packet bodies stay in the caller's hands.
CREATE TABLE IF NOT EXISTS overview_packets (
    packet_id      TEXT PRIMARY KEY,
    repo_root      TEXT NOT NULL,
    subject_id     TEXT NOT NULL,
    commit_sha     TEXT NOT NULL,
    schema_version INTEGER NOT NULL,
    evidence_json  TEXT NOT NULL DEFAULT '[]',
    created_at     TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_overview_packets_repo
    ON overview_packets(repo_root, subject_id);
