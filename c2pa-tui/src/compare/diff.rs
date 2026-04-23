use indexmap::IndexMap;

use crate::manifest::tree::DisplayNode;

/// Field-level diff result between two manifest trees.
#[derive(Debug, Clone)]
pub struct ManifestDiff {
    /// Label identifying the left manifest (e.g. filename).
    pub left_label: String,
    /// Label identifying the right manifest (e.g. filename).
    pub right_label: String,
    /// Per-field diff entries, ordered by left tree DFS then right-only fields.
    pub fields: Vec<FieldDiff>,
}

/// One field's comparison result between left and right manifests.
#[derive(Debug, Clone)]
pub enum FieldDiff {
    /// Field is present in both manifests with identical values.
    Equal { path: String, value: String },
    /// Field is present in both manifests but with different values.
    Changed {
        path: String,
        left: String,
        right: String,
    },
    /// Field exists only in the left manifest.
    OnlyLeft { path: String, value: String },
    /// Field exists only in the right manifest.
    OnlyRight { path: String, value: String },
}

impl ManifestDiff {
    /// Count of fields that differ (Changed + OnlyLeft + OnlyRight).
    pub fn diff_count(&self) -> usize {
        self.fields
            .iter()
            .filter(|f| !matches!(f, FieldDiff::Equal { .. }))
            .count()
    }

    /// True if left and right are identical.
    pub fn is_identical(&self) -> bool {
        self.diff_count() == 0
    }

    /// Return only the non-equal fields.
    pub fn differences(&self) -> impl Iterator<Item = &FieldDiff> {
        self.fields
            .iter()
            .filter(|f| !matches!(f, FieldDiff::Equal { .. }))
    }
}

/// Extract the value portion of a display string for comparison.
///
/// Display strings have the form `"key: value"`. Trimming just the value avoids
/// spurious diffs caused by surrounding whitespace in the raw field data.
fn comparison_value(display: &str) -> &str {
    display
        .split_once(": ")
        .map(|(_, val)| val.trim())
        .unwrap_or_else(|| display.trim())
}

/// Compute the field-level diff between two manifest [`DisplayNode`] trees.
///
/// Flattens both trees to path-keyed maps and emits one [`FieldDiff`] per
/// union key. Left tree DFS order is preserved; right-only fields are appended
/// in right tree DFS order.
pub fn diff(
    left_label: &str,
    left: &[DisplayNode],
    right_label: &str,
    right: &[DisplayNode],
) -> ManifestDiff {
    use crate::manifest::tree::flatten;

    let left_flat = flatten(left);
    let right_flat = flatten(right);

    let left_map: IndexMap<String, String> = left_flat
        .iter()
        .map(|n| (n.path.clone(), n.display.clone()))
        .collect();
    let right_map: IndexMap<String, String> = right_flat
        .iter()
        .map(|n| (n.path.clone(), n.display.clone()))
        .collect();

    let mut fields = Vec::new();

    for (path, left_display) in &left_map {
        match right_map.get(path) {
            Some(right_display)
                if comparison_value(left_display) == comparison_value(right_display) =>
            {
                fields.push(FieldDiff::Equal {
                    path: path.clone(),
                    value: left_display.clone(),
                });
            }
            Some(right_display) => {
                fields.push(FieldDiff::Changed {
                    path: path.clone(),
                    left: left_display.clone(),
                    right: right_display.clone(),
                });
            }
            None => {
                fields.push(FieldDiff::OnlyLeft {
                    path: path.clone(),
                    value: left_display.clone(),
                });
            }
        }
    }

    for (path, right_display) in &right_map {
        if !left_map.contains_key(path) {
            fields.push(FieldDiff::OnlyRight {
                path: path.clone(),
                value: right_display.clone(),
            });
        }
    }

    ManifestDiff {
        left_label: left_label.to_string(),
        right_label: right_label.to_string(),
        fields,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::tree::{DisplayNode, NodeValue};

    fn node(key: &str, val: &str) -> DisplayNode {
        DisplayNode {
            key: key.into(),
            value: NodeValue::Str(val.into()),
            children: vec![],
        }
    }

    fn node_with_children(key: &str, children: Vec<DisplayNode>) -> DisplayNode {
        DisplayNode {
            key: key.into(),
            value: NodeValue::Missing,
            children,
        }
    }

    #[test]
    fn identical_trees_produce_only_equal_fields() {
        let left = vec![node("title", "My Photo"), node("format", "image/jpeg")];
        let right = left.clone();
        let d = diff("left", &left, "right", &right);
        assert!(d.is_identical());
        assert_eq!(d.diff_count(), 0);
        assert!(d
            .fields
            .iter()
            .all(|f| matches!(f, FieldDiff::Equal { .. })));
    }

    #[test]
    fn changed_value_detected() {
        let left = vec![node("title", "Photo A")];
        let right = vec![node("title", "Photo B")];
        let d = diff("l", &left, "r", &right);
        assert_eq!(d.diff_count(), 1);
        assert!(
            matches!(&d.fields[0], FieldDiff::Changed { path, left, right }
            if path == "title" && left == "title: Photo A" && right == "title: Photo B")
        );
    }

    #[test]
    fn only_left_field_detected() {
        let left = vec![node("title", "x"), node("extra", "y")];
        let right = vec![node("title", "x")];
        let d = diff("l", &left, "r", &right);
        assert!(d
            .fields
            .iter()
            .any(|f| matches!(f, FieldDiff::OnlyLeft { path, .. } if path == "extra")));
    }

    #[test]
    fn only_right_field_detected() {
        let left = vec![node("title", "x")];
        let right = vec![node("title", "x"), node("extra", "y")];
        let d = diff("l", &left, "r", &right);
        assert!(d
            .fields
            .iter()
            .any(|f| matches!(f, FieldDiff::OnlyRight { path, .. } if path == "extra")));
    }

    #[test]
    fn nested_children_are_flattened_and_compared() {
        let left = vec![node_with_children(
            "Claim",
            vec![node("title", "A"), node("format", "jpeg")],
        )];
        let right = vec![node_with_children(
            "Claim",
            vec![node("title", "B"), node("format", "jpeg")],
        )];
        let d = diff("l", &left, "r", &right);
        assert!(d
            .fields
            .iter()
            .any(|f| matches!(f, FieldDiff::Changed { path, .. } if path == "Claim.title")));
        assert!(d
            .fields
            .iter()
            .any(|f| matches!(f, FieldDiff::Equal { path, .. } if path == "Claim.format")));
    }

    #[test]
    fn whitespace_trimmed_before_compare() {
        let left = vec![node("k", "  value  ")];
        let right = vec![node("k", "value")];
        let d = diff("l", &left, "r", &right);
        assert!(d.is_identical());
    }

    #[test]
    fn left_order_preserved_in_output() {
        let left = vec![node("a", "1"), node("b", "2"), node("c", "3")];
        let right = vec![node("c", "3"), node("a", "1"), node("b", "2")];
        let d = diff("l", &left, "r", &right);
        let paths: Vec<&str> = d
            .fields
            .iter()
            .map(|f| match f {
                FieldDiff::Equal { path, .. } => path.as_str(),
                FieldDiff::Changed { path, .. } => path.as_str(),
                FieldDiff::OnlyLeft { path, .. } => path.as_str(),
                FieldDiff::OnlyRight { path, .. } => path.as_str(),
            })
            .collect();
        let a_pos = paths.iter().position(|&p| p == "a").unwrap();
        let b_pos = paths.iter().position(|&p| p == "b").unwrap();
        let c_pos = paths.iter().position(|&p| p == "c").unwrap();
        assert!(a_pos < b_pos);
        assert!(b_pos < c_pos);
    }

    #[test]
    fn empty_trees_produce_empty_diff() {
        let d = diff("l", &[], "r", &[]);
        assert!(d.fields.is_empty());
        assert!(d.is_identical());
    }

    #[test]
    fn labels_are_preserved() {
        let d = diff("left-file.jpg", &[], "right-file.jpg", &[]);
        assert_eq!(d.left_label, "left-file.jpg");
        assert_eq!(d.right_label, "right-file.jpg");
    }

    mod property_tests {
        use super::*;
        use crate::manifest::tree::flatten;
        use proptest::prelude::*;
        use std::collections::HashSet;

        fn arb_node() -> impl Strategy<Value = DisplayNode> {
            ("[a-z]{1,8}", ".*").prop_map(|(key, val)| DisplayNode {
                key,
                value: NodeValue::Str(val),
                children: vec![],
            })
        }

        proptest! {
            #[test]
            fn diff_of_identical_trees_has_no_differences(
                nodes in prop::collection::vec(arb_node(), 0..15)
            ) {
                let d = diff("l", &nodes, "r", &nodes.clone());
                prop_assert!(d.is_identical());
            }

            #[test]
            fn diff_count_is_non_negative(
                left in prop::collection::vec(arb_node(), 0..10),
                right in prop::collection::vec(arb_node(), 0..10),
            ) {
                let d = diff("l", &left, "r", &right);
                prop_assert!(d.diff_count() <= d.fields.len());
            }

            #[test]
            fn field_count_equals_union_of_paths(
                left in prop::collection::vec(arb_node(), 0..10),
                right in prop::collection::vec(arb_node(), 0..10),
            ) {
                let left_paths: HashSet<_> = flatten(&left).into_iter().map(|n| n.path).collect();
                let right_paths: HashSet<_> = flatten(&right).into_iter().map(|n| n.path).collect();
                let union_count = left_paths.union(&right_paths).count();
                let d = diff("l", &left, "r", &right);
                prop_assert_eq!(d.fields.len(), union_count);
            }
        }
    }
}
