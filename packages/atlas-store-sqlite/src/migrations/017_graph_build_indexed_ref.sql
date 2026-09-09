-- Migration 017: record the git ref a build/update synced from.
--
-- Enables the default `atlas update` target to diff against the last indexed
-- ref instead of the git index, so committed changes (e.g. a refactor split
-- into new module files) are picked up without `--base`/`--staged`.

ALTER TABLE graph_build_state
    ADD COLUMN last_indexed_ref TEXT;