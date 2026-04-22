use atlas_core::{AtlasError, Community, CommunityNode, Flow, FlowMembership, Result};
use rusqlite::params;

use super::Store;

fn row_to_flow(row: &rusqlite::Row<'_>) -> rusqlite::Result<Flow> {
    let extra_str: Option<String> = row.get(4)?;
    let extra_json = extra_str
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::Value::Null);
    Ok(Flow {
        id: row.get(0)?,
        name: row.get(1)?,
        kind: row.get(2)?,
        description: row.get(3)?,
        extra_json,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
    })
}

fn row_to_flow_membership(row: &rusqlite::Row<'_>) -> rusqlite::Result<FlowMembership> {
    let extra_str: Option<String> = row.get(4)?;
    let extra_json = extra_str
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::Value::Null);
    Ok(FlowMembership {
        flow_id: row.get(0)?,
        node_qualified_name: row.get(1)?,
        position: row.get(2)?,
        role: row.get(3)?,
        extra_json,
    })
}

fn row_to_community(row: &rusqlite::Row<'_>) -> rusqlite::Result<Community> {
    let extra_str: Option<String> = row.get(5)?;
    let extra_json = extra_str
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::Value::Null);
    Ok(Community {
        id: row.get(0)?,
        name: row.get(1)?,
        algorithm: row.get(2)?,
        level: row.get(3)?,
        parent_community_id: row.get(4)?,
        extra_json,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

impl Store {
    // --- Flows ---------------------------------------------------------------

    /// Create a new flow and return its database id.
    pub fn create_flow(
        &self,
        name: &str,
        kind: Option<&str>,
        description: Option<&str>,
    ) -> Result<i64> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        self.conn
            .execute(
                "INSERT INTO flows (name, kind, description)
                 VALUES (?1, ?2, ?3)",
                params![name, kind, description],
            )
            .map_err(db_err)?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Delete a flow and all its memberships (cascade).
    pub fn delete_flow(&self, flow_id: i64) -> Result<()> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        self.conn
            .execute("DELETE FROM flows WHERE id = ?1", params![flow_id])
            .map_err(db_err)?;
        Ok(())
    }

    /// Return all flows ordered by name.
    pub fn list_flows(&self) -> Result<Vec<Flow>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, kind, description, extra_json, created_at, updated_at
                 FROM flows ORDER BY name",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], row_to_flow)
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Return a single flow by id, or `None` if not found.
    pub fn get_flow(&self, flow_id: i64) -> Result<Option<Flow>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, kind, description, extra_json, created_at, updated_at
                 FROM flows WHERE id = ?1",
            )
            .map_err(db_err)?;
        let mut rows = stmt
            .query_map(params![flow_id], row_to_flow)
            .map_err(db_err)?;
        Ok(rows.next().and_then(|r| r.ok()))
    }

    /// Return a single flow by name, or `None` if not found.
    pub fn get_flow_by_name(&self, name: &str) -> Result<Option<Flow>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, kind, description, extra_json, created_at, updated_at
                 FROM flows WHERE name = ?1",
            )
            .map_err(db_err)?;
        let mut rows = stmt.query_map(params![name], row_to_flow).map_err(db_err)?;
        Ok(rows.next().and_then(|r| r.ok()))
    }

    /// Add a node to a flow.  `position` and `role` are optional metadata.
    ///
    /// Uses `INSERT OR REPLACE` so re-adding an existing member updates its
    /// position/role rather than failing.
    pub fn add_flow_member(
        &self,
        flow_id: i64,
        node_qn: &str,
        position: Option<i64>,
        role: Option<&str>,
    ) -> Result<()> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        self.conn
            .execute(
                "INSERT OR REPLACE INTO flow_memberships
                     (flow_id, node_qualified_name, position, role)
                 VALUES (?1, ?2, ?3, ?4)",
                params![flow_id, node_qn, position, role],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Remove a node from a flow.  No-op if the membership did not exist.
    pub fn remove_flow_member(&self, flow_id: i64, node_qn: &str) -> Result<()> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        self.conn
            .execute(
                "DELETE FROM flow_memberships
                 WHERE flow_id = ?1 AND node_qualified_name = ?2",
                params![flow_id, node_qn],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Return all members of a flow ordered by position then qualified name.
    pub fn get_flow_members(&self, flow_id: i64) -> Result<Vec<FlowMembership>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT flow_id, node_qualified_name, position, role, extra_json
                 FROM flow_memberships
                 WHERE flow_id = ?1
                 ORDER BY position ASC NULLS LAST, node_qualified_name ASC",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![flow_id], row_to_flow_membership)
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Return all flows that include `node_qn` as a member.
    pub fn flows_for_node(&self, node_qn: &str) -> Result<Vec<Flow>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT f.id, f.name, f.kind, f.description, f.extra_json,
                        f.created_at, f.updated_at
                 FROM flows f
                 JOIN flow_memberships fm ON fm.flow_id = f.id
                 WHERE fm.node_qualified_name = ?1
                 ORDER BY f.name",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![node_qn], row_to_flow)
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    // --- Communities ---------------------------------------------------------

    /// Create a new community and return its database id.
    pub fn create_community(
        &self,
        name: &str,
        algorithm: Option<&str>,
        level: Option<i64>,
        parent_id: Option<i64>,
    ) -> Result<i64> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        self.conn
            .execute(
                "INSERT INTO communities (name, algorithm, level, parent_community_id)
                 VALUES (?1, ?2, ?3, ?4)",
                params![name, algorithm, level, parent_id],
            )
            .map_err(db_err)?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Delete a community and all its node memberships (cascade).
    pub fn delete_community(&self, community_id: i64) -> Result<()> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        self.conn
            .execute(
                "DELETE FROM communities WHERE id = ?1",
                params![community_id],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Return all communities ordered by name.
    pub fn list_communities(&self) -> Result<Vec<Community>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, algorithm, level, parent_community_id, extra_json,
                        created_at, updated_at
                 FROM communities ORDER BY name",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], row_to_community)
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Return a single community by id, or `None` if not found.
    pub fn get_community(&self, community_id: i64) -> Result<Option<Community>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, algorithm, level, parent_community_id, extra_json,
                        created_at, updated_at
                 FROM communities WHERE id = ?1",
            )
            .map_err(db_err)?;
        let mut rows = stmt
            .query_map(params![community_id], row_to_community)
            .map_err(db_err)?;
        Ok(rows.next().and_then(|r| r.ok()))
    }

    /// Return a single community by name, or `None` if not found.
    pub fn get_community_by_name(&self, name: &str) -> Result<Option<Community>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, name, algorithm, level, parent_community_id, extra_json,
                        created_at, updated_at
                 FROM communities WHERE name = ?1",
            )
            .map_err(db_err)?;
        let mut rows = stmt
            .query_map(params![name], row_to_community)
            .map_err(db_err)?;
        Ok(rows.next().and_then(|r| r.ok()))
    }

    /// Add a node to a community.  No-op if already a member.
    pub fn add_community_node(&self, community_id: i64, node_qn: &str) -> Result<()> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        self.conn
            .execute(
                "INSERT OR IGNORE INTO community_nodes (community_id, node_qualified_name)
                 VALUES (?1, ?2)",
                params![community_id, node_qn],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Remove a node from a community.  No-op if not a member.
    pub fn remove_community_node(&self, community_id: i64, node_qn: &str) -> Result<()> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        self.conn
            .execute(
                "DELETE FROM community_nodes
                 WHERE community_id = ?1 AND node_qualified_name = ?2",
                params![community_id, node_qn],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Return all node qualified names belonging to a community.
    pub fn get_community_nodes(&self, community_id: i64) -> Result<Vec<CommunityNode>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT community_id, node_qualified_name
                 FROM community_nodes
                 WHERE community_id = ?1
                 ORDER BY node_qualified_name ASC",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![community_id], |row| {
                Ok(CommunityNode {
                    community_id: row.get(0)?,
                    node_qualified_name: row.get(1)?,
                })
            })
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Return all communities that include `node_qn` as a member.
    pub fn communities_for_node(&self, node_qn: &str) -> Result<Vec<Community>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT c.id, c.name, c.algorithm, c.level, c.parent_community_id,
                        c.extra_json, c.created_at, c.updated_at
                 FROM communities c
                 JOIN community_nodes cn ON cn.community_id = c.id
                 WHERE cn.node_qualified_name = ?1
                 ORDER BY c.name",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![node_qn], row_to_community)
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }
}
