//! Accessibility tree access via the CDP `Accessibility` domain.
//!
//! Provides a simplified view of the page's accessibility tree,
//! hiding raw CDP types behind plain Rust structs.

use std::collections::HashMap;

use crate::cdp::browser_protocol::accessibility::GetFullAxTreeParams;
use crate::error::Result;
use crate::target::Target;

/// A node in the accessibility tree.
#[derive(Debug, Clone)]
pub struct AXNode {
    pub role: String,
    pub name: Option<String>,
    pub value: Option<String>,
    pub description: Option<String>,
    pub children: Vec<AXNode>,
}

/// Accessor for the page's accessibility tree.
pub struct Accessibility {
    target: Target,
}

impl Accessibility {
    pub(crate) fn new(target: Target) -> Self {
        Self { target }
    }

    /// Get a full snapshot of the accessibility tree.
    ///
    /// Returns the root [`AXNode`] with all descendants populated
    /// in the `children` field.
    pub async fn snapshot(&self) -> Result<AXNode> {
        let result = self
            .target
            .execute(GetFullAxTreeParams {
                depth: None,
                frame_id: None,
            })
            .await?;

        // Build a flat map of node_id → (converted AXNode, child_ids).
        let mut nodes: HashMap<String, (AXNode, Vec<String>)> = HashMap::new();
        let mut root_id: Option<String> = None;

        for cdp_node in &result.nodes {
            let node_id = cdp_node.node_id.inner().to_string();

            if root_id.is_none() {
                root_id = Some(node_id.clone());
            }

            let role = cdp_node
                .role
                .as_ref()
                .and_then(|v| v.value.as_ref())
                .and_then(|v| v.as_str())
                .unwrap_or("none")
                .to_string();

            let name = cdp_node
                .name
                .as_ref()
                .and_then(|v| v.value.as_ref())
                .and_then(|v| v.as_str())
                .map(String::from);

            let value = cdp_node
                .value
                .as_ref()
                .and_then(|v| v.value.as_ref())
                .and_then(|v| v.as_str())
                .map(String::from);

            let description = cdp_node
                .description
                .as_ref()
                .and_then(|v| v.value.as_ref())
                .and_then(|v| v.as_str())
                .map(String::from);

            let child_ids: Vec<String> = cdp_node
                .child_ids
                .as_ref()
                .map(|ids| ids.iter().map(|id| id.inner().to_string()).collect())
                .unwrap_or_default();

            nodes.insert(
                node_id,
                (
                    AXNode {
                        role,
                        name,
                        value,
                        description,
                        children: Vec::new(),
                    },
                    child_ids,
                ),
            );
        }

        // Build the tree bottom-up by resolving children.
        let root_id = root_id.unwrap_or_default();
        build_tree(&root_id, &mut nodes)
    }
}

/// Recursively build the tree from the flat map.
fn build_tree(node_id: &str, nodes: &mut HashMap<String, (AXNode, Vec<String>)>) -> Result<AXNode> {
    let (mut node, child_ids) = nodes.remove(node_id).unwrap_or_else(|| {
        (
            AXNode {
                role: "none".into(),
                name: None,
                value: None,
                description: None,
                children: Vec::new(),
            },
            Vec::new(),
        )
    });

    for cid in &child_ids {
        if let Ok(child) = build_tree(cid, nodes) {
            node.children.push(child);
        }
    }

    Ok(node)
}
