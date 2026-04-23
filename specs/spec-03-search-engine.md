# Spec 03 — Search Engine

**Phase:** 1 (concurrent with spec-01, spec-02, spec-04, spec-05)  
**Depends on:** spec-00 foundation committed and compiling  
**Produces:** `search/matcher.rs` fully implemented

---

## Goal

Implement fuzzy/substring search over a flat list of manifest nodes using the
`nucleo` crate. Follow TDD: write tests first. No `.unwrap()` in production code.
Add `#[tracing::instrument]` to `index` and `query`. The engine takes a slice of `FlatNode` (defined in `manifest/tree.rs`),
builds an index, then returns ranked `MatchResult` items with character-level
highlight ranges for rendering.

---

## Files to modify

- `src/search/matcher.rs` — full implementation

Do **not** touch any UI file, `app.rs`, or loader code.

---

## Background: `nucleo`

`nucleo` is a high-performance fuzzy matcher (used in Helix editor). Key types:

- `nucleo::Nucleo<T>` — the matcher parameterized over item data `T`
- `nucleo::Config` — configuration (case sensitivity, etc.)
- `nucleo::pattern::Pattern` — compiled query
- `nucleo::pattern::CaseMatching` — `Ignore`, `Smart`, `Respect`
- `nucleo::Utf32String` — intern strings for matching

The general workflow:
1. Create `Nucleo::new(config, notify, None, 1)` — single column
2. Call `injector = nucleo.injector()` then `injector.push(item, |item, cols| { cols[0] = item.display.clone().into(); })`
3. Call `nucleo.tick(timeout_ms)` to process
4. Call `nucleo.snapshot()` to get results
5. Iterate `snapshot.matched_items(..)`

> **Action required:** Read the `nucleo` crate docs/examples to verify the exact API.
> The API may differ between versions — check the version pinned in `Cargo.toml`.
> Key things to verify: how `push` works, how to extract match indices from the snapshot.

---

## `src/search/matcher.rs`

```rust
use std::ops::Range;
use std::sync::Arc;
use nucleo::{Nucleo, Config, Utf32String};
use nucleo::pattern::{Pattern, CaseMatching, Normalization};
use crate::manifest::tree::FlatNode;

pub struct Matcher {
    nucleo: Nucleo<usize>,  // usize = original index into the FlatNode slice
    items: Vec<FlatNode>,
}

#[derive(Debug, Clone)]
pub struct MatchResult {
    /// Index into the original FlatNode slice passed to `index()`.
    pub node_index: usize,
    /// Fuzzy match score (higher = better).
    pub score: u32,
    /// Byte ranges within `FlatNode::display` that matched (for highlight rendering).
    pub highlight_ranges: Vec<Range<usize>>,
}

impl Matcher {
    pub fn new() -> Self {
        let config = Config::DEFAULT;
        let nucleo = Nucleo::new(config, Arc::new(|| {}), None, 1);
        Self { nucleo, items: Vec::new() }
    }

    /// Replace the current index with a new set of nodes.
    ///
    /// Clears any previous query state. Subsequent calls to `query` search only
    /// the nodes provided here.
    #[tracing::instrument(skip(self, nodes), fields(count = nodes.len()))]
    pub fn index(&mut self, nodes: &[FlatNode]) {
        self.items = nodes.to_vec();
        // Restart nucleo to clear old items
        self.nucleo.restart(true);
        let injector = self.nucleo.injector();
        for (i, node) in nodes.iter().enumerate() {
            let display: Utf32String = node.display.as_str().into();
            injector.push(i, move |_item, cols| {
                cols[0] = display.clone();
            });
        }
        // Tick until all items are ingested
        loop {
            let status = self.nucleo.tick(10);
            if status.changed { break; }
        }
    }

    /// Run a fuzzy/substring query against the current index.
    ///
    /// Returns ranked `MatchResult`s sorted by score descending.
    /// Returns all items with score 0 when `pattern` is empty.
    #[tracing::instrument(skip(self), fields(pattern))]
    pub fn query(&mut self, pattern: &str) -> Vec<MatchResult> {
        if pattern.is_empty() {
            return self.items.iter().enumerate().map(|(i, _)| MatchResult {
                node_index: i,
                score: 0,
                highlight_ranges: vec![],
            }).collect();
        }

        let pat = Pattern::new(
            pattern,
            CaseMatching::Smart,
            Normalization::Smart,
            nucleo::pattern::AtomKind::Fuzzy,
        );
        self.nucleo.pattern.reparse(0, pattern, CaseMatching::Smart, Normalization::Smart, false);

        // Tick until stable
        loop {
            let status = self.nucleo.tick(10);
            if !status.running { break; }
        }

        let snapshot = self.nucleo.snapshot();
        let mut results = Vec::new();

        for item in snapshot.matched_items(..) {
            let node_index = *item.data;
            let score = item.matcher_columns[0].score().unwrap_or(0) as u32;

            // Extract highlight indices from the match
            let highlight_ranges = extract_highlights(
                &self.items[node_index].display,
                &item.matcher_columns[0],
            );

            results.push(MatchResult { node_index, score, highlight_ranges });
        }

        // Sort by score descending (nucleo snapshot may already be sorted, but be explicit)
        results.sort_by(|a, b| b.score.cmp(&a.score));
        results
    }
}

/// Convert nucleo match indices into byte ranges for the display string.
///
/// # Implementation requirement
///
/// This function MUST return tight per-character byte ranges, not a single
/// range covering the whole string. The `highlight_ranges_are_tight` test in the
/// done criteria enforces this.
///
/// To implement: call `nucleo::Snapshot::get_indices` (or the equivalent method
/// on the snapshot item for your nucleo version) to retrieve a `Vec<u32>` of
/// matched char positions. Then convert char positions to byte offsets by walking
/// `display.char_indices()`.
///
/// Look at how Helix's `helix-tui` or `telescope-nucleo` integrations call
/// `get_indices` for a reference implementation.
fn extract_highlights(display: &str, snapshot: &nucleo::Snapshot<usize>, item_index: u32) -> Vec<Range<usize>> {
    // Use nucleo's index extraction API to get matched char positions.
    // The exact call depends on the nucleo version — check the crate docs.
    // Pseudocode:
    //   let mut indices = Vec::new();
    //   snapshot.pattern().column_pattern(0).indices(item.matcher_columns[0].slice(..), &mut nucleo::Matcher::new(nucleo::Config::DEFAULT), &mut indices);
    //   convert indices (Vec<u32>) to byte ranges via display.char_indices()
    todo!("implement extract_highlights using nucleo index API — see doc comment above")
}

impl Default for Matcher {
    fn default() -> Self { Self::new() }
}
```

> **Implementation requirement:** `extract_highlights` is **not optional** —
> the done criteria include a test that asserts highlight ranges are tight (i.e.,
> shorter than the full display string for a non-empty query on a matching item).
> Do not mark this spec done while `extract_highlights` contains a `todo!()` or
> returns a full-length range.

---

## Unit tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::tree::{DisplayNode, NodeValue, FlatNode, flatten};

    fn make_nodes() -> Vec<FlatNode> {
        vec![
            FlatNode { path: "Claim.title".into(), display: "title: My Photo".into(), node_index: 0 },
            FlatNode { path: "Claim.format".into(), display: "format: image/jpeg".into(), node_index: 1 },
            FlatNode { path: "Assertions.c2pa.actions".into(), display: "c2pa.actions: {...}".into(), node_index: 2 },
            FlatNode { path: "Validation.status".into(), display: "status: valid".into(), node_index: 3 },
        ]
    }

    #[test]
    fn empty_query_returns_all_items() {
        let mut m = Matcher::new();
        m.index(&make_nodes());
        let results = m.query("");
        assert_eq!(results.len(), 4);
    }

    #[test]
    fn fuzzy_match_finds_substring() {
        let mut m = Matcher::new();
        m.index(&make_nodes());
        let results = m.query("jpeg");
        assert!(!results.is_empty());
        assert!(results.iter().any(|r| r.node_index == 1));
    }

    #[test]
    fn fuzzy_match_is_case_insensitive_by_default() {
        let mut m = Matcher::new();
        m.index(&make_nodes());
        let results_lower = m.query("photo");
        let results_upper = m.query("PHOTO");
        assert_eq!(results_lower.len(), results_upper.len());
    }

    #[test]
    fn results_are_sorted_by_score_descending() {
        let mut m = Matcher::new();
        m.index(&make_nodes());
        let results = m.query("title");
        // "title: My Photo" should score higher than e.g. partial matches
        if results.len() > 1 {
            for i in 0..results.len() - 1 {
                assert!(results[i].score >= results[i + 1].score);
            }
        }
    }

    #[test]
    fn reindex_clears_previous_items() {
        let mut m = Matcher::new();
        m.index(&make_nodes());
        m.index(&[]); // re-index with empty set
        let results = m.query("title");
        assert!(results.is_empty());
    }

    #[test]
    fn highlight_ranges_are_within_display_len() {
        let mut m = Matcher::new();
        m.index(&make_nodes());
        let results = m.query("jpeg");
        for r in &results {
            let display_len = make_nodes()[r.node_index].display.len();
            for range in &r.highlight_ranges {
                assert!(range.end <= display_len);
                assert!(range.start <= range.end);
            }
        }
    }
}
```

---

## Property-based tests

Add to `matcher.rs`:

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn query_never_panics(pattern in ".*", displays in prop::collection::vec(".*", 0..20)) {
        let nodes: Vec<FlatNode> = displays.iter().enumerate().map(|(i, d)| FlatNode {
            path: format!("key{i}"),
            display: d.clone(),
            node_index: i,
        }).collect();
        let mut m = Matcher::new();
        m.index(&nodes);
        let _ = m.query(&pattern);
    }

    #[test]
    fn result_count_never_exceeds_indexed_count(
        pattern in "[a-z]{0,5}",
        count in 0usize..50
    ) {
        let nodes: Vec<FlatNode> = (0..count).map(|i| FlatNode {
            path: format!("p{i}"),
            display: format!("display {i}"),
            node_index: i,
        }).collect();
        let mut m = Matcher::new();
        m.index(&nodes);
        let results = m.query(&pattern);
        prop_assert!(results.len() <= count);
    }
}
```

## Done criteria

```
cargo test --lib search
cargo build
cargo fmt -- --check
cargo clippy -- -D warnings
```

Additionally, the following test must pass — add it to the unit test block:

```rust
#[test]
fn highlight_ranges_are_tight_not_full_string() {
    // For an exact substring match, the highlight range must not span the
    // entire display string. This confirms extract_highlights is real, not stubbed.
    let mut m = Matcher::new();
    m.index(&[FlatNode {
        path: "Claim.format".into(),
        display: "format: image/jpeg".into(),
        node_index: 0,
    }]);
    let results = m.query("jpeg");
    assert!(!results.is_empty(), "should match");
    let ranges = &results[0].highlight_ranges;
    assert!(!ranges.is_empty(), "highlight_ranges must not be empty");
    // The matched range must be shorter than the full display string
    let total_highlighted: usize = ranges.iter().map(|r| r.end - r.start).sum();
    assert!(
        total_highlighted < "format: image/jpeg".len(),
        "highlight ranges must be tight, not the full string (got {} bytes highlighted)",
        total_highlighted
    );
}
```
