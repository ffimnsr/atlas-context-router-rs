-- ICM-B — memory curation: superseded markers and supersession links.
--
-- `superseded_by` marks memory rows replaced by a consolidated memory;
-- recall ranks non-superseded rows first and list hides them by default.
-- `memory_supersessions` keeps the deterministic link log
-- (old_memory_id -> new_memory_id + reason) used by consolidation apply.

ALTER TABLE memories ADD COLUMN superseded_by TEXT;

CREATE TABLE memory_supersessions (
    old_memory_id TEXT NOT NULL,
    new_memory_id TEXT NOT NULL,
    reason       TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    PRIMARY KEY (old_memory_id, new_memory_id)
);

CREATE INDEX idx_memories_superseded
    ON memories(superseded_by);

CREATE INDEX idx_memory_supersessions_new
    ON memory_supersessions(new_memory_id);
