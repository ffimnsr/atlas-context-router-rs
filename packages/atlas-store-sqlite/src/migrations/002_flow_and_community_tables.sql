-- Migration 002: optional flow/community tables for post-MVP features

CREATE TABLE IF NOT EXISTS flows (
    id          INTEGER PRIMARY KEY,
    name        TEXT    NOT NULL UNIQUE,
    kind        TEXT,
    description TEXT,
    extra_json  TEXT,
    created_at  TEXT    NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  TEXT    NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_flows_kind ON flows (kind);

CREATE TABLE IF NOT EXISTS flow_memberships (
    flow_id              INTEGER NOT NULL,
    node_qualified_name  TEXT    NOT NULL,
    position             INTEGER,
    role                 TEXT,
    extra_json           TEXT,
    PRIMARY KEY (flow_id, node_qualified_name),
    FOREIGN KEY(flow_id) REFERENCES flows(id) ON DELETE CASCADE,
    FOREIGN KEY(node_qualified_name) REFERENCES nodes(qualified_name) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_flow_memberships_node_qualified_name
    ON flow_memberships (node_qualified_name);
CREATE INDEX IF NOT EXISTS idx_flow_memberships_flow_position
    ON flow_memberships (flow_id, position);

CREATE TABLE IF NOT EXISTS communities (
    id                   INTEGER PRIMARY KEY,
    name                 TEXT    NOT NULL UNIQUE,
    algorithm            TEXT,
    level                INTEGER,
    parent_community_id  INTEGER,
    extra_json           TEXT,
    created_at           TEXT    NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at           TEXT    NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(parent_community_id) REFERENCES communities(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS idx_communities_algorithm ON communities (algorithm);
CREATE INDEX IF NOT EXISTS idx_communities_parent ON communities (parent_community_id);
