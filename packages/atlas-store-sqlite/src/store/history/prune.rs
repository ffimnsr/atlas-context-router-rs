use anyhow::Result;
use rusqlite::params;

use super::Store;

impl Store {
    pub fn delete_history_snapshots(&self, snapshot_ids: &[i64]) -> Result<()> {
        if snapshot_ids.is_empty() {
            return Ok(());
        }

        let tx = self.conn.unchecked_transaction()?;
        {
            let mut delete_files =
                tx.prepare_cached("DELETE FROM snapshot_files WHERE snapshot_id = ?1")?;
            let mut delete_snapshots =
                tx.prepare_cached("DELETE FROM graph_snapshots WHERE snapshot_id = ?1")?;
            for snapshot_id in snapshot_ids {
                delete_files.execute(params![snapshot_id])?;
                delete_snapshots.execute(params![snapshot_id])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_history_commits(&self, repo_id: i64, commit_shas: &[String]) -> Result<()> {
        if commit_shas.is_empty() {
            return Ok(());
        }

        let tx = self.conn.unchecked_transaction()?;
        {
            let mut delete_commit =
                tx.prepare_cached("DELETE FROM commits WHERE repo_id = ?1 AND commit_sha = ?2")?;
            for commit_sha in commit_shas {
                delete_commit.execute(params![repo_id, commit_sha])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn prune_orphan_historical_file_graphs(&self) -> Result<(u64, u64, u64)> {
        let orphan_hashes = {
            let mut stmt = self.conn.prepare(
                "SELECT DISTINCT hn.file_hash
                 FROM historical_nodes hn
                 LEFT JOIN snapshot_files sf ON sf.file_hash = hn.file_hash
                 WHERE sf.file_hash IS NULL
                 ORDER BY hn.file_hash ASC",
            )?;
            let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        if orphan_hashes.is_empty() {
            return Ok((0, 0, 0));
        }

        let tx = self.conn.unchecked_transaction()?;
        let mut removed_hashes = 0u64;
        let mut removed_nodes = 0u64;
        let mut removed_edges = 0u64;
        {
            let mut count_nodes =
                tx.prepare_cached("SELECT COUNT(*) FROM historical_nodes WHERE file_hash = ?1")?;
            let mut count_edges =
                tx.prepare_cached("SELECT COUNT(*) FROM historical_edges WHERE file_hash = ?1")?;
            let mut delete_nodes =
                tx.prepare_cached("DELETE FROM historical_nodes WHERE file_hash = ?1")?;
            let mut delete_edges =
                tx.prepare_cached("DELETE FROM historical_edges WHERE file_hash = ?1")?;
            for file_hash in orphan_hashes {
                let node_count: u64 =
                    count_nodes.query_row(params![&file_hash], |row| row.get(0))?;
                let edge_count: u64 =
                    count_edges.query_row(params![&file_hash], |row| row.get(0))?;
                delete_nodes.execute(params![&file_hash])?;
                delete_edges.execute(params![&file_hash])?;
                removed_hashes += 1;
                removed_nodes += node_count;
                removed_edges += edge_count;
            }
        }
        tx.commit()?;
        Ok((removed_hashes, removed_nodes, removed_edges))
    }
}
