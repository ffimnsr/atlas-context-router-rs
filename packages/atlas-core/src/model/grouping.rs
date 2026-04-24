use serde::{Deserialize, Serialize};

/// A named ordered sequence of graph nodes.
///
/// Flows represent user-defined traversal paths or call chains. Membership
/// is stored with soft references so it survives graph rebuilds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Flow {
    pub id: i64,
    pub name: String,
    pub kind: Option<String>,
    pub description: Option<String>,
    pub extra_json: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

/// One node's participation in a flow with optional ordering and role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowMembership {
    pub flow_id: i64,
    /// Qualified name of the node. Soft reference — survives node rebuild.
    pub node_qualified_name: String,
    pub position: Option<i64>,
    pub role: Option<String>,
    pub extra_json: serde_json::Value,
}

/// A named partition of graph nodes produced by a clustering algorithm.
///
/// Communities may nest: `parent_community_id` links child to parent.
/// The actual node members are stored in the `community_nodes` join table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Community {
    pub id: i64,
    pub name: String,
    pub algorithm: Option<String>,
    pub level: Option<i64>,
    pub parent_community_id: Option<i64>,
    pub extra_json: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

/// One node's membership in a community. Soft reference on qualified name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityNode {
    pub community_id: i64,
    /// Qualified name of the node. Soft reference — survives node rebuild.
    pub node_qualified_name: String,
}
