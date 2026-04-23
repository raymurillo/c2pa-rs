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

/// Convert a `c2pa::Reader` into a flat list of top-level `DisplayNode`s.
///
/// Each manifest in the reader becomes one root node whose children are the
/// Claim, Claim Signature, Assertions, Ingredients, and Validation sections.
/// The active manifest is placed first and labelled `(active)`.
pub fn store_to_nodes(reader: &c2pa::Reader) -> Vec<DisplayNode> {
    let active_label = reader.active_label();
    let mut nodes = Vec::new();

    // Active manifest first.
    if let Some(label) = active_label {
        if let Some(manifest) = reader.get_manifest(label) {
            nodes.push(manifest_to_node(manifest, label, true, reader));
        }
    }

    // Remaining manifests sorted for determinism.
    let mut rest: Vec<(&str, &c2pa::Manifest)> = reader
        .manifests()
        .iter()
        .filter(|(l, _)| Some(l.as_str()) != active_label)
        .map(|(l, m)| (l.as_str(), m))
        .collect();
    rest.sort_by_key(|(l, _)| *l);
    for (label, manifest) in rest {
        nodes.push(manifest_to_node(manifest, label, false, reader));
    }

    nodes
}

fn manifest_to_node(
    manifest: &c2pa::Manifest,
    label: &str,
    is_active: bool,
    reader: &c2pa::Reader,
) -> DisplayNode {
    let key = if is_active {
        format!("Manifest: {} (active)", label)
    } else {
        format!("Manifest: {}", label)
    };

    let children = vec![
        claim_to_node(manifest),
        signature_to_node(manifest),
        assertions_to_node(manifest),
        ingredients_to_node(manifest),
        validation_to_node(reader, is_active),
    ];

    DisplayNode {
        key,
        value: NodeValue::Missing,
        children,
    }
}

fn leaf(key: impl Into<String>, value: NodeValue) -> DisplayNode {
    DisplayNode {
        key: key.into(),
        value,
        children: vec![],
    }
}

fn claim_to_node(manifest: &c2pa::Manifest) -> DisplayNode {
    let mut children = Vec::new();
    if let Some(title) = manifest.title() {
        children.push(leaf("title", NodeValue::Str(title.to_owned())));
    }
    if let Some(format) = manifest.format() {
        children.push(leaf("format", NodeValue::Str(format.to_owned())));
    }
    children.push(leaf(
        "instance_id",
        NodeValue::Str(manifest.instance_id().to_owned()),
    ));
    if let Some(cg) = manifest.claim_generator() {
        children.push(leaf("claim_generator", NodeValue::Str(cg.to_owned())));
    }
    DisplayNode {
        key: "Claim".into(),
        value: NodeValue::Missing,
        children,
    }
}

fn signature_to_node(manifest: &c2pa::Manifest) -> DisplayNode {
    let (issuer, time, alg) = if let Some(sig) = manifest.signature_info() {
        let issuer = sig.issuer.clone().unwrap_or_else(|| "unknown".to_owned());
        let time = sig.time.clone().unwrap_or_else(|| "unknown".to_owned());
        let alg = sig
            .alg
            .as_ref()
            .map(|a| format!("{a:?}"))
            .unwrap_or_else(|| "unknown".to_owned());
        (issuer, time, alg)
    } else {
        (
            "unknown".to_owned(),
            "unknown".to_owned(),
            "unknown".to_owned(),
        )
    };

    DisplayNode {
        key: "Claim Signature".into(),
        value: NodeValue::Missing,
        children: vec![
            leaf("issuer", NodeValue::Str(issuer)),
            leaf("time", NodeValue::Str(time)),
            leaf("alg", NodeValue::Str(alg)),
        ],
    }
}

fn assertions_to_node(manifest: &c2pa::Manifest) -> DisplayNode {
    let assertions = manifest.assertions();
    let children: Vec<DisplayNode> = assertions
        .iter()
        .map(|assertion| {
            let label = assertion.label().to_owned();
            match assertion.value() {
                Ok(value) => DisplayNode {
                    key: label,
                    value: NodeValue::Missing,
                    children: json_to_children(value),
                },
                Err(_) => {
                    let size = assertion.binary().map(|b| b.len()).unwrap_or(0);
                    leaf(label, NodeValue::Bytes(size))
                }
            }
        })
        .collect();

    DisplayNode {
        key: format!("Assertions ({})", assertions.len()),
        value: NodeValue::Missing,
        children,
    }
}

fn ingredients_to_node(manifest: &c2pa::Manifest) -> DisplayNode {
    let ingredients = manifest.ingredients();
    let children: Vec<DisplayNode> = ingredients
        .iter()
        .enumerate()
        .map(|(i, ingredient)| {
            let name = ingredient
                .title()
                .map(|t| t.to_owned())
                .unwrap_or_else(|| i.to_string());

            let mut ing_children = Vec::new();
            if let Some(fmt) = ingredient.format() {
                ing_children.push(leaf("format", NodeValue::Str(fmt.to_owned())));
            }
            ing_children.push(leaf(
                "instance_id",
                NodeValue::Str(ingredient.instance_id().to_owned()),
            ));
            ing_children.push(leaf(
                "relationship",
                NodeValue::Str(format!("{:?}", ingredient.relationship())),
            ));
            if ingredient.active_manifest().is_some() {
                ing_children.push(leaf("Manifest", NodeValue::Str("<embedded>".into())));
            }

            DisplayNode {
                key: name,
                value: NodeValue::Missing,
                children: ing_children,
            }
        })
        .collect();

    DisplayNode {
        key: format!("Ingredients ({})", ingredients.len()),
        value: NodeValue::Missing,
        children,
    }
}

fn validation_to_node(reader: &c2pa::Reader, is_active: bool) -> DisplayNode {
    let state_str = if is_active {
        match reader.validation_state() {
            c2pa::ValidationState::Valid | c2pa::ValidationState::Trusted => "valid",
            c2pa::ValidationState::Invalid => "invalid",
        }
    } else {
        "unknown"
    };

    let mut children = vec![leaf("status", NodeValue::Str(state_str.to_owned()))];

    if is_active {
        if let Some(statuses) = reader.validation_status() {
            if !statuses.is_empty() {
                let error_children: Vec<DisplayNode> = statuses
                    .iter()
                    .map(|s| {
                        let explanation = s.explanation().unwrap_or("").to_owned();
                        leaf(s.code(), NodeValue::Str(explanation))
                    })
                    .collect();
                children.push(DisplayNode {
                    key: format!("Errors ({})", statuses.len()),
                    value: NodeValue::Missing,
                    children: error_children,
                });
            }
        }
    }

    DisplayNode {
        key: "Validation".into(),
        value: NodeValue::Missing,
        children,
    }
}

/// Recursively convert a JSON value to child `DisplayNode`s.
pub(crate) fn json_to_children(value: &Value) -> Vec<DisplayNode> {
    match value {
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| json_to_node(k.as_str(), v))
            .collect(),
        Value::Array(arr) => arr
            .iter()
            .enumerate()
            .map(|(i, v)| json_to_node(&format!("[{i}]"), v))
            .collect(),
        _ => vec![],
    }
}

fn json_to_node(key: &str, value: &Value) -> DisplayNode {
    match value {
        Value::Object(_) | Value::Array(_) => DisplayNode {
            key: key.to_owned(),
            value: NodeValue::Missing,
            children: json_to_children(value),
        },
        Value::String(s) => leaf(key, NodeValue::Str(s.clone())),
        _ => leaf(key, NodeValue::Str(value.to_string())),
    }
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

    fn leaf_node(key: &str, value: NodeValue) -> DisplayNode {
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
        let nodes = vec![leaf_node("title", NodeValue::Str("test".into()))];
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
                leaf_node("a", NodeValue::Str("1".into())),
                leaf_node("b", NodeValue::Bytes(8)),
            ],
        )];
        let flat = flatten(&nodes);
        assert_eq!(flat.len(), 3);
        assert_eq!(flat[0].path, "root");
        assert_eq!(flat[1].path, "root.a");
        assert_eq!(flat[2].path, "root.b");
    }

    #[test]
    fn flatten_node_index_is_position() {
        let nodes = vec![
            leaf_node("x", NodeValue::Missing),
            leaf_node("y", NodeValue::Missing),
            leaf_node("z", NodeValue::Missing),
        ];
        let flat = flatten(&nodes);
        for (i, node) in flat.iter().enumerate() {
            assert_eq!(node.node_index, i);
        }
    }

    #[test]
    fn flatten_deeply_nested_path() {
        let deep = branch(
            "a",
            vec![branch("b", vec![leaf_node("c", NodeValue::Missing)])],
        );
        let flat = flatten(&[deep]);
        assert_eq!(flat[2].path, "a.b.c");
    }

    #[test]
    fn flatten_produces_dot_joined_paths() {
        let nodes = vec![DisplayNode {
            key: "Claim".into(),
            value: NodeValue::Missing,
            children: vec![DisplayNode {
                key: "title".into(),
                value: NodeValue::Str("x".into()),
                children: vec![],
            }],
        }];
        let flat = flatten(&nodes);
        assert_eq!(flat[0].path, "Claim");
        assert_eq!(flat[1].path, "Claim.title");
    }

    // --- json helpers ---

    #[test]
    fn json_array_assertion_expands_to_indexed_children() {
        let arr = json!([1, 2, 3]);
        let children = json_to_children(&arr);
        assert_eq!(children.len(), 3);
        assert_eq!(children[0].key, "[0]");
        assert_eq!(children[1].key, "[1]");
        assert_eq!(children[2].key, "[2]");
    }

    #[test]
    fn json_object_expands_to_named_children() {
        let obj = json!({"a": 1, "b": "hello"});
        let children = json_to_children(&obj);
        assert_eq!(children.len(), 2);
        let keys: std::collections::HashSet<&str> =
            children.iter().map(|c| c.key.as_str()).collect();
        assert!(keys.contains("a"));
        assert!(keys.contains("b"));
    }

    // --- store_to_nodes via fixture ---

    #[test]
    fn signed_jpeg_has_claim_node() {
        let reader = c2pa::Reader::default()
            .with_file("tests/fixtures/C.jpg")
            .expect("C.jpg should be loadable");
        let nodes = store_to_nodes(&reader);
        assert!(!nodes.is_empty(), "expected at least one manifest node");
        let manifest_node = &nodes[0];
        assert!(
            manifest_node.key.starts_with("Manifest:"),
            "root node should be a Manifest node"
        );
        assert!(
            manifest_node.children.iter().any(|n| n.key == "Claim"),
            "manifest node should have a Claim child"
        );
    }

    #[test]
    fn empty_reader_returns_no_nodes() {
        // A default (empty) Reader has no manifests loaded.
        let reader = c2pa::Reader::default();
        let nodes = store_to_nodes(&reader);
        assert!(
            nodes.is_empty(),
            "empty reader should produce no manifest nodes"
        );
    }
}
