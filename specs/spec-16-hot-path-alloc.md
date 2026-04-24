# Spec 16 — Hot-Path Allocation Reductions

**Phase:** 5 (parallel — requires spec-13 merged and `cargo build` clean)  
**Depends on:** spec-13  
**Produces:** `Cow`-returning `NodeValue::as_str`; reusable path buffer in filter traversal; depth guard on `flatten_inner`

---

## Goal

Three findings from the architecture review all concern unnecessary allocations
and an unbounded recursion in the manifest data layer:

- **Finding 5** — `NodeValue::as_str()` returns an owned `String` on every
  call.  The two most common variants (`Str` and `Missing`) could return a
  borrow or a `'static` str respectively.  The method is called in
  `flatten_inner` (once per node on every index rebuild) and potentially on
  every render frame from the detail pane.
- **Finding 6** — Both `apply_inner` and `apply_inner_ref` in `filter.rs`
  allocate two `String`s per node: the dot-joined path and its `.to_lowercase()`
  copy.  With a manifest of 200 nodes at depth 4, every render frame with the
  filter bar open allocates ~400 short-lived strings.
- **Finding 15** — `flatten_inner` in `tree.rs` is unbounded recursive.  A
  deeply nested assertion payload (crafted or malformed) could exhaust the
  stack.  `ui/detail.rs` already has a 256-level guard; `flatten_inner` must
  have one too.

---

## Files to modify

- `src/manifest/tree.rs` — `NodeValue::as_str` return type; `flatten_inner` depth guard
- `src/manifest/filter.rs` — path-buffer for `apply_inner_ref`

---

## H1 — `NodeValue::as_str` returns `Cow<'_, str>`

### Current

```rust
pub fn as_str(&self) -> String {
    match self {
        NodeValue::Str(s)   => s.clone(),         // heap alloc
        NodeValue::Json(v)  => v.to_string(),     // heap alloc
        NodeValue::Bytes(n) => format!("<{n} bytes>"),  // heap alloc
        NodeValue::Missing  => "<missing>".into(),// heap alloc
    }
}
```

### New signature

```rust
pub fn as_str(&self) -> std::borrow::Cow<'_, str> {
    match self {
        NodeValue::Str(s)   => std::borrow::Cow::Borrowed(s.as_str()),
        NodeValue::Json(v)  => std::borrow::Cow::Owned(v.to_string()),
        NodeValue::Bytes(n) => std::borrow::Cow::Owned(format!("<{n} bytes>")),
        NodeValue::Missing  => std::borrow::Cow::Borrowed("<missing>"),
    }
}
```

`Str` and `Missing` (the two dominant cases) now return borrows; no heap
allocation occurs for them.

### Call-site updates

Every call site that currently uses `node.value.as_str()` receives a
`Cow<'_, str>`.  Most callers pass the result to `format!` or concatenate
with `&str` — both work transparently because `Cow<str>` implements `Deref<Target=str>`.

Update `flatten_inner` in `tree.rs`:

```rust
// Before
display: format!("{}: {}", node.key, node.value.as_str()),

// After — Cow derefs to &str; format! does not allocate an intermediate String
display: format!("{}: {}", node.key, node.value.as_str()),
// (no change to the format! call — Cow derefs to &str automatically)
```

Audit all other call sites in `ui/detail.rs` and `ui/compare.rs`; update any
that call `.to_string()` or `.to_owned()` on the result unnecessarily.

### Requirements

- Return type is `Cow<'_, str>`.
- `NodeValue::Str("x")` → borrows the inner `String`, zero allocation.
- `NodeValue::Missing` → borrows a `'static` str, zero allocation.
- `NodeValue::Json(v)` → still allocates (serialisation is inherently owned).
- `NodeValue::Bytes(n)` → still allocates (formatted string).
- All existing tests continue to pass (the display strings are identical).

---

## H2 — Reusable path buffer in filter traversal

### Problem

Both `apply_inner` and `apply_inner_ref` in `filter.rs` build paths like:

```rust
let path = if prefix.is_empty() {
    node.key.clone()
} else {
    format!("{}.{}", prefix, node.key)
};
let lpath = path.to_lowercase();
```

This allocates a new `String` for `path` and another for `lpath` at every node,
every call.

### Fix

Replace the `prefix: &str` parameter with a `path_buf: &mut String` that the
caller pushes to and then truncates (stack discipline):

```rust
/// Borrow-based recursive filter with a reusable path buffer.
fn apply_inner_ref(
    nodes: &[DisplayNode],
    path_buf: &mut String,
    filter: &FieldFilter,
) -> Vec<DisplayNode> {
    nodes.iter().filter_map(|node| {
        let segment_start = path_buf.len();

        // Extend the buffer.
        if !path_buf.is_empty() {
            path_buf.push('.');
        }
        path_buf.push_str(&node.key);

        // Lowercase comparison without a second allocation:
        // build a lowercase copy of just the new segment.
        let lpath = path_buf.to_lowercase();   // still one alloc per node

        let excluded = filter.exclude_paths.iter().any(|p| p.matches(&lpath));
        let result = if excluded {
            None
        } else {
            let self_included = filter.include_paths.is_empty()
                || filter.include_paths.iter().any(|p| p.matches(&lpath));

            let children = apply_inner_ref(&node.children, path_buf, filter);

            let kept = self_included
                || (!filter.include_paths.is_empty() && !children.is_empty());

            if kept {
                Some(DisplayNode {
                    key: node.key.clone(),
                    value: node.value.clone(),
                    children,
                })
            } else {
                None
            }
        };

        // Restore the buffer to its pre-node state.
        path_buf.truncate(segment_start);
        result
    })
    .collect()
}
```

Public entry points become:

```rust
pub fn apply_ref(&self, nodes: &[DisplayNode]) -> Vec<DisplayNode> {
    apply_inner_ref(nodes, &mut String::new(), self)
}
```

**Further optimization (optional):** maintain a lowercase buffer alongside the
path buffer to avoid the `.to_lowercase()` alloc entirely.  This doubles the
complexity for a minor gain; leave it as a future improvement unless profiling
shows it is a bottleneck.

### Requirements

- One `String` (the path buffer) is allocated per `apply_ref` call (not per
  node).
- Behaviour is identical to the current implementation — all filter tests pass.
- `apply` (`owned` variant) delegates to `apply_ref` (see spec-17 H1 for the
  full deduplication; this spec only changes the buffer strategy in `apply_ref`).

---

## H3 — Depth guard on `flatten_inner`

### Problem

`flatten_inner` is recursive with no depth limit.  The detail pane renderer
already guards at 256 levels; the search indexer does not.

### Fix

Add a `depth: usize` parameter and return early at 256:

```rust
pub fn flatten(nodes: &[DisplayNode]) -> Vec<FlatNode> {
    let mut out = Vec::new();
    flatten_inner(nodes, &mut String::new(), &mut out, 0);
    out
}

fn flatten_inner(
    nodes: &[DisplayNode],
    path_buf: &mut String,
    out: &mut Vec<FlatNode>,
    depth: usize,
) {
    if depth > 256 {
        return;
    }
    for node in nodes {
        let segment_start = path_buf.len();
        if !path_buf.is_empty() { path_buf.push('.'); }
        path_buf.push_str(&node.key);

        let idx = out.len();
        out.push(FlatNode {
            path: path_buf.clone(),
            display: format!("{}: {}", node.key, node.value.as_str()),
            node_index: idx,
        });
        flatten_inner(&node.children, path_buf, out, depth + 1);

        path_buf.truncate(segment_start);
    }
}
```

This also migrates `flatten_inner` to use the path buffer (same technique as
H2), eliminating the `format!("{}.{}", prefix, node.key)` per node.

### Requirements

- Trees with depth ≤ 256 flatten identically to the current implementation.
- Trees with depth > 256 are truncated at depth 256 without panicking.
- A test verifies the truncation behaviour.

---

## Testing Strategy

### `tree.rs`

```rust
#[test]
fn node_value_str_returns_borrowed() {
    let v = NodeValue::Str("hello".into());
    let s = v.as_str();
    assert!(matches!(s, std::borrow::Cow::Borrowed(_)));
}

#[test]
fn node_value_missing_returns_borrowed() {
    let s = NodeValue::Missing.as_str();
    assert!(matches!(s, std::borrow::Cow::Borrowed(_)));
}

#[test]
fn node_value_json_returns_owned() {
    let s = NodeValue::Json(serde_json::json!({"k": 1})).as_str();
    assert!(matches!(s, std::borrow::Cow::Owned(_)));
}

#[test]
fn flatten_truncates_at_depth_256() {
    fn deep(depth: usize) -> DisplayNode {
        if depth == 0 {
            DisplayNode { key: "leaf".into(), value: NodeValue::Missing, children: vec![] }
        } else {
            DisplayNode { key: format!("n{depth}"), value: NodeValue::Missing,
                          children: vec![deep(depth - 1)] }
        }
    }
    let root = deep(300); // deeper than the guard
    let flat = flatten(&[root]);
    // Should not stack-overflow and should stop at or before depth 256.
    assert!(flat.len() <= 257, "expected truncation at depth 256, got {} nodes", flat.len());
}
```

### `filter.rs`

All existing proptest and unit tests must continue to pass.  Add:

```rust
proptest! {
    #[test]
    fn apply_ref_path_buffer_is_always_restored(
        nodes in prop::collection::vec(arb_node(), 0..20),
        include in "[a-z.*]+"
    ) {
        // Verify that path_buf is empty after the call completes.
        // (Indirectly tested by running apply_ref twice and getting identical results.)
        let f = FieldFilter::from_query(&include).unwrap_or_default();
        let r1 = f.apply_ref(&nodes);
        let r2 = f.apply_ref(&nodes);
        prop_assert_eq!(r1.len(), r2.len());
    }
}
```

---

## Edge Cases

- Empty node list: `flatten` returns `[]`; `apply_ref` returns `[]`.
  Both must work correctly with the path buffer starting empty.
- Node keys containing `.`: the dot-joined path will contain extra dots
  (e.g. `"c2pa.actions"` at depth 1 produces `"Assertions.c2pa.actions"`).
  This is the existing behaviour and is unchanged.
- `NodeValue::Bytes(0)` → `"<0 bytes>"` (no change).

---

## Dependencies

No new crate dependencies.  `std::borrow::Cow` is in the standard library.

---

## Done criteria

```bash
cargo test -p c2pa-tui -- manifest::tree::tests manifest::filter::tests
cargo clippy -p c2pa-tui -- -D warnings
cargo fmt -p c2pa-tui -- --check
```

All new tests pass.  No existing tests regress.  No stack overflow when
`flatten` is called on a depth-300 tree.
