-- Migration 008: historical graph metadata foundation
-- Tables: repos, commits, graph_snapshots, snapshot_files

CREATE TABLE IF NOT EXISTS repos (
    repo_id     INTEGER PRIMARY KEY,
    root_path   TEXT    NOT NULL UNIQUE,
    created_at  TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_repos_root_path ON repos (root_path);

CREATE TABLE IF NOT EXISTS commits (
    commit_sha      TEXT    NOT NULL,
    repo_id         INTEGER NOT NULL REFERENCES repos (repo_id),
    parent_sha      TEXT,
    author_name     TEXT,
    author_email    TEXT,
    author_time     INTEGER NOT NULL,
    committer_time  INTEGER NOT NULL,
    subject         TEXT    NOT NULL,
    message         TEXT,
    indexed_at      TEXT    NOT NULL,
    PRIMARY KEY (commit_sha, repo_id)
);

CREATE INDEX IF NOT EXISTS idx_commits_repo_id        ON commits (repo_id);
CREATE INDEX IF NOT EXISTS idx_commits_author_time    ON commits (author_time);
CREATE INDEX IF NOT EXISTS idx_commits_committer_time ON commits (committer_time);

CREATE TABLE IF NOT EXISTS graph_snapshots (
    snapshot_id      INTEGER PRIMARY KEY,
    repo_id          INTEGER NOT NULL REFERENCES repos (repo_id),
    commit_sha       TEXT    NOT NULL,
    root_tree_hash   TEXT,
    node_count       INTEGER NOT NULL DEFAULT 0,
    edge_count       INTEGER NOT NULL DEFAULT 0,
    file_count       INTEGER NOT NULL DEFAULT 0,
    created_at       TEXT    NOT NULL,
    completeness     REAL    NOT NULL DEFAULT 1.0,
    parse_error_count INTEGER NOT NULL DEFAULT 0,
    UNIQUE (repo_id, commit_sha)
);

CREATE INDEX IF NOT EXISTS idx_graph_snapshots_repo_id    ON graph_snapshots (repo_id);
CREATE INDEX IF NOT EXISTS idx_graph_snapshots_commit_sha ON graph_snapshots (commit_sha);

CREATE TABLE IF NOT EXISTS snapshot_files (
    snapshot_id INTEGER NOT NULL REFERENCES graph_snapshots (snapshot_id),
    file_path   TEXT    NOT NULL,
    file_hash   TEXT    NOT NULL,
    language    TEXT,
    size        INTEGER,
    PRIMARY KEY (snapshot_id, file_path)
);

CREATE INDEX IF NOT EXISTS idx_snapshot_files_snapshot_id ON snapshot_files (snapshot_id);
CREATE INDEX IF NOT EXISTS idx_snapshot_files_file_hash   ON snapshot_files (file_hash);
