-- ICM-F1 — memoir concept graph, stored beside shared memory in the session
-- database. Deliberately NOT part of worldtree.db: memoir concepts and
-- relations never merge into code graph `nodes`/`edges`.
--
-- Design rules:
-- - Separate graph ids namespace memoir content away from code graph ids.
-- - Scope is constrained at the storage boundary; graph identity is
--   `(repo_root, scope, name)` with case-insensitive names.
-- - Concept identity is `(graph_id, name)` with case-insensitive names.
-- - Relation types are validated in the Rust service layer against the
--   controlled vocabulary so future custom-relation config can relax the
--   service without a schema migration.

CREATE TABLE memoir_graphs (
    id            TEXT PRIMARY KEY,
    repo_root     TEXT NOT NULL,
    scope         TEXT NOT NULL DEFAULT 'project',
    name          TEXT NOT NULL COLLATE NOCASE,
    description   TEXT NOT NULL DEFAULT '',
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    CHECK (scope IN ('project', 'session', 'frontend', 'global')),
    UNIQUE (repo_root, scope, name)
);

CREATE INDEX idx_memoir_graphs_repo_scope
    ON memoir_graphs(repo_root, scope);

CREATE TABLE memoir_concepts (
    id            TEXT PRIMARY KEY,
    graph_id      TEXT NOT NULL REFERENCES memoir_graphs(id) ON DELETE CASCADE,
    repo_root     TEXT NOT NULL,
    name          TEXT NOT NULL COLLATE NOCASE,
    description   TEXT NOT NULL DEFAULT '',
    kind          TEXT NOT NULL DEFAULT 'concept',
    source_id     TEXT,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    UNIQUE (graph_id, name)
);

CREATE INDEX idx_memoir_concepts_graph
    ON memoir_concepts(graph_id, name);

CREATE INDEX idx_memoir_concepts_repo_name
    ON memoir_concepts(repo_root, name);

CREATE TABLE memoir_relations (
    id                TEXT PRIMARY KEY,
    graph_id          TEXT NOT NULL REFERENCES memoir_graphs(id) ON DELETE CASCADE,
    repo_root         TEXT NOT NULL,
    source_concept_id TEXT NOT NULL REFERENCES memoir_concepts(id) ON DELETE CASCADE,
    target_concept_id TEXT NOT NULL REFERENCES memoir_concepts(id) ON DELETE CASCADE,
    relation_type     TEXT NOT NULL,
    confidence        REAL NOT NULL DEFAULT 1.0,
    source_id         TEXT,
    created_at        TEXT NOT NULL,
    metadata_json     TEXT NOT NULL DEFAULT '{}',
    CHECK (confidence >= 0.0 AND confidence <= 1.0),
    CHECK (source_concept_id <> target_concept_id),
    UNIQUE (graph_id, source_concept_id, target_concept_id, relation_type)
);

CREATE INDEX idx_memoir_relations_graph
    ON memoir_relations(graph_id, relation_type);

CREATE INDEX idx_memoir_relations_source
    ON memoir_relations(graph_id, source_concept_id);

CREATE INDEX idx_memoir_relations_target
    ON memoir_relations(graph_id, target_concept_id);
