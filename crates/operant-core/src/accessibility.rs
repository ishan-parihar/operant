//! Accessibility tree extraction for browser pages.
//!
//! Provides structured accessibility tree snapshots from CDP's
//! `Accessibility.getFullAXTree`, with text-based rendering and
//! ref-based element selectors (`@e1`, `@e2`, …) matching the
//! Python operant-agent's `ariaSnapshot` output.
//!
//! # Example
//!
//! ```ignore
//! use operant_core::accessibility::{AccessibilityTree, AccessibilityNode};
//!
//! // Parse from CDP response
//! let tree = AccessibilityTree::from_cdp_nodes(&flat_nodes)?;
//!
//! // Text-based rendering
//! let text = tree.render_compact();
//! println!("{text}");
//!
//! // Find interactive elements by ref
//! if let Some(node) = tree.find_by_ref("@e3") {
//!     println!("Found: {} - {:?}", node.name.unwrap_or_default(), node.role);
//! }
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// A single node in the accessibility tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessibilityNode {
    /// ARIA role (e.g. "button", "link", "textbox", "heading").
    pub role: String,
    /// Accessible name (text content, label, etc.).
    pub name: Option<String>,
    /// Current value (for inputs, sliders, etc.).
    pub value: Option<String>,
    /// Child nodes.
    pub children: Vec<AccessibilityNode>,
    /// Ref selector for interactive elements (e.g. `@e1`).
    /// Only set on elements that can be clicked/filled.
    pub ref_id: Option<String>,
    /// ARIA properties as key-value pairs.
    pub properties: Vec<(String, String)>,
    /// Whether this node is ignored by the accessibility tree.
    pub ignored: bool,
}

/// Complete accessibility tree for a page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessibilityTree {
    /// Root node containing the full tree.
    pub root: AccessibilityNode,
    /// Flat list of interactive elements with their ref IDs.
    pub refs: HashMap<String, String>,
    /// Total number of interactive elements.
    pub element_count: usize,
}

#[derive(Debug, Clone, Deserialize)]
struct CdpAxNode {
    #[serde(rename = "nodeId")]
    node_id: String,
    #[serde(default)]
    ignored: bool,
    #[serde(default)]
    role: Option<Value>,
    #[serde(default)]
    name: Option<Value>,
    #[serde(default)]
    value: Option<Value>,
    #[serde(default)]
    properties: Option<Vec<CdpAxProperty>>,
    #[serde(rename = "childIds", default)]
    child_ids: Option<Vec<String>>,
    #[serde(rename = "parentId", default)]
    parent_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CdpAxProperty {
    name: String,
    value: Value,
}

/// Interactive roles that get ref selectors.
const INTERACTIVE_ROLES: &[&str] = &[
    "button",
    "link",
    "textbox",
    "checkbox",
    "radio",
    "combobox",
    "listbox",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "option",
    "slider",
    "spinbutton",
    "switch",
    "tab",
    "treeitem",
    "menu",
    "menubar",
    "toolbar",
    "searchbox",
    "scrollbar",
];

impl AccessibilityNode {
    /// Whether this node is interactive (can be clicked/filled).
    pub fn is_interactive(&self) -> bool {
        INTERACTIVE_ROLES.contains(&self.role.as_str())
    }

    /// Whether this node has visible text content.
    pub fn has_text(&self) -> bool {
        self.name.as_ref().is_some_and(|n| !n.is_empty())
            || self.value.as_ref().is_some_and(|v| !v.is_empty())
    }
}

impl AccessibilityTree {
    /// CDP returns a flat array of `{nodeId, parentId, childIds}` nodes.
    /// This reconstructs the tree and assigns `@e1`-style ref IDs to interactive elements.
    pub fn from_cdp_nodes(cdp_nodes: &[Value]) -> Result<Self, String> {
        let nodes: Vec<CdpAxNode> = cdp_nodes
            .iter()
            .filter_map(|v| serde_json::from_value(v.clone()).ok())
            .collect();

        if nodes.is_empty() {
            return Err("No accessibility nodes found".into());
        }

        let mut node_map: HashMap<String, CdpAxNode> = HashMap::new();
        for node in &nodes {
            node_map.insert(node.node_id.clone(), node.clone());
        }

        let roots: Vec<&str> = nodes
            .iter()
            .filter(|n| n.parent_id.is_none())
            .map(|n| n.node_id.as_str())
            .collect();

        if roots.is_empty() {
            return Err("No root node found in accessibility tree".into());
        }

        let mut ref_counter = 0u32;
        let mut refs = HashMap::new();

        let root_children: Vec<AccessibilityNode> = roots
            .iter()
            .flat_map(|root_id| build_children(root_id, &node_map, &mut ref_counter, &mut refs))
            .collect();

        let root = AccessibilityNode {
            role: "root".into(),
            name: None,
            value: None,
            children: root_children,
            ref_id: None,
            properties: vec![],
            ignored: false,
        };

        let element_count = refs.len();

        Ok(AccessibilityTree {
            root,
            refs,
            element_count,
        })
    }

    /// Render the tree as compact text (interactive elements only).
    ///
    /// Returns a text representation similar to the Python browser tool's
    /// `browser_snapshot` output with ref selectors.
    pub fn render_compact(&self) -> String {
        let mut output = String::new();
        render_node_compact(&self.root, &mut output, 0);
        output
    }

    /// Render the full tree (all elements with text content).
    pub fn render_full(&self) -> String {
        let mut output = String::new();
        render_node_full(&self.root, &mut output, 0);
        output
    }

    /// Find a node by its ref selector (e.g. "@e3").
    pub fn find_by_ref(&self, ref_id: &str) -> Option<&AccessibilityNode> {
        let clean_ref = ref_id.trim_start_matches('@');
        find_by_ref_recursive(&self.root, clean_ref)
    }

    /// Get all interactive elements as a flat list with their refs.
    pub fn interactive_elements(&self) -> Vec<(String, &AccessibilityNode)> {
        let mut elements = Vec::new();
        collect_interactive(&self.root, &mut elements);
        elements
    }
}

fn build_children(
    parent_id: &str,
    node_map: &HashMap<String, CdpAxNode>,
    ref_counter: &mut u32,
    refs: &mut HashMap<String, String>,
) -> Vec<AccessibilityNode> {
    let parent = match node_map.get(parent_id) {
        Some(n) => n,
        None => return vec![],
    };

    let child_ids = match &parent.child_ids {
        Some(ids) => ids.clone(),
        None => return vec![],
    };

    child_ids
        .iter()
        .filter_map(|child_id| {
            let child = node_map.get(child_id)?;
            if child.ignored {
                return None;
            }

            let role = extract_string_value(&child.role).unwrap_or_else(|| "unknown".into());
            let name = extract_string_value(&child.name);
            let value = extract_string_value(&child.value);

            let properties: Vec<(String, String)> = child
                .properties
                .as_ref()
                .map(|props| {
                    props
                        .iter()
                        .map(|p| {
                            let val = match &p.value {
                                Value::String(s) if !s.is_empty() => s.clone(),
                                Value::Number(n) => n.to_string(),
                                Value::Bool(b) => b.to_string(),
                                _ => String::new(),
                            };
                            (p.name.clone(), val)
                        })
                        .collect()
                })
                .unwrap_or_default();

            let interactive = INTERACTIVE_ROLES.contains(&role.as_str());
            let ref_id = if interactive {
                *ref_counter += 1;
                let rid = format!("e{}", ref_counter);
                refs.insert(rid.clone(), child.node_id.clone());
                Some(format!("@{}", rid))
            } else {
                None
            };

            let grand_children = build_children(child_id, node_map, ref_counter, refs);

            Some(AccessibilityNode {
                role,
                name,
                value,
                children: grand_children,
                ref_id,
                properties,
                ignored: false,
            })
        })
        .collect()
}

fn extract_string_value(v: &Option<Value>) -> Option<String> {
    v.as_ref().and_then(|val| {
        // CDP wraps field values in {"value": "..."} objects
        if let Some(inner) = val.get("value") {
            return match inner {
                Value::String(s) if !s.is_empty() => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                Value::Bool(b) => Some(b.to_string()),
                _ => None,
            };
        }
        match val {
            Value::String(s) if !s.is_empty() => Some(s.clone()),
            Value::Number(n) => Some(n.to_string()),
            Value::Bool(b) => Some(b.to_string()),
            _ => None,
        }
    })
}

fn render_node_compact(node: &AccessibilityNode, output: &mut String, depth: usize) {
    if node.ignored {
        return;
    }

    let indent = "  ".repeat(depth);
    let ref_tag = node.ref_id.as_deref().unwrap_or("");

    match node.role.as_str() {
        "heading" => {
            let level = node
                .properties
                .iter()
                .find(|(k, _)| k == "level")
                .map(|(_, v)| v.as_str())
                .unwrap_or("#");
            let text = node.name.as_deref().unwrap_or("");
            let prefix = if ref_tag.is_empty() {
                String::new()
            } else {
                format!("[{}] ", ref_tag)
            };
            output.push_str(&format!(
                "{}{}{} {}\n",
                indent,
                prefix,
                "#".repeat(level.len()),
                text
            ));
        }
        "button" | "link" => {
            let text = node.name.as_deref().unwrap_or("[no text]");
            output.push_str(&format!(
                "{}[{}] {} \"{}\"\n",
                indent, ref_tag, node.role, text
            ));
        }
        "textbox" | "searchbox" => {
            let placeholder = node.name.as_deref().unwrap_or("");
            let value = node.value.as_deref().unwrap_or("");
            output.push_str(&format!(
                "{}[{}] {} \"{}\" value=\"{}\"\n",
                indent, ref_tag, node.role, placeholder, value
            ));
        }
        "checkbox" | "switch" => {
            let checked = node
                .properties
                .iter()
                .find(|(k, _)| k == "checked")
                .map(|(_, v)| v.as_str())
                .unwrap_or("false");
            let text = node.name.as_deref().unwrap_or("");
            output.push_str(&format!(
                "{}[{}] {} \"{}\" checked={}\n",
                indent, ref_tag, node.role, text, checked
            ));
        }
        "img" => {
            let alt = node.name.as_deref().unwrap_or("[no alt]");
            output.push_str(&format!("{}[{}] img \"{}\"\n", indent, ref_tag, alt));
        }
        "navigation" | "banner" | "main" | "contentinfo" | "complementary" | "region" => {
            let label = node.name.as_deref().unwrap_or("");
            output.push_str(&format!("{}{} \"{}\"\n", indent, node.role, label));
        }
        "list" | "listitem" | "tree" | "treeitem" | "table" | "row" | "cell" | "grid" => {
            let text = node.name.as_deref().unwrap_or("");
            if !text.is_empty() {
                output.push_str(&format!("{}{} \"{}\"\n", indent, node.role, text));
            }
        }
        "root" => {}
        _ => {
            if let Some(text) = &node.name
                && !text.is_empty()
            {
                output.push_str(&format!("{}{} \"{}\"\n", indent, node.role, text));
            }
        }
    }

    for child in &node.children {
        render_node_compact(child, output, depth + 1);
    }
}

fn render_node_full(node: &AccessibilityNode, output: &mut String, depth: usize) {
    if node.ignored {
        return;
    }

    let indent = "  ".repeat(depth);

    let mut parts = vec![node.role.clone()];
    if let Some(name) = &node.name
        && !name.is_empty()
    {
        parts.push(format!("\"{}\"", name));
    }
    if let Some(value) = &node.value
        && !value.is_empty()
    {
        parts.push(format!("value=\"{}\"", value));
    }
    if let Some(ref_id) = &node.ref_id {
        parts.insert(0, format!("[{}]", ref_id));
    }

    output.push_str(&format!("{}{}\n", indent, parts.join(" ")));

    for child in &node.children {
        render_node_full(child, output, depth + 1);
    }
}

fn find_by_ref_recursive<'a>(
    node: &'a AccessibilityNode,
    clean_ref: &str,
) -> Option<&'a AccessibilityNode> {
    if let Some(ref_id) = &node.ref_id
        && ref_id.trim_start_matches('@') == clean_ref
    {
        return Some(node);
    }
    for child in &node.children {
        if let Some(found) = find_by_ref_recursive(child, clean_ref) {
            return Some(found);
        }
    }
    None
}

fn collect_interactive<'a>(
    node: &'a AccessibilityNode,
    out: &mut Vec<(String, &'a AccessibilityNode)>,
) {
    if let Some(ref_id) = &node.ref_id {
        out.push((ref_id.clone(), node));
    }
    for child in &node.children {
        collect_interactive(child, out);
    }
}

pub async fn fetch_accessibility_tree(cdp_url: &str) -> Result<AccessibilityTree, String> {
    let command = serde_json::json!({
        "id": 1,
        "method": "Accessibility.getFullAXTree",
        "params": {}
    });

    let response = crate::tools::cdp_utils::send_cdp_command(cdp_url, &command)
        .await
        .map_err(|e| format!("CDP command failed: {}", e))?;

    let nodes = response
        .get("result")
        .and_then(|r| r.get("nodes"))
        .and_then(|n| n.as_array())
        .ok_or_else(|| "CDP response missing result.nodes".to_string())?;

    AccessibilityTree::from_cdp_nodes(nodes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_empty_nodes_returns_error() {
        let result = AccessibilityTree::from_cdp_nodes(&[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_simple_button_tree() {
        let nodes = json!([
            {
                "nodeId": "1",
                "role": {"value": "root-web-area"},
                "childIds": ["2"]
            },
            {
                "nodeId": "2",
                "parentId": "1",
                "role": {"value": "button"},
                "name": {"value": "Click Me"},
                "childIds": []
            }
        ]);

        let tree = AccessibilityTree::from_cdp_nodes(nodes.as_array().unwrap()).unwrap();
        assert_eq!(tree.element_count, 1);
        assert!(tree.refs.contains_key("e1"));

        let text = tree.render_compact();
        assert!(text.contains("@e1"));
        assert!(text.contains("Click Me"));
    }

    #[test]
    fn test_heading_and_paragraph() {
        let nodes = json!([
            {
                "nodeId": "1",
                "role": {"value": "root-web-area"},
                "childIds": ["2", "3"]
            },
            {
                "nodeId": "2",
                "parentId": "1",
                "role": {"value": "heading"},
                "name": {"value": "Welcome"},
                "properties": [{"name": "level", "value": "1"}],
                "childIds": []
            },
            {
                "nodeId": "3",
                "parentId": "1",
                "role": {"value": "paragraph"},
                "name": {"value": "Hello world"},
                "childIds": []
            }
        ]);

        let tree = AccessibilityTree::from_cdp_nodes(nodes.as_array().unwrap()).unwrap();
        assert_eq!(tree.element_count, 0);

        let text = tree.render_full();
        assert!(text.contains("heading"));
        assert!(text.contains("Welcome"));
    }

    #[test]
    fn test_textbox_with_value() {
        let nodes = json!([
            {
                "nodeId": "1",
                "role": {"value": "root-web-area"},
                "childIds": ["2"]
            },
            {
                "nodeId": "2",
                "parentId": "1",
                "role": {"value": "textbox"},
                "name": {"value": "Email"},
                "value": {"value": "user@example.com"},
                "childIds": []
            }
        ]);

        let tree = AccessibilityTree::from_cdp_nodes(nodes.as_array().unwrap()).unwrap();
        assert_eq!(tree.element_count, 1);

        let text = tree.render_compact();
        assert!(text.contains("@e1"));
        assert!(text.contains("Email"));
        assert!(text.contains("user@example.com"));
    }

    #[test]
    fn test_nested_tree() {
        let nodes = json!([
            {
                "nodeId": "1",
                "role": {"value": "root-web-area"},
                "childIds": ["2"]
            },
            {
                "nodeId": "2",
                "parentId": "1",
                "role": {"value": "navigation"},
                "name": {"value": "Main nav"},
                "childIds": ["3", "4"]
            },
            {
                "nodeId": "3",
                "parentId": "2",
                "role": {"value": "link"},
                "name": {"value": "Home"},
                "childIds": []
            },
            {
                "nodeId": "4",
                "parentId": "2",
                "role": {"value": "link"},
                "name": {"value": "About"},
                "childIds": []
            }
        ]);

        let tree = AccessibilityTree::from_cdp_nodes(nodes.as_array().unwrap()).unwrap();
        assert_eq!(tree.element_count, 2);
        assert!(tree.refs.contains_key("e1"));
        assert!(tree.refs.contains_key("e2"));

        let elements = tree.interactive_elements();
        assert_eq!(elements.len(), 2);
    }

    #[test]
    fn test_ignored_nodes_filtered() {
        let nodes = json!([
            {
                "nodeId": "1",
                "role": {"value": "root-web-area"},
                "childIds": ["2", "3"]
            },
            {
                "nodeId": "2",
                "parentId": "1",
                "ignored": true,
                "role": {"value": "button"},
                "name": {"value": "Hidden"},
                "childIds": []
            },
            {
                "nodeId": "3",
                "parentId": "1",
                "role": {"value": "button"},
                "name": {"value": "Visible"},
                "childIds": []
            }
        ]);

        let tree = AccessibilityTree::from_cdp_nodes(nodes.as_array().unwrap()).unwrap();
        assert_eq!(tree.element_count, 1);
        assert!(tree.render_compact().contains("Visible"));
        assert!(!tree.render_compact().contains("Hidden"));
    }

    #[test]
    fn test_find_by_ref() {
        let nodes = json!([
            {
                "nodeId": "1",
                "role": {"value": "root-web-area"},
                "childIds": ["2", "3"]
            },
            {
                "nodeId": "2",
                "parentId": "1",
                "role": {"value": "button"},
                "name": {"value": "Submit"},
                "childIds": []
            },
            {
                "nodeId": "3",
                "parentId": "1",
                "role": {"value": "link"},
                "name": {"value": "Cancel"},
                "childIds": []
            }
        ]);

        let tree = AccessibilityTree::from_cdp_nodes(nodes.as_array().unwrap()).unwrap();
        let node = tree.find_by_ref("@e1").unwrap();
        assert_eq!(node.name.as_deref(), Some("Submit"));

        let node = tree.find_by_ref("@e2").unwrap();
        assert_eq!(node.name.as_deref(), Some("Cancel"));

        assert!(tree.find_by_ref("@e99").is_none());
    }

    #[test]
    fn test_interactive_roles() {
        let roles = vec![
            "button", "link", "textbox", "checkbox", "radio", "combobox", "listbox", "menuitem",
            "slider", "switch", "tab",
        ];
        for role in roles {
            let node = AccessibilityNode {
                role: role.into(),
                name: None,
                value: None,
                children: vec![],
                ref_id: None,
                properties: vec![],
                ignored: false,
            };
            assert!(node.is_interactive(), "{} should be interactive", role);
        }

        let non_interactive = AccessibilityNode {
            role: "paragraph".into(),
            name: None,
            value: None,
            children: vec![],
            ref_id: None,
            properties: vec![],
            ignored: false,
        };
        assert!(!non_interactive.is_interactive());
    }
}
