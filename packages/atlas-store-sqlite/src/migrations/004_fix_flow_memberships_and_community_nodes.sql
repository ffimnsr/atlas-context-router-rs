-- Migration 004: fix flow_memberships FK and add community_nodes join table
--
-- flow_memberships previously used a CASCADE FK on node_qualified_name which
-- silently deleted memberships whenever `atlas build` / `atlas update` rebuilt
-- the node row for a file. Memberships must be durable across graph refreshes
-- because they are user data, not derived from source. Rebuild the table as a
-- soft reference (no FK on node_qualified_name).
--
-- community_nodes is a new join table for attaching graph nodes to a community.
-- Same soft-reference design: community membership survives graph rebuilds.

-- Step 1: recreate flow_memberships without the fragile node FK.
CREATE TABLE IF NOT EXISTS flow_memberships_new (
    flow_id              INTEGER NOT NULL,
    node_qualified_name  TEXT    NOT NULL,
    position             INTEGER,
    role                 TEXT,
    extra_json           TEXT,
    PRIMARY KEY (flow_id, node_qualified_name),
    FOREIGN KEY(flow_id) REFERENCES flows(id) ON DELETE CASCADE
);

INSERT OR IGNORE INTO flow_memberships_new
    SELECT flow_id, node_qualified_name, position, role, extra_json
    FROM flow_memberships;

DROP TABLE flow_memberships;

ALTER TABLE flow_memberships_new RENAME TO flow_memberships;

CREATE INDEX IF NOT EXISTS idx_flow_memberships_node_qualified_name
    ON flow_memberships (node_qualified_name);
CREATE INDEX IF NOT EXISTS idx_flow_memberships_flow_position
    ON flow_memberships (flow_id, position);

-- Step 2: community_nodes join table.
-- Nodes are a soft reference so membership survives node replacement.
CREATE TABLE IF NOT EXISTS community_nodes (
    community_id         INTEGER NOT NULL,
    node_qualified_name  TEXT    NOT NULL,
    PRIMARY KEY (community_id, node_qualified_name),
    FOREIGN KEY(community_id) REFERENCES communities(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_community_nodes_node_qn
    ON community_nodes (node_qualified_name);
