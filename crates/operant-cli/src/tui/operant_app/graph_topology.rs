// Vendored from jcode (crates/operant-tui-core/src/graph_topology.rs), MIT
// License, Copyright (c) 2025 Jeremy Huang. Ported verbatim @ 0a9dc7805.
//! Compatibility surface for the memory-graph topology helpers the info
//! widget renders. Only the parts the ported widget calls are here; see the
//! per-symbol port-decision on `build_graph_topology`.

#[derive(Debug, Clone)]
pub struct GraphNode {
    /// Stable node ID from memory graph (mem:*, tag:*, cluster:*)
    pub id: String,
    /// Human-readable display label
    pub label: String,
    /// Category: "fact", "preference", "correction", "tag"
    pub kind: String,
    /// Whether this node is a memory (vs tag/cluster)
    pub is_memory: bool,
    /// Whether this node is active (superseded memories are inactive)
    pub is_active: bool,
    /// Effective confidence score (0.0-1.0)
    pub confidence: f32,
    /// Number of connections (degree)
    pub degree: usize,
}

#[derive(Debug, Clone)]
pub struct GraphEdge {
    /// Source index into MemoryInfo::graph_nodes
    pub source: usize,
    /// Target index into MemoryInfo::graph_nodes
    pub target: usize,
    /// Edge kind (has_tag, supersedes, contradicts, ...)
    pub kind: String,
}

pub fn graph_node_score(node: &GraphNode) -> f32 {
    let memory_bias = if node.is_memory { 2.0 } else { 0.0 };
    let active_bias = if node.is_active { 1.0 } else { 0.0 };
    node.degree as f32 + memory_bias + active_bias + node.confidence * 2.0
}

/// [port-decision] upstream `build_graph_topology` (:63) walks
/// `operant_memory_types::MemoryGraph` (`graph.memories` / `graph.tags` /
/// `graph.clusters` / `graph.edges`, with `MemoryEntry::effective_confidence`
/// and `EdgeKind`) to produce the node/edge vectors. None of that memory-graph
/// type is ported into operant (no `MemoryGraph`/`EdgeKind` anywhere under
/// `crates/`), and no ported call site invokes this function — it is reached
/// only through the `info_widget` re-export at info_widget/mod.rs:80. Its body
/// is therefore not ported; the signature is kept so the re-export stays
/// whole. Re-activate at W-memory when the memory-graph types land: port
/// `truncate_chars` (:27), `truncate_smart` (:33), `collect_memory_nodes`
/// (:92), `collect_tag_nodes` (:114), `collect_cluster_nodes` (:140),
/// `collect_edges` (:180), `bound_topology_size` (:236) and `edge_kind_name`
/// (:281) verbatim alongside them.
pub fn build_graph_topology() -> (Vec<GraphNode>, Vec<GraphEdge>) {
    (Vec::new(), Vec::new())
}
