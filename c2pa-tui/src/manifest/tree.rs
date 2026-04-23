use serde_json::Value;

/// A single node in the rendered manifest tree.
///
/// Leaf nodes have an empty `children` vec. Interior nodes (sections, arrays,
/// objects) carry their content in `children` and use `NodeValue::Missing` for
/// their own value.
#[derive(Debug, Clone)]
pub struct DisplayNode {
    /// Field key or section label.
    pub key: String,
    /// Value of this node; `Missing` for interior nodes.
    pub value: NodeValue,
    /// Child nodes for interior nodes.
    pub children: Vec<DisplayNode>,
}

/// The typed value held by a [`DisplayNode`].
#[derive(Debug, Clone)]
pub enum NodeValue {
    /// Plain string value.
    Str(String),
    /// Arbitrary JSON value.
    Json(Value),
    /// Binary blob represented by its byte length.
    Bytes(usize),
    /// No value (interior / section node).
    Missing,
}

/// Flat representation used by the search engine.
#[derive(Debug, Clone)]
pub struct FlatNode {
    /// Dot-joined key path, e.g. `"assertions.c2pa.actions.action"`.
    pub path: String,
    /// Human-readable display string for this node.
    pub display: String,
    /// Index of this node in the flattened vec.
    pub node_index: usize,
}

/// Convert a `Reader` into a flat list of top-level `DisplayNode`s.
///
/// Each manifest in the reader becomes one root node whose children are the
/// Claim, Assertions, Ingredients, and Validation sections.
/// Stub: returns empty vec. Implemented in spec-01.
pub fn store_to_nodes(_reader: &c2pa::Reader) -> Vec<DisplayNode> {
    todo!("spec-01: implement store_to_nodes")
}

/// Flatten a `DisplayNode` tree to a `Vec<FlatNode>` for search indexing.
pub fn flatten(nodes: &[DisplayNode]) -> Vec<FlatNode> {
    let mut out = Vec::new();
    flatten_inner(nodes, "", &mut out);
    out
}

fn flatten_inner(nodes: &[DisplayNode], prefix: &str, out: &mut Vec<FlatNode>) {
    for node in nodes {
        let path = if prefix.is_empty() {
            node.key.clone()
        } else {
            format!("{}.{}", prefix, node.key)
        };
        let idx = out.len();
        out.push(FlatNode {
            path: path.clone(),
            display: format!("{}: {}", node.key, node.value.as_str()),
            node_index: idx,
        });
        flatten_inner(&node.children, &path, out);
    }
}

impl NodeValue {
    /// Render the value as a display string.
    pub fn as_str(&self) -> String {
        match self {
            NodeValue::Str(s) => s.clone(),
            NodeValue::Json(v) => v.to_string(),
            NodeValue::Bytes(n) => format!("<{n} bytes>"),
            NodeValue::Missing => "<missing>".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn leaf(key: &str, value: NodeValue) -> DisplayNode {
        DisplayNode {
            key: key.to_owned(),
            value,
            children: vec![],
        }
    }

    fn branch(key: &str, children: Vec<DisplayNode>) -> DisplayNode {
        DisplayNode {
            key: key.to_owned(),
            value: NodeValue::Missing,
            children,
        }
    }

    // --- NodeValue::as_str ---

    #[test]
    fn node_value_str() {
        assert_eq!(NodeValue::Str("hello".into()).as_str(), "hello");
    }

    #[test]
    fn node_value_json_null() {
        assert_eq!(NodeValue::Json(json!(null)).as_str(), "null");
    }

    #[test]
    fn node_value_json_object() {
        let v = json!({"k": 1});
        assert_eq!(NodeValue::Json(v).as_str(), r#"{"k":1}"#);
    }

    #[test]
    fn node_value_bytes() {
        assert_eq!(NodeValue::Bytes(42).as_str(), "<42 bytes>");
        assert_eq!(NodeValue::Bytes(0).as_str(), "<0 bytes>");
    }

    #[test]
    fn node_value_missing() {
        assert_eq!(NodeValue::Missing.as_str(), "<missing>");
    }

    // --- flatten ---

    #[test]
    fn flatten_empty() {
        let result = flatten(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn flatten_single_leaf() {
        let nodes = vec![leaf("title", NodeValue::Str("test".into()))];
        let flat = flatten(&nodes);
        assert_eq!(flat.len(), 1);
        assert_eq!(flat[0].path, "title");
        assert_eq!(flat[0].display, "title: test");
        assert_eq!(flat[0].node_index, 0);
    }

    #[test]
    fn flatten_nested() {
        let nodes = vec![branch(
            "root",
            vec![
                leaf("a", NodeValue::Str("1".into())),
                leaf("b", NodeValue::Bytes(8)),
            ],
        )];
        let flat = flatten(&nodes);
        // root + a + b = 3 entries
        assert_eq!(flat.len(), 3);
        assert_eq!(flat[0].path, "root");
        assert_eq!(flat[1].path, "root.a");
        assert_eq!(flat[2].path, "root.b");
    }

    #[test]
    fn flatten_node_index_is_position() {
        let nodes = vec![
            leaf("x", NodeValue::Missing),
            leaf("y", NodeValue::Missing),
            leaf("z", NodeValue::Missing),
        ];
        let flat = flatten(&nodes);
        for (i, node) in flat.iter().enumerate() {
            assert_eq!(node.node_index, i);
        }
    }

    #[test]
    fn flatten_deeply_nested_path() {
        let deep = branch("a", vec![branch("b", vec![leaf("c", NodeValue::Missing)])]);
        let flat = flatten(&[deep]);
        assert_eq!(flat[2].path, "a.b.c");
    }
}
