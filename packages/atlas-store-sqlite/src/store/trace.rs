//! Bounded call-chain tracing over the graph.
//!
//! Unlike `traverse_from_qnames`, tracing is directional and edge-kind
//! filtered. Every hop joins `nodes`, so expression-level call targets that
//! never became graph nodes are dropped instead of polluting the chain.

use std::collections::{HashMap, HashSet, VecDeque};
use std::str::FromStr;

use anyhow::Result;
use atlas_core::{Edge, EdgeKind, Node};
use rusqlite::params;

use super::Store;
use super::helpers::{row_to_edge_at, row_to_node_at};

/// Number of projected node columns in a trace row.
const NODE_COLUMNS: usize = 15;

/// Edge direction followed while tracing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceDirection {
    /// Follow incoming edges: who calls the symbol.
    Up,
    /// Follow outgoing edges: what the symbol calls.
    Down,
    /// Follow both directions.
    Both,
}

impl TraceDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Down => "down",
            Self::Both => "both",
        }
    }
}

impl FromStr for TraceDirection {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "up" | "in" | "inbound" | "callers" => Ok(Self::Up),
            "down" | "out" | "outbound" | "callees" => Ok(Self::Down),
            "both" | "all" | "bidirectional" => Ok(Self::Both),
            other => Err(format!(
                "unknown trace direction: {other} (expected up, down, or both)"
            )),
        }
    }
}

/// Bounded trace configuration.
#[derive(Debug, Clone)]
pub struct TraceOptions<'a> {
    pub direction: TraceDirection,
    pub edge_kinds: &'a [EdgeKind],
    pub max_depth: u32,
    pub max_nodes: usize,
}

impl<'a> TraceOptions<'a> {
    pub fn new(direction: TraceDirection, edge_kinds: &'a [EdgeKind]) -> Self {
        Self {
            direction,
            edge_kinds,
            max_depth: 3,
            max_nodes: 200,
        }
    }
}

/// One discovered symbol with the hop that reached it.
#[derive(Debug, Clone)]
pub struct TraceNode {
    pub node: Node,
    pub depth: u32,
    /// `seed`, `outbound`, or `inbound` relative to the parent hop.
    pub direction: &'static str,
    pub via_edge: Option<Edge>,
}

/// Result of a bounded directional trace.
#[derive(Debug, Clone)]
pub struct TraceResult {
    pub root: Node,
    pub nodes: Vec<TraceNode>,
    pub edges: Vec<Edge>,
    pub truncated: bool,
    pub max_depth_reached: u32,
}

/// Shortest path between two symbols.
#[derive(Debug, Clone)]
pub struct TracePath {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

/// Result of a shortest-path trace, including the no-path case.
#[derive(Debug, Clone)]
pub struct TracePathResult {
    pub from: Node,
    pub to: Node,
    pub path: Option<TracePath>,
    pub visited_nodes: usize,
    pub truncated: bool,
}

impl Store {
    /// Directional trace from `root_qn`.
    ///
    /// Returns `Ok(None)` when the root symbol does not exist.
    pub fn trace_calls(
        &self,
        root_qn: &str,
        opts: &TraceOptions<'_>,
    ) -> Result<Option<TraceResult>> {
        let Some(root) = self.node_by_qname(root_qn)? else {
            return Ok(None);
        };

        let mut visited: HashSet<String> = HashSet::from([root.qualified_name.clone()]);
        let mut nodes = vec![TraceNode {
            node: root.clone(),
            depth: 0,
            direction: "seed",
            via_edge: None,
        }];
        let mut edges: Vec<Edge> = Vec::new();
        let mut edge_ids: HashSet<i64> = HashSet::new();
        let mut frontier = vec![(root.clone(), 0u32)];
        let mut truncated = false;
        let mut max_depth_reached = 0u32;

        'bfs: while !frontier.is_empty() {
            let mut next_frontier = Vec::new();
            for (node, depth) in std::mem::take(&mut frontier) {
                if depth >= opts.max_depth {
                    continue;
                }
                for (next_node, edge) in
                    self.trace_neighbors(&node.qualified_name, opts, depth + 1)?
                {
                    if edge_ids.insert(edge.id) {
                        edges.push(edge.clone());
                    }
                    if !visited.insert(next_node.qualified_name.clone()) {
                        continue;
                    }
                    if nodes.len() >= opts.max_nodes {
                        truncated = true;
                        break 'bfs;
                    }
                    let direction = if edge.source_qn == node.qualified_name {
                        "outbound"
                    } else {
                        "inbound"
                    };
                    max_depth_reached = max_depth_reached.max(depth + 1);
                    nodes.push(TraceNode {
                        node: next_node.clone(),
                        depth: depth + 1,
                        direction,
                        via_edge: Some(edge),
                    });
                    next_frontier.push((next_node, depth + 1));
                }
            }
            frontier = next_frontier;
        }

        Ok(Some(TraceResult {
            root,
            nodes,
            edges,
            truncated,
            max_depth_reached,
        }))
    }

    /// Shortest path from `from_qn` to `to_qn` under the given direction and
    /// edge-kind filter.
    ///
    /// Returns `Ok(None)` when either endpoint is missing.
    pub fn trace_call_path(
        &self,
        from_qn: &str,
        to_qn: &str,
        opts: &TraceOptions<'_>,
    ) -> Result<Option<TracePathResult>> {
        let Some(from) = self.node_by_qname(from_qn)? else {
            return Ok(None);
        };
        let Some(to) = self.node_by_qname(to_qn)? else {
            return Ok(None);
        };

        if from.qualified_name == to.qualified_name {
            return Ok(Some(TracePathResult {
                from: from.clone(),
                to,
                path: Some(TracePath {
                    nodes: vec![from],
                    edges: Vec::new(),
                }),
                visited_nodes: 1,
                truncated: false,
            }));
        }

        let mut parents: HashMap<String, (String, Edge)> = HashMap::new();
        let mut nodes_by_qn: HashMap<String, Node> =
            HashMap::from([(from.qualified_name.clone(), from.clone())]);
        let mut visited: HashSet<String> = HashSet::from([from.qualified_name.clone()]);
        let mut queue: VecDeque<(Node, u32)> = VecDeque::from([(from.clone(), 0)]);
        let mut truncated = false;

        'search: while let Some((node, depth)) = queue.pop_front() {
            if depth >= opts.max_depth {
                continue;
            }
            for (next_node, edge) in self.trace_neighbors(&node.qualified_name, opts, depth + 1)? {
                if !visited.insert(next_node.qualified_name.clone()) {
                    continue;
                }
                if visited.len() > opts.max_nodes {
                    truncated = true;
                    break 'search;
                }
                parents.insert(
                    next_node.qualified_name.clone(),
                    (node.qualified_name.clone(), edge),
                );
                nodes_by_qn.insert(next_node.qualified_name.clone(), next_node.clone());
                if next_node.qualified_name == to.qualified_name {
                    let path = reconstruct_path(
                        &from.qualified_name,
                        &to.qualified_name,
                        &parents,
                        &nodes_by_qn,
                    );
                    return Ok(Some(TracePathResult {
                        from,
                        to,
                        path: Some(path),
                        visited_nodes: visited.len(),
                        truncated: false,
                    }));
                }
                queue.push_back((next_node, depth + 1));
            }
        }

        Ok(Some(TracePathResult {
            from,
            to,
            path: None,
            visited_nodes: visited.len(),
            truncated,
        }))
    }

    fn trace_neighbors(
        &self,
        qn: &str,
        opts: &TraceOptions<'_>,
        next_depth: u32,
    ) -> Result<Vec<(Node, Edge)>> {
        if opts.edge_kinds.is_empty() || next_depth > opts.max_depth {
            return Ok(Vec::new());
        }
        let limit = opts.max_nodes.max(1);
        let mut out = Vec::new();
        if matches!(opts.direction, TraceDirection::Down | TraceDirection::Both) {
            out.extend(self.trace_edges(qn, true, opts.edge_kinds, limit)?);
        }
        if matches!(opts.direction, TraceDirection::Up | TraceDirection::Both) {
            out.extend(self.trace_edges(qn, false, opts.edge_kinds, limit)?);
        }
        Ok(out)
    }

    fn trace_edges(
        &self,
        qn: &str,
        outbound: bool,
        edge_kinds: &[EdgeKind],
        limit: usize,
    ) -> Result<Vec<(Node, Edge)>> {
        if edge_kinds.is_empty() {
            return Ok(Vec::new());
        }
        let kind_list = edge_kinds
            .iter()
            .map(|kind| format!("'{}'", kind.as_str()))
            .collect::<Vec<_>>()
            .join(",");
        let (join, filter) = if outbound {
            (
                "n.qualified_name = e.target_qualified",
                "e.source_qualified = ?1",
            )
        } else {
            (
                "n.qualified_name = e.source_qualified",
                "e.target_qualified = ?1",
            )
        };
        let sql = format!(
            "SELECT n.id, n.kind, n.name, n.qualified_name, n.file_path, n.line_start, n.line_end,
                    n.language, n.parent_name, n.params, n.return_type, n.modifiers, n.is_test,
                    n.file_hash, n.extra_json,
                    e.id, e.kind, e.source_qualified, e.target_qualified, e.file_path, e.line,
                    e.confidence, e.confidence_tier, e.extra_json
             FROM edges e
             JOIN nodes n ON {join}
             WHERE {filter}
               AND e.kind IN ({kind_list})
               AND n.qualified_name <> ?1
             ORDER BY e.id
             LIMIT ?2"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![qn, limit as i64], |row| {
            Ok((row_to_node_at(row, 0)?, row_to_edge_at(row, NODE_COLUMNS)?))
        })?;
        Ok(rows.filter_map(|row| row.ok()).collect())
    }
}

fn reconstruct_path(
    from_qn: &str,
    to_qn: &str,
    parents: &HashMap<String, (String, Edge)>,
    nodes_by_qn: &HashMap<String, Node>,
) -> TracePath {
    let mut chain: Vec<(Node, Edge)> = Vec::new();
    let mut current = to_qn.to_owned();
    while current != from_qn {
        let Some((parent_qn, edge)) = parents.get(&current).cloned() else {
            break;
        };
        let Some(node) = nodes_by_qn.get(&current).cloned() else {
            break;
        };
        chain.push((node, edge));
        current = parent_qn;
    }
    chain.reverse();

    let mut nodes = vec![
        nodes_by_qn
            .get(from_qn)
            .cloned()
            .expect("path root must be recorded"),
    ];
    let mut edges = Vec::with_capacity(chain.len());
    for (node, edge) in chain {
        edges.push(edge);
        nodes.push(node);
    }
    TracePath { nodes, edges }
}
