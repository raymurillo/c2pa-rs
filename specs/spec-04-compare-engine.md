# Spec 04 — Compare Engine

**Phase:** 1 (concurrent with spec-01, spec-02, spec-03, spec-05)  
**Depends on:** spec-00 foundation committed and compiling  
**Produces:** `compare/diff.rs` fully implemented

---

## Goal

Implement field-level diffing between two `DisplayNode` trees. Follow TDD: write
tests first. No `.unwrap()` in production code. Add rustdoc to all public items. Takes two trees
(each representing one manifest), flattens them to path-keyed maps, and produces
a `ManifestDiff` whose `fields` list records which paths are equal, changed,
or exclusive to one side.

---

## Files to modify

- `src/compare/diff.rs` — full implementation

No UI code. No loading code. Pure data transformation over `DisplayNode` slices.

---

## Types (already declared in foundation — implement only the `diff` function)

Add rustdoc to each type:

```rust
/// Field-level diff result between two manifest trees.
pub struct ManifestDiff {
    pub left_label: String,
    pub right_label: String,
    pub fields: Vec<FieldDiff>,
}

/// One field's comparison result between left and right manifests.
pub enum FieldDiff {
    Equal   { path: String, value: String },
    Changed { path: String, left: String, right: String },
    OnlyLeft  { path: String, value: String },
    OnlyRight { path: String, value: String },
}
```

---

## `diff()` implementation

### Algorithm

1. Flatten both `DisplayNode` slices using the existing `flatten()` helper from
   `manifest/tree.rs`. This produces a `Vec<FlatNode>` for each side, where each
   `FlatNode` has a `path` (dot-joined key) and a `display` string.

2. Build two `IndexMap<String, String>` (path → display value) for left and right.
   Use insertion order to preserve the tree's natural depth-first ordering for the
   diff output. (`IndexMap` from the `indexmap` crate preserves insertion order;
   add `indexmap = "2"` to `Cargo.toml`.)

3. Collect the union of all keys. For each key in union order (left keys first,
   then right-only keys):
   - Present in both, same value → `FieldDiff::Equal`
   - Present in both, different value → `FieldDiff::Changed`
   - Only in left → `FieldDiff::OnlyLeft`
   - Only in right → `FieldDiff::OnlyRight`

4. Return `ManifestDiff { left_label, right_label, fields }`.

### Key ordering

Preserve left tree's DFS order for shared/left-only keys, then append right-only
keys in right tree's DFS order. This keeps the diff readable top-to-bottom.

### Value comparison

Compare the `display` field of `FlatNode` (already a human-readable string).
Trim whitespace before comparing to avoid spurious diffs.

```rust
pub fn diff(
    left_label: &str,
    left: &[DisplayNode],
    right_label: &str,
    right: &[DisplayNode],
) -> ManifestDiff {
    use indexmap::IndexMap;
    use crate::manifest::tree::flatten;

    let left_flat = flatten(left);
    let right_flat = flatten(right);

    let left_map: IndexMap<String, String> = left_flat.iter()
        .map(|n| (n.path.clone(), n.display.trim().to_string()))
        .collect();
    let right_map: IndexMap<String, String> = right_flat.iter()
        .map(|n| (n.path.clone(), n.display.trim().to_string()))
        .collect();

    let mut fields = Vec::new();

    for (path, left_val) in &left_map {
        match right_map.get(path) {
            Some(right_val) if left_val == right_val =>
                fields.push(FieldDiff::Equal { path: path.clone(), value: left_val.clone() }),
            Some(right_val) =>
                fields.push(FieldDiff::Changed {
                    path: path.clone(),
                    left: left_val.clone(),
                    right: right_val.clone(),
                }),
            None =>
                fields.push(FieldDiff::OnlyLeft { path: path.clone(), value: left_val.clone() }),
        }
    }

    for (path, right_val) in &right_map {
        if !left_map.contains_key(path) {
            fields.push(FieldDiff::OnlyRight { path: path.clone(), value: right_val.clone() });
        }
    }

    ManifestDiff {
        left_label: left_label.to_string(),
        right_label: right_label.to_string(),
        fields,
    }
}
```

---

## Helper: `ManifestDiff` convenience methods

Add these to `ManifestDiff` — they are used by the compare UI in spec-08:

```rust
impl ManifestDiff {
    /// Count of fields that differ (Changed + OnlyLeft + OnlyRight).
    pub fn diff_count(&self) -> usize {
        self.fields.iter().filter(|f| !matches!(f, FieldDiff::Equal { .. })).count()
    }

    /// True if left and right are identical.
    pub fn is_identical(&self) -> bool {
        self.diff_count() == 0
    }

    /// Return only the non-equal fields.
    pub fn differences(&self) -> impl Iterator<Item = &FieldDiff> {
        self.fields.iter().filter(|f| !matches!(f, FieldDiff::Equal { .. }))
    }
}
```

---

## Unit tests

```rust
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
        assert!(d.fields.iter().all(|f| matches!(f, FieldDiff::Equal { .. })));
    }

    #[test]
    fn changed_value_detected() {
        let left = vec![node("title", "Photo A")];
        let right = vec![node("title", "Photo B")];
        let d = diff("l", &left, "r", &right);
        assert_eq!(d.diff_count(), 1);
        assert!(matches!(&d.fields[0], FieldDiff::Changed { path, left, right }
            if path == "title" && left == "title: Photo A" && right == "title: Photo B"));
    }

    #[test]
    fn only_left_field_detected() {
        let left = vec![node("title", "x"), node("extra", "y")];
        let right = vec![node("title", "x")];
        let d = diff("l", &left, "r", &right);
        assert!(d.fields.iter().any(|f| matches!(f, FieldDiff::OnlyLeft { path, .. } if path == "extra")));
    }

    #[test]
    fn only_right_field_detected() {
        let left = vec![node("title", "x")];
        let right = vec![node("title", "x"), node("extra", "y")];
        let d = diff("l", &left, "r", &right);
        assert!(d.fields.iter().any(|f| matches!(f, FieldDiff::OnlyRight { path, .. } if path == "extra")));
    }

    #[test]
    fn nested_children_are_flattened_and_compared() {
        let left = vec![
            node_with_children("Claim", vec![node("title", "A"), node("format", "jpeg")])
        ];
        let right = vec![
            node_with_children("Claim", vec![node("title", "B"), node("format", "jpeg")])
        ];
        let d = diff("l", &left, "r", &right);
        // "Claim.title" should be Changed, "Claim.format" should be Equal
        assert!(d.fields.iter().any(|f| matches!(f, FieldDiff::Changed { path, .. } if path == "Claim.title")));
        assert!(d.fields.iter().any(|f| matches!(f, FieldDiff::Equal { path, .. } if path == "Claim.format")));
    }

    #[test]
    fn whitespace_trimmed_before_compare() {
        let left = vec![node("k", "  value  ")];
        let right = vec![node("k", "value")];
        let d = diff("l", &left, "r", &right);
        // Should be Equal after trimming
        assert!(d.is_identical());
    }

    #[test]
    fn left_order_preserved_in_output() {
        let left = vec![node("a", "1"), node("b", "2"), node("c", "3")];
        let right = vec![node("c", "3"), node("a", "1"), node("b", "2")];
        let d = diff("l", &left, "r", &right);
        // Output order should follow left tree: a, b, c
        let paths: Vec<&str> = d.fields.iter().map(|f| match f {
            FieldDiff::Equal { path, .. } => path.as_str(),
            FieldDiff::Changed { path, .. } => path.as_str(),
            FieldDiff::OnlyLeft { path, .. } => path.as_str(),
            FieldDiff::OnlyRight { path, .. } => path.as_str(),
        }).collect();
        // a and b must appear before c in the output from the left ordering
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
}
```

---

## Property-based tests

Add to `diff.rs`:

```rust
use proptest::prelude::*;

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
        use std::collections::HashSet;
        use crate::manifest::tree::flatten;
        let left_paths: HashSet<_> = flatten(&left).into_iter().map(|n| n.path).collect();
        let right_paths: HashSet<_> = flatten(&right).into_iter().map(|n| n.path).collect();
        let union_count = left_paths.union(&right_paths).count();
        let d = diff("l", &left, "r", &right);
        prop_assert_eq!(d.fields.len(), union_count);
    }
}
```

## Done criteria

```
cargo test --lib compare
cargo build
cargo fmt -- --check
cargo clippy -- -D warnings
```
