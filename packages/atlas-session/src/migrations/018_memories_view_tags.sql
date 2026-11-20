-- M1 follow-up: expose `tags_json` on the memories compatibility view so
-- tag-filtered recall (primary, auxiliary, and embedding paths) can push the
-- predicate into SQL.

DROP VIEW IF EXISTS memories;

CREATE VIEW memories AS
SELECT i.id,
       i.repo_root,
       i.session_id,
       i.frontend,
       i.scope,
       i.topic,
       i.title,
       i.body,
       i.importance,
       i.created_at,
       i.updated_at,
       COALESCE(a.last_accessed_at, i.created_at) AS last_accessed_at,
       COALESCE(a.decay_score, 0) AS decay_score,
       i.source_id,
       i.metadata_json,
       i.tags_json,
       i.superseded_by,
       i.op_history_json,
       i.numeric_value
FROM memory_index i
LEFT JOIN memory_access a ON a.memory_id = i.id;
