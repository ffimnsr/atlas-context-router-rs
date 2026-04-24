use anyhow::{Context, Result};
use rusqlite::params;

use super::{HistoricalEdge, HistoricalNode, Store};

impl Store {
    pub fn get_historical_file_language(&self, file_hash: &str) -> Result<Option<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT language FROM historical_nodes WHERE file_hash = ?1 LIMIT 1")?;
        let mut rows = stmt.query(params![file_hash])?;
        if let Some(r) = rows.next()? {
            Ok(r.get(0)?)
        } else {
            Ok(None)
        }
    }

    pub fn list_historical_node_qns(&self, file_hash: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT qualified_name FROM historical_nodes WHERE file_hash = ?1")?;
        let rows = stmt.query_map(params![file_hash], |r| r.get(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("list historical node qns")
    }

    pub fn list_historical_nodes_for_hash(&self, file_hash: &str) -> Result<Vec<HistoricalNode>> {
        let mut stmt = self.conn.prepare(
            "SELECT file_hash, qualified_name, kind, name, file_path,
                    line_start, line_end, language, parent_name,
                    params, return_type, modifiers, is_test, extra_json
             FROM historical_nodes
             WHERE file_hash = ?1
             ORDER BY file_path ASC, qualified_name ASC",
        )?;
        let rows = stmt.query_map(params![file_hash], |r| {
            Ok(HistoricalNode {
                file_hash: r.get(0)?,
                qualified_name: r.get(1)?,
                kind: r.get(2)?,
                name: r.get(3)?,
                file_path: r.get(4)?,
                line_start: r.get(5)?,
                line_end: r.get(6)?,
                language: r.get(7)?,
                parent_name: r.get(8)?,
                params: r.get(9)?,
                return_type: r.get(10)?,
                modifiers: r.get(11)?,
                is_test: r.get::<_, i64>(12)? != 0,
                extra_json: r.get(13)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("list historical nodes for hash")
    }

    pub fn list_historical_edge_keys(
        &self,
        file_hash: &str,
    ) -> Result<Vec<(String, String, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT source_qn, target_qn, kind FROM historical_edges WHERE file_hash = ?1",
        )?;
        let rows = stmt.query_map(params![file_hash], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("list historical edge keys")
    }

    pub fn list_historical_edges_for_hash(&self, file_hash: &str) -> Result<Vec<HistoricalEdge>> {
        let mut stmt = self.conn.prepare(
            "SELECT file_hash, source_qn, target_qn, kind, file_path,
                    line, confidence, confidence_tier, extra_json
             FROM historical_edges
             WHERE file_hash = ?1
             ORDER BY file_path ASC, source_qn ASC, target_qn ASC, kind ASC",
        )?;
        let rows = stmt.query_map(params![file_hash], |r| {
            Ok(HistoricalEdge {
                file_hash: r.get(0)?,
                source_qn: r.get(1)?,
                target_qn: r.get(2)?,
                kind: r.get(3)?,
                file_path: r.get(4)?,
                line: r.get(5)?,
                confidence: r.get(6)?,
                confidence_tier: r.get(7)?,
                extra_json: r.get(8)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("list historical edges for hash")
    }

    pub fn reconstruct_snapshot_nodes(&self, snapshot_id: i64) -> Result<Vec<HistoricalNode>> {
        let mut stmt = self.conn.prepare(
            "SELECT hn.file_hash, hn.qualified_name, hn.kind, hn.name, hn.file_path,
                    hn.line_start, hn.line_end, hn.language, hn.parent_name,
                    hn.params, hn.return_type, hn.modifiers, hn.is_test, hn.extra_json
             FROM snapshot_nodes sn
             JOIN historical_nodes hn
               ON hn.file_hash = sn.file_hash
              AND hn.qualified_name = sn.qualified_name
             WHERE sn.snapshot_id = ?1
             ORDER BY hn.file_path ASC, hn.qualified_name ASC",
        )?;
        let rows = stmt.query_map(params![snapshot_id], |r| {
            Ok(HistoricalNode {
                file_hash: r.get(0)?,
                qualified_name: r.get(1)?,
                kind: r.get(2)?,
                name: r.get(3)?,
                file_path: r.get(4)?,
                line_start: r.get(5)?,
                line_end: r.get(6)?,
                language: r.get(7)?,
                parent_name: r.get(8)?,
                params: r.get(9)?,
                return_type: r.get(10)?,
                modifiers: r.get(11)?,
                is_test: r.get::<_, i64>(12)? != 0,
                extra_json: r.get(13)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("reconstruct snapshot nodes")
    }

    pub fn reconstruct_snapshot_edges(&self, snapshot_id: i64) -> Result<Vec<HistoricalEdge>> {
        let mut stmt = self.conn.prepare(
            "SELECT he.file_hash, he.source_qn, he.target_qn, he.kind, he.file_path,
                    he.line, he.confidence, he.confidence_tier, he.extra_json
             FROM snapshot_edges se
             JOIN historical_edges he
               ON he.file_hash = se.file_hash
              AND he.source_qn = se.source_qn
              AND he.target_qn = se.target_qn
              AND he.kind = se.kind
             WHERE se.snapshot_id = ?1
             ORDER BY he.file_path ASC, he.source_qn ASC, he.target_qn ASC, he.kind ASC",
        )?;
        let rows = stmt.query_map(params![snapshot_id], |r| {
            Ok(HistoricalEdge {
                file_hash: r.get(0)?,
                source_qn: r.get(1)?,
                target_qn: r.get(2)?,
                kind: r.get(3)?,
                file_path: r.get(4)?,
                line: r.get(5)?,
                confidence: r.get(6)?,
                confidence_tier: r.get(7)?,
                extra_json: r.get(8)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .context("reconstruct snapshot edges")
    }
}
