//! Learning Graph — skills + memory as first-class nodes with edges.
//!
//! Ported from `hermes-agent/agent/learning_graph.py` and
//! `hermes-agent/agent/learning_mutations.py`.
//!
//! The learning graph makes self-learning *visible*: learned skills and
//! memory chunks become graph nodes connected by edges derived from
//! lexical overlap and declared `related_skills`. This powers the
//! `/journey` overlay in the TUI.
//!
//! ## Node Types
//!
//! - **Skill nodes**: skills that are NOT base-installed and show real
//!   learning signal (agent-created or used). Each node carries metadata
//!   like category, source, use_count, state, created_by, pinned.
//! - **Memory nodes**: memory chunks from MEMORY.md / USER.md, split on
//!   `§` separators. Each chunk becomes one node.
//!
//! ## Edge Types
//!
//! - **Skill↔Skill edges**: connect skills that share the same category
//!   field. Both endpoints must exist; edges are deduped.
//! - **Memory↔Skill edges**: from lexical overlap between memory card
//!   text and skill names. Top 4 matches per memory card.
//!
//! ## Mutations
//!
//! User-initiated edit/delete operations for graph nodes:
//! - Delete a skill → archive it (recoverable via curator restore)
//! - Delete a memory → rewrite the source file
//! - Edit a skill → rewrite SKILL.md
//! - Edit a memory → rewrite the source file chunk

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

// ---------------------------------------------------------------------------
// Node types
// ---------------------------------------------------------------------------

/// A node in the learning graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    /// Stable ID. Skills use the skill name; memories use
    /// `memory:<source>:<index>`.
    pub id: String,
    /// Human-readable label.
    pub label: String,
    /// Node kind: "skill" or "memory".
    pub kind: NodeKind,
    /// Unix timestamp of last activity or file mtime.
    pub timestamp: Option<i64>,
    /// Category (e.g. "research", "memory").
    pub category: String,
    /// How many times this skill has been used.
    pub use_count: u32,
    /// "active", "archived", etc.
    pub state: String,
    /// Who created this node: "agent", "user", "memory".
    pub created_by: String,
    /// Whether this node is pinned (blocks archive/delete).
    pub pinned: bool,
    /// For memory nodes: "memory" or "profile".
    pub memory_source: Option<String>,
}

/// Kind of graph node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Skill,
    Memory,
}

// ---------------------------------------------------------------------------
// Edge type
// ---------------------------------------------------------------------------

/// An edge between two graph nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
}

// ---------------------------------------------------------------------------
// Graph structure
// ---------------------------------------------------------------------------

/// The full learning graph payload, suitable for TUI rendering.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub clusters: Vec<ClusterInfo>,
    pub stats: GraphStats,
}

/// Category cluster with count.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterInfo {
    pub category: String,
    pub count: usize,
}

/// Aggregate statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphStats {
    pub total_nodes: usize,
    pub skill_nodes: usize,
    pub memory_nodes: usize,
    pub total_edges: usize,
    pub edges_per_node: f64,
    pub linked_nodes: usize,
    pub isolated_pct: f64,
    pub agent_created: usize,
    pub used: usize,
}

// ---------------------------------------------------------------------------
// Mutation operations
// ---------------------------------------------------------------------------

/// Result of a mutation operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MutationResult {
    pub ok: bool,
    pub message: String,
}

/// Delete a node from the learning graph.
///
/// - Skills: archive the skill directory (recoverable via curator restore).
/// - Memories: remove the chunk from the source file.
pub fn delete_node(node_id: &str, skills_dir: &Path, memory_dir: &Path) -> MutationResult {
    if node_id.starts_with("memory:") {
        delete_memory_node(node_id, memory_dir)
    } else {
        delete_skill_node(node_id, skills_dir)
    }
}

/// Edit a node's content.
///
/// - Skills: rewrite the SKILL.md file.
/// - Memories: rewrite the specific chunk in the source file.
pub fn edit_node(
    node_id: &str,
    content: &str,
    skills_dir: &Path,
    memory_dir: &Path,
) -> MutationResult {
    if node_id.starts_with("memory:") {
        edit_memory_node(node_id, content, memory_dir)
    } else {
        edit_skill_node(node_id, content, skills_dir)
    }
}

// ---------------------------------------------------------------------------
// Skill mutations
// ---------------------------------------------------------------------------

fn delete_skill_node(name: &str, skills_dir: &Path) -> MutationResult {
    let skill_dir = skills_dir.join(name);
    if !skill_dir.exists() {
        return MutationResult {
            ok: false,
            message: format!("Skill '{}' not found", name),
        };
    }

    // Check if pinned — reuse frontmatter parser
    let skill_md = skill_dir.join("SKILL.md");
    if skill_md.exists() {
        let (_, _, _, pinned, _) = parse_skill_frontmatter(&skill_md);
        if pinned {
            return MutationResult {
                ok: false,
                message: format!("'{}' is pinned — unpin it first before archiving", name),
            };
        }
    }

    // Archive: move to .archive/ subdirectory
    let archive_dir = skills_dir.join(".archive");
    if let Err(e) = std::fs::create_dir_all(&archive_dir) {
        return MutationResult {
            ok: false,
            message: format!("Failed to create archive directory: {}", e),
        };
    }

    let archive_target = archive_dir.join(name);
    match std::fs::rename(&skill_dir, &archive_target) {
        Ok(()) => MutationResult {
            ok: true,
            message: format!(
                "Archived '{}' — restore with: operant curator restore {}",
                name, name
            ),
        },
        Err(e) => MutationResult {
            ok: false,
            message: format!("Failed to archive '{}': {}", name, e),
        },
    }
}

fn edit_skill_node(name: &str, content: &str, skills_dir: &Path) -> MutationResult {
    let skill_md = skills_dir.join(name).join("SKILL.md");
    if !skill_md.exists() {
        return MutationResult {
            ok: false,
            message: format!("Skill '{}' not found (no SKILL.md)", name),
        };
    }

    match std::fs::write(&skill_md, content) {
        Ok(()) => MutationResult {
            ok: true,
            message: format!("Updated skill '{}'", name),
        },
        Err(e) => MutationResult {
            ok: false,
            message: format!("Failed to update '{}': {}", name, e),
        },
    }
}

// ---------------------------------------------------------------------------
// Memory mutations
// ---------------------------------------------------------------------------

/// Parse a memory node ID like `memory:memory:0` or `memory:profile:2`
/// into (source, local_index).
fn parse_memory_id(node_id: &str) -> Option<(&str, usize)> {
    let rest = node_id.strip_prefix("memory:")?;
    let parts: Vec<&str> = rest.splitn(2, ':').collect();
    if parts.len() != 2 {
        return None;
    }
    let source = parts[0]; // "memory" or "profile"
    let index: usize = parts[1].parse().ok()?;
    Some((source, index))
}

fn memory_file_name(source: &str) -> Option<&'static str> {
    match source {
        "memory" => Some("MEMORY.md"),
        "profile" => Some("USER.md"),
        _ => None,
    }
}

/// Read memory chunks from a file, split on `§` separators.
fn read_memory_chunks(path: &Path) -> Vec<String> {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content
        .split("\n§\n")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Write memory chunks back to a file, joined by `\n§\n`.
fn write_memory_chunks(path: &Path, chunks: &[String]) -> Result<(), String> {
    let content = chunks.join("\n§\n");
    std::fs::write(path, content + "\n")
        .map_err(|e| format!("Failed to write {}: {}", path.display(), e))
}

fn delete_memory_node(node_id: &str, memory_dir: &Path) -> MutationResult {
    let (source, index) = match parse_memory_id(node_id) {
        Some(v) => v,
        None => {
            return MutationResult {
                ok: false,
                message: format!("Invalid memory node ID: {}", node_id),
            };
        }
    };

    let file_name = match memory_file_name(source) {
        Some(f) => f,
        None => {
            return MutationResult {
                ok: false,
                message: format!("Unknown memory source: {}", source),
            };
        }
    };

    let path = memory_dir.join(file_name);
    if !path.exists() {
        return MutationResult {
            ok: false,
            message: format!("{} not found", file_name),
        };
    }

    let mut chunks = read_memory_chunks(&path);
    if index >= chunks.len() {
        return MutationResult {
            ok: false,
            message: format!(
                "Memory index {} out of range ({} chunks in {})",
                index,
                chunks.len(),
                file_name
            ),
        };
    }

    chunks.remove(index);
    match write_memory_chunks(&path, &chunks) {
        Ok(()) => MutationResult {
            ok: true,
            message: format!("Deleted memory from {}", file_name),
        },
        Err(e) => MutationResult {
            ok: false,
            message: e,
        },
    }
}

fn edit_memory_node(node_id: &str, content: &str, memory_dir: &Path) -> MutationResult {
    let (source, index) = match parse_memory_id(node_id) {
        Some(v) => v,
        None => {
            return MutationResult {
                ok: false,
                message: format!("Invalid memory node ID: {}", node_id),
            };
        }
    };

    let body = content.trim();
    if body.is_empty() {
        return MutationResult {
            ok: false,
            message: "Empty memory — use delete to remove it".to_string(),
        };
    }

    let file_name = match memory_file_name(source) {
        Some(f) => f,
        None => {
            return MutationResult {
                ok: false,
                message: format!("Unknown memory source: {}", source),
            };
        }
    };

    let path = memory_dir.join(file_name);
    if !path.exists() {
        return MutationResult {
            ok: false,
            message: format!("{} not found", file_name),
        };
    }

    let mut chunks = read_memory_chunks(&path);
    if index >= chunks.len() {
        return MutationResult {
            ok: false,
            message: format!(
                "Memory index {} out of range ({} chunks in {})",
                index,
                chunks.len(),
                file_name
            ),
        };
    }

    chunks[index] = body.to_string();
    match write_memory_chunks(&path, &chunks) {
        Ok(()) => MutationResult {
            ok: true,
            message: format!("Updated memory in {}", file_name),
        },
        Err(e) => MutationResult {
            ok: false,
            message: e,
        },
    }
}

// ---------------------------------------------------------------------------
// Graph building (lightweight TUI-friendly)
// ---------------------------------------------------------------------------

/// Build the learning graph from skills and memory directories.
///
/// This is a simplified version of hermes-agent's `build_learning_graph()`.
/// It scans SKILL.md files for metadata and MEMORY.md/USER.md for memory
/// chunks, then connects them via lexical overlap.
pub fn build_learning_graph(skills_dir: &Path, memory_dir: &Path) -> LearningGraph {
    let mut nodes: Vec<GraphNode> = Vec::new();
    let mut edges: Vec<GraphEdge> = Vec::new();
    let mut categories: HashMap<String, usize> = HashMap::new();
    let mut skill_related: HashMap<String, Vec<String>> = HashMap::new();

    // ── Skill nodes ────────────────────────────────────────────────
    if skills_dir.exists() {
        for entry in std::fs::read_dir(skills_dir).into_iter().flatten() {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            // Skip hidden dirs (.archive, .hub, etc.)
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.'))
            {
                continue;
            }

            let skill_md = path.join("SKILL.md");
            if !skill_md.exists() {
                continue;
            }

            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            let (category, created_by, use_count, pinned, related_skills) =
                parse_skill_frontmatter(&skill_md);

            let ts = skill_md
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64);

            *categories.entry(category.clone()).or_insert(0) += 1;

            // Store related_skills for edge construction
            if !related_skills.is_empty() {
                skill_related.insert(name.clone(), related_skills);
            }

            nodes.push(GraphNode {
                id: name.clone(),
                label: name.clone(),
                kind: NodeKind::Skill,
                timestamp: ts,
                category,
                use_count,
                state: "active".to_string(),
                created_by,
                pinned,
                memory_source: None,
            });
        }
    }

    // ── Memory nodes ───────────────────────────────────────────────
    let mut memory_cards: Vec<(String, String, i64)> = Vec::new(); // (source, title, ts)
    for (source, file_name) in &[("memory", "MEMORY.md"), ("profile", "USER.md")] {
        let path = memory_dir.join(file_name);
        if !path.exists() {
            continue;
        }
        let ts = path
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        let chunks = read_memory_chunks(&path);
        for (idx, chunk) in chunks.iter().enumerate() {
            let first_line = chunk
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .trim_start_matches('#')
                .trim();
            let label = if first_line.len() > 80 {
                format!("{}…", &first_line[..80])
            } else {
                first_line.to_string()
            };
            let node_id = format!("memory:{}:{}", source, idx);
            memory_cards.push((source.to_string(), label.clone(), ts + idx as i64));

            *categories.entry("memory".to_string()).or_insert(0) += 1;

            nodes.push(GraphNode {
                id: node_id,
                label,
                kind: NodeKind::Memory,
                timestamp: Some(ts + idx as i64),
                category: "memory".to_string(),
                use_count: 0,
                state: "active".to_string(),
                created_by: "memory".to_string(),
                pinned: false,
                memory_source: Some(source.to_string()),
            });
        }
    }

    // ── Skill↔Skill edges (from declared related_skills) ──────────
    // Matches hermes-agent's pattern: connect skills via declared
    // related_skills in YAML frontmatter, with category fallback.
    let skill_nodes: Vec<GraphNode> = nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Skill)
        .cloned()
        .collect();
    let mut seen_edges: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();

    // Build a lookup from skill name to its related_skills list
    let mut related_map: HashMap<String, Vec<String>> = HashMap::new();
    for (name, related) in &skill_related {
        if !related.is_empty() {
            related_map.insert(name.clone(), related.clone());
        }
    }

    // Phase 1: Edges from declared related_skills (high-precision)
    for skill in &skill_nodes {
        if let Some(related) = related_map.get(&skill.id) {
            for target_name in related {
                // Only connect if the target skill exists
                if skill_nodes.iter().any(|s| s.id == *target_name) {
                    let key = if skill.id < *target_name {
                        (skill.id.clone(), target_name.clone())
                    } else {
                        (target_name.clone(), skill.id.clone())
                    };
                    if seen_edges.insert(key.clone()) {
                        edges.push(GraphEdge {
                            source: key.0,
                            target: key.1,
                        });
                    }
                }
            }
        }
    }

    // Phase 2: Category-based edges for skills without declared relations
    // Only connect skills that have no related_skills AND share a category.
    // This avoids overly dense graphs when related_skills are available.
    for skill in &skill_nodes {
        // Skip skills that already have declared relations
        if related_map.contains_key(&skill.id) {
            continue;
        }
        for other in &skill_nodes {
            if skill.id == other.id {
                continue;
            }
            // Skip other skills that already have declared relations
            if related_map.contains_key(&other.id) {
                continue;
            }
            if skill.category == other.category && skill.category != "general" {
                let key = if skill.id < other.id {
                    (skill.id.clone(), other.id.clone())
                } else {
                    (other.id.clone(), skill.id.clone())
                };
                if seen_edges.insert(key.clone()) {
                    edges.push(GraphEdge {
                        source: key.0,
                        target: key.1,
                    });
                }
            }
        }
    }

    // ── Memory↔Skill edges (lexical overlap) ──────────────────────
    for (mem_idx, (source, title, _)) in memory_cards.iter().enumerate() {
        let text_lower = title.to_lowercase();
        let mut scored: Vec<(u32, String)> = Vec::new();

        for skill in &skill_nodes {
            let skill_lower = skill.label.to_lowercase();
            let mut score = 0u32;
            if text_lower.contains(&skill_lower) {
                score += 6;
            }
            // Simple word overlap
            let skill_words: Vec<&str> = skill_lower.split_whitespace().collect();
            for word in &skill_words {
                if word.len() >= 3 && text_lower.contains(word) {
                    score += 1;
                }
            }
            if score > 0 {
                scored.push((score, skill.id.clone()));
            }
        }

        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        for (_, skill_name) in scored.iter().take(4) {
            let mem_id = format!("memory:{}:{}", source, mem_idx);
            let key = if mem_id < *skill_name {
                (mem_id.clone(), skill_name.clone())
            } else {
                (skill_name.clone(), mem_id)
            };
            if seen_edges.insert(key.clone()) {
                edges.push(GraphEdge {
                    source: key.0,
                    target: key.1,
                });
            }
        }
    }

    // ── Stats ──────────────────────────────────────────────────────
    let total_nodes = nodes.len();
    let skill_count = nodes.iter().filter(|n| n.kind == NodeKind::Skill).count();
    let memory_count = nodes.iter().filter(|n| n.kind == NodeKind::Memory).count();
    let linked: std::collections::HashSet<String> = edges
        .iter()
        .flat_map(|e| vec![e.source.clone(), e.target.clone()])
        .collect();
    let agent_created = nodes.iter().filter(|n| n.created_by == "agent").count();
    let used = nodes.iter().filter(|n| n.use_count > 0).count();

    let clusters: Vec<ClusterInfo> = categories
        .into_iter()
        .map(|(category, count)| ClusterInfo { category, count })
        .collect();

    let stats = GraphStats {
        total_nodes,
        skill_nodes: skill_count,
        memory_nodes: memory_count,
        total_edges: edges.len(),
        edges_per_node: if total_nodes > 0 {
            edges.len() as f64 / total_nodes as f64
        } else {
            0.0
        },
        linked_nodes: linked.len(),
        isolated_pct: if total_nodes > 0 {
            100.0 * (total_nodes - linked.len()) as f64 / total_nodes as f64
        } else {
            0.0
        },
        agent_created,
        used,
    };

    LearningGraph {
        nodes,
        edges,
        clusters,
        stats,
    }
}

/// Parse minimal frontmatter from a SKILL.md file.
/// Returns (category, created_by, use_count, pinned, related_skills).
fn parse_skill_frontmatter(path: &Path) -> (String, String, u32, bool, Vec<String>) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => {
            return (
                "general".to_string(),
                "user".to_string(),
                0,
                false,
                Vec::new(),
            );
        }
    };

    let mut category = "general".to_string();
    let mut created_by = "user".to_string();
    let mut pinned = false;
    let mut related_skills: Vec<String> = Vec::new();

    // Simple line-by-line parsing (YAML frontmatter between --- delimiters)
    let mut in_frontmatter = false;
    let mut last_key = ""; // Track last YAML key to handle multi-line lists robustly
    for line in content.lines().take(30) {
        let trimmed = line.trim();
        if trimmed == "---" {
            in_frontmatter = !in_frontmatter;
            last_key = "";
            continue;
        }
        if !in_frontmatter {
            break;
        }
        if let Some(val) = trimmed.strip_prefix("category:") {
            category = val.trim().to_string();
            last_key = "category";
        } else if let Some(val) = trimmed.strip_prefix("created_by:") {
            created_by = val.trim().to_string();
            last_key = "created_by";
        } else if trimmed == "pinned: true" || trimmed == "pinned:true" {
            pinned = true;
            last_key = "pinned";
        } else if let Some(val) = trimmed.strip_prefix("related_skills:") {
            let val = val.trim();
            last_key = "related_skills";
            if val.starts_with('[') && val.ends_with(']') {
                // Inline: related_skills: [skill-a, skill-b]
                let inner = &val[1..val.len() - 1];
                related_skills = inner
                    .split(',')
                    .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                last_key = ""; // List complete on this line
            } else if !val.is_empty() {
                // Single value on same line
                related_skills.push(val.trim_matches('"').trim_matches('\'').to_string());
                last_key = ""; // Single value, not a multi-line list
            }
            // else: multi-line list starts on next lines (last_key stays "related_skills")
            continue;
        } else if trimmed.starts_with('-') && last_key == "related_skills" {
            // Multi-line YAML list item (e.g. "  - skill-name")
            if let Some(val) = trimmed.strip_prefix('-') {
                let val = val.trim().trim_matches('"').trim_matches('\'').to_string();
                if !val.is_empty() {
                    related_skills.push(val);
                }
            }
            // Continue collecting list items; last_key stays "related_skills"
        } else if !trimmed.is_empty() && !trimmed.starts_with('#') {
            // Any other non-empty, non-comment line ends the current list context
            last_key = "";
        }
    }

    (category, created_by, 0, pinned, related_skills)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn tmp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("learning_graph_{}_{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn test_parse_memory_id() {
        assert_eq!(parse_memory_id("memory:memory:0"), Some(("memory", 0)));
        assert_eq!(parse_memory_id("memory:profile:3"), Some(("profile", 3)));
        assert_eq!(parse_memory_id("invalid"), None);
        assert_eq!(parse_memory_id("memory:x"), None);
    }

    #[test]
    fn test_memory_file_name() {
        assert_eq!(memory_file_name("memory"), Some("MEMORY.md"));
        assert_eq!(memory_file_name("profile"), Some("USER.md"));
        assert_eq!(memory_file_name("unknown"), None);
    }

    #[test]
    fn test_read_write_memory_chunks() {
        let dir = tmp_dir("chunks");
        let path = dir.join("MEMORY.md");
        fs::write(&path, "chunk one\n§\nchunk two\n§\nchunk three\n").unwrap();

        let chunks = read_memory_chunks(&path);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0], "chunk one");
        assert_eq!(chunks[1], "chunk two");
        assert_eq!(chunks[2], "chunk three");

        // Write back
        let mut modified = chunks.clone();
        modified[1] = "modified two".to_string();
        write_memory_chunks(&path, &modified).unwrap();

        let reloaded = read_memory_chunks(&path);
        assert_eq!(reloaded.len(), 3);
        assert_eq!(reloaded[1], "modified two");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_delete_memory_node() {
        let dir = tmp_dir("del_mem");
        let path = dir.join("MEMORY.md");
        fs::write(&path, "alpha\n§\nbeta\n§\ngamma\n").unwrap();

        let result = delete_node("memory:memory:1", &dir, &dir);
        assert!(result.ok);
        assert!(result.message.contains("Deleted"));

        let chunks = read_memory_chunks(&path);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], "alpha");
        assert_eq!(chunks[1], "gamma");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_delete_memory_out_of_range() {
        let dir = tmp_dir("del_mem_oob");
        let path = dir.join("MEMORY.md");
        fs::write(&path, "alpha\n").unwrap();

        let result = delete_node("memory:memory:5", &dir, &dir);
        assert!(!result.ok);
        assert!(result.message.contains("out of range"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_edit_memory_node() {
        let dir = tmp_dir("edit_mem");
        let path = dir.join("USER.md");
        fs::write(&path, "old content\n§\nkeep this\n").unwrap();

        let result = edit_node("memory:profile:0", "new content", &dir, &dir);
        assert!(result.ok);

        let chunks = read_memory_chunks(&path);
        assert_eq!(chunks[0], "new content");
        assert_eq!(chunks[1], "keep this");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_edit_memory_empty_rejected() {
        let dir = tmp_dir("edit_mem_empty");
        let path = dir.join("MEMORY.md");
        fs::write(&path, "content\n").unwrap();

        let result = edit_node("memory:memory:0", "  \n  ", &dir, &dir);
        assert!(!result.ok);
        assert!(result.message.contains("Empty"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_delete_skill_not_found() {
        let dir = tmp_dir("del_skill");
        let result = delete_node("nonexistent", &dir, &dir);
        assert!(!result.ok);
        assert!(result.message.contains("not found"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_edit_skill_not_found() {
        let dir = tmp_dir("edit_skill");
        let result = edit_node("nonexistent", "content", &dir, &dir);
        assert!(!result.ok);
        assert!(result.message.contains("not found"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_build_empty_graph() {
        let skills = tmp_dir("skills_empty");
        let memory = tmp_dir("memory_empty");
        let graph = build_learning_graph(&skills, &memory);
        assert_eq!(graph.nodes.len(), 0);
        assert_eq!(graph.edges.len(), 0);
        let _ = fs::remove_dir_all(&skills);
        let _ = fs::remove_dir_all(&memory);
    }

    #[test]
    fn test_build_graph_with_skills() {
        let skills = tmp_dir("skills_build");
        let skill_dir = skills.join("rust-patterns");
        fs::create_dir_all(&skill_dir).unwrap();
        fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: rust-patterns\ncategory: coding\n---\n# Rust Patterns\n",
        )
        .unwrap();

        let memory = tmp_dir("memory_build");
        let graph = build_learning_graph(&skills, &memory);
        assert_eq!(graph.stats.skill_nodes, 1);
        assert_eq!(graph.nodes[0].id, "rust-patterns");
        assert_eq!(graph.nodes[0].category, "coding");

        let _ = fs::remove_dir_all(&skills);
        let _ = fs::remove_dir_all(&memory);
    }

    #[test]
    fn test_build_graph_with_memory() {
        let skills = tmp_dir("skills_mem");
        let memory = tmp_dir("memory_mem");
        fs::write(memory.join("MEMORY.md"), "Fact one\n§\nFact two\n").unwrap();

        let graph = build_learning_graph(&skills, &memory);
        assert_eq!(graph.stats.memory_nodes, 2);
        assert!(graph.nodes.iter().any(|n| n.id == "memory:memory:0"));
        assert!(graph.nodes.iter().any(|n| n.id == "memory:memory:1"));

        let _ = fs::remove_dir_all(&skills);
        let _ = fs::remove_dir_all(&memory);
    }

    #[test]
    fn test_graph_serialization_roundtrip() {
        let graph = LearningGraph {
            nodes: vec![GraphNode {
                id: "test".to_string(),
                label: "Test".to_string(),
                kind: NodeKind::Skill,
                timestamp: Some(12345),
                category: "test".to_string(),
                use_count: 5,
                state: "active".to_string(),
                created_by: "agent".to_string(),
                pinned: false,
                memory_source: None,
            }],
            edges: vec![],
            clusters: vec![],
            stats: GraphStats {
                total_nodes: 1,
                skill_nodes: 1,
                memory_nodes: 0,
                total_edges: 0,
                edges_per_node: 0.0,
                linked_nodes: 0,
                isolated_pct: 100.0,
                agent_created: 1,
                used: 1,
            },
        };

        let json = serde_json::to_string(&graph).unwrap();
        let deserialized: LearningGraph = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.nodes.len(), 1);
        assert_eq!(deserialized.nodes[0].id, "test");
    }

    #[test]
    fn test_related_skills_edges() {
        let skills = tmp_dir("skills_related");
        let skill_a = skills.join("rust-patterns");
        fs::create_dir_all(&skill_a).unwrap();
        fs::write(
            skill_a.join("SKILL.md"),
            "---\nname: rust-patterns\ncategory: coding\nrelated_skills:\n  - debugging\n---\n# Rust Patterns\n",
        )
        .unwrap();

        let skill_b = skills.join("debugging");
        fs::create_dir_all(&skill_b).unwrap();
        fs::write(
            skill_b.join("SKILL.md"),
            "---\nname: debugging\ncategory: coding\n---\n# Debugging\n",
        )
        .unwrap();

        let memory = tmp_dir("memory_related");
        let graph = build_learning_graph(&skills, &memory);

        assert_eq!(graph.stats.skill_nodes, 2);

        let has_edge = graph.edges.iter().any(|e| {
            (e.source == "rust-patterns" && e.target == "debugging")
                || (e.source == "debugging" && e.target == "rust-patterns")
        });
        assert!(
            has_edge,
            "Expected edge between rust-patterns and debugging from related_skills"
        );

        let _ = fs::remove_dir_all(&skills);
        let _ = fs::remove_dir_all(&memory);
    }

    #[test]
    fn test_category_fallback_edges() {
        let skills = tmp_dir("skills_fallback");

        // Three skills in same category, NONE with declared related_skills
        for name in &["alpha", "beta", "gamma"] {
            let dir = skills.join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("SKILL.md"),
                format!("---\nname: {}\ncategory: research\n---\n# {}\n", name, name),
            )
            .unwrap();
        }

        let memory = tmp_dir("memory_fallback");
        let graph = build_learning_graph(&skills, &memory);
        assert_eq!(graph.stats.skill_nodes, 3);

        // All 3 pairs should be connected via category fallback:
        // alpha-beta, alpha-gamma, beta-gamma
        let edge_count = graph.edges.len();
        assert_eq!(
            edge_count, 3,
            "Expected 3 category-fallback edges for 3 undeclared skills in same category"
        );

        let _ = fs::remove_dir_all(&skills);
        let _ = fs::remove_dir_all(&memory);
    }

    #[test]
    fn test_mixed_declared_and_fallback() {
        let skills = tmp_dir("skills_mixed");

        // skill_a declares related_skills: [skill_b]
        let dir_a = skills.join("skill-a");
        fs::create_dir_all(&dir_a).unwrap();
        fs::write(
            dir_a.join("SKILL.md"),
            "---\nname: skill-a\ncategory: coding\nrelated_skills:\n  - skill-b\n---\n# Skill A\n",
        )
        .unwrap();

        // skill_b has no related_skills — should NOT get category fallback with skill-a
        let dir_b = skills.join("skill-b");
        fs::create_dir_all(&dir_b).unwrap();
        fs::write(
            dir_b.join("SKILL.md"),
            "---\nname: skill-b\ncategory: coding\n---\n# Skill B\n",
        )
        .unwrap();

        // skill_c also no related_skills, same category as skill_b
        let dir_c = skills.join("skill-c");
        fs::create_dir_all(&dir_c).unwrap();
        fs::write(
            dir_c.join("SKILL.md"),
            "---\nname: skill-c\ncategory: coding\n---\n# Skill C\n",
        )
        .unwrap();

        let memory = tmp_dir("memory_mixed");
        let graph = build_learning_graph(&skills, &memory);

        // Expected edges:
        // 1. skill-a → skill-b (declared related_skills)
        // 2. skill-b → skill-c (category fallback — neither has declarations)
        // skill-a is skipped for category fallback because it has declarations
        let edge_count = graph.edges.len();
        assert_eq!(
            edge_count, 2,
            "Expected 2 edges: 1 declared + 1 category fallback"
        );

        let has_ab = graph.edges.iter().any(|e| {
            (e.source == "skill-a" && e.target == "skill-b")
                || (e.source == "skill-b" && e.target == "skill-a")
        });
        assert!(has_ab, "Expected declared edge between skill-a and skill-b");

        let has_bc = graph.edges.iter().any(|e| {
            (e.source == "skill-b" && e.target == "skill-c")
                || (e.source == "skill-c" && e.target == "skill-b")
        });
        assert!(
            has_bc,
            "Expected category-fallback edge between skill-b and skill-c"
        );

        let has_ac = graph.edges.iter().any(|e| {
            (e.source == "skill-a" && e.target == "skill-c")
                || (e.source == "skill-c" && e.target == "skill-a")
        });
        assert!(
            !has_ac,
            "Should NOT have fallback edge between skill-a and skill-c (skill-a has declarations)"
        );

        let _ = fs::remove_dir_all(&skills);
        let _ = fs::remove_dir_all(&memory);
    }

    #[test]
    fn test_yaml_comment_resilience() {
        let skills = tmp_dir("skills_comments");
        let dir_a = skills.join("my-skill");
        fs::create_dir_all(&dir_a).unwrap();
        fs::write(
            dir_a.join("SKILL.md"),
            "---\nname: my-skill\ncategory: coding\nrelated_skills:\n  # This is a comment that should NOT be parsed as a skill name\n  - target-skill\n  # Another comment\n---\n# My Skill\n",
        )
        .unwrap();

        let dir_b = skills.join("target-skill");
        fs::create_dir_all(&dir_b).unwrap();
        fs::write(
            dir_b.join("SKILL.md"),
            "---\nname: target-skill\ncategory: coding\n---\n# Target\n",
        )
        .unwrap();

        let memory = tmp_dir("memory_comments");
        let graph = build_learning_graph(&skills, &memory);

        // Verify exactly 1 edge (from related_skills), no spurious comment-as-skill edges
        let skill_edge_count = graph
            .edges
            .iter()
            .filter(|e| {
                graph
                    .nodes
                    .iter()
                    .any(|n| n.id == e.source && n.kind == NodeKind::Skill)
                    && graph
                        .nodes
                        .iter()
                        .any(|n| n.id == e.target && n.kind == NodeKind::Skill)
            })
            .count();
        assert_eq!(
            skill_edge_count, 1,
            "Expected exactly 1 skill edge; comments should not become skill names"
        );

        let _ = fs::remove_dir_all(&skills);
        let _ = fs::remove_dir_all(&memory);
    }
}
