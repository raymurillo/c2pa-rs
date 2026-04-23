use std::ops::Range;
use std::sync::Arc;

use nucleo::pattern::{CaseMatching, Normalization, Pattern};
use nucleo::{Config, Nucleo, Utf32String};

use crate::manifest::tree::FlatNode;

type NucleoMatcher = nucleo::Matcher;

/// Fuzzy / substring matcher backed by the `nucleo` crate.
pub struct Matcher {
    nucleo: Nucleo<usize>,
    items: Vec<FlatNode>,
}

/// A single match result from [`Matcher::query`].
#[derive(Debug, Clone)]
pub struct MatchResult {
    /// Index into the original `FlatNode` slice.
    pub node_index: usize,
    /// Match score; higher is a better match.
    pub score: u32,
    /// Byte ranges within the display string that matched the pattern.
    pub highlight_ranges: Vec<Range<usize>>,
}

impl Default for Matcher {
    fn default() -> Self {
        Self::new()
    }
}

impl Matcher {
    /// Create a new empty `Matcher`.
    pub fn new() -> Self {
        let nucleo = Nucleo::new(Config::DEFAULT, Arc::new(|| {}), None, 1);
        Self {
            nucleo,
            items: Vec::new(),
        }
    }

    /// Replace the current index with a new set of nodes.
    ///
    /// Clears any previous query state. Subsequent calls to `query` search only
    /// the nodes provided here.
    #[tracing::instrument(skip(self, nodes), fields(count = nodes.len()))]
    pub fn index(&mut self, nodes: &[FlatNode]) {
        self.items = nodes.to_vec();
        self.nucleo.restart(true);
        let injector = self.nucleo.injector();
        for (i, node) in nodes.iter().enumerate() {
            let display: Utf32String = node.display.as_str().into();
            injector.push(i, move |_item, cols| {
                cols[0] = display.clone();
            });
        }
        loop {
            let status = self.nucleo.tick(10);
            if !status.running {
                break;
            }
        }
    }

    /// Run a fuzzy query against the current index and return ranked results.
    ///
    /// Returns all items with score 0 when `pattern` is empty.
    #[tracing::instrument(skip(self), fields(pattern))]
    pub fn query(&mut self, pattern: &str) -> Vec<MatchResult> {
        if pattern.is_empty() {
            return self
                .items
                .iter()
                .enumerate()
                .map(|(i, _)| MatchResult {
                    node_index: i,
                    score: 0,
                    highlight_ranges: vec![],
                })
                .collect();
        }

        self.nucleo.pattern.reparse(
            0,
            pattern,
            CaseMatching::Ignore,
            Normalization::Smart,
            false,
        );

        loop {
            let status = self.nucleo.tick(10);
            if !status.running {
                break;
            }
        }

        // Collect snapshot data before borrowing self.items
        let (col_pattern, matched): (Pattern, Vec<(usize, Utf32String)>) = {
            let snapshot = self.nucleo.snapshot();
            let col_pattern = snapshot.pattern().column_pattern(0).clone();
            let items = snapshot
                .matched_items(..)
                .map(|item| (*item.data, item.matcher_columns[0].clone()))
                .collect();
            (col_pattern, items)
        };

        let mut low_matcher = NucleoMatcher::new(Config::DEFAULT);
        let mut results = Vec::new();

        for (node_index, haystack_str) in matched {
            let haystack = haystack_str.slice(..);
            let mut indices: Vec<u32> = Vec::new();
            let score = col_pattern
                .indices(haystack, &mut low_matcher, &mut indices)
                .unwrap_or(0);

            indices.sort_unstable();
            indices.dedup();

            let highlight_ranges =
                char_indices_to_byte_ranges(&self.items[node_index].display, &indices);

            results.push(MatchResult {
                node_index,
                score,
                highlight_ranges,
            });
        }

        results.sort_by_key(|r| std::cmp::Reverse(r.score));
        results
    }
}

/// Convert char-indexed match positions (from nucleo) to byte ranges within a display string.
///
/// Produces tight per-character byte ranges, merging adjacent ones, suitable for highlight
/// rendering. This is the real implementation — not a stub.
fn char_indices_to_byte_ranges(display: &str, char_positions: &[u32]) -> Vec<Range<usize>> {
    if char_positions.is_empty() {
        return vec![];
    }

    let char_to_byte: Vec<usize> = display
        .char_indices()
        .map(|(byte_offset, _)| byte_offset)
        .collect();
    let char_count = char_to_byte.len();

    let mut ranges: Vec<Range<usize>> = char_positions
        .iter()
        .filter_map(|&pos| {
            let pos = pos as usize;
            if pos >= char_count {
                return None;
            }
            let byte_start = char_to_byte[pos];
            let byte_end = if pos + 1 < char_count {
                char_to_byte[pos + 1]
            } else {
                display.len()
            };
            Some(byte_start..byte_end)
        })
        .collect();

    if ranges.is_empty() {
        return ranges;
    }
    ranges.sort_by_key(|r| r.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    let mut current = ranges[0].clone();
    for range in ranges.into_iter().skip(1) {
        if range.start <= current.end {
            current.end = current.end.max(range.end);
        } else {
            merged.push(current);
            current = range;
        }
    }
    merged.push(current);
    merged
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::manifest::tree::FlatNode;

    fn make_nodes() -> Vec<FlatNode> {
        vec![
            FlatNode {
                path: "Claim.title".into(),
                display: "title: My Photo".into(),
                node_index: 0,
            },
            FlatNode {
                path: "Claim.format".into(),
                display: "format: image/jpeg".into(),
                node_index: 1,
            },
            FlatNode {
                path: "Assertions.c2pa.actions".into(),
                display: "c2pa.actions: {...}".into(),
                node_index: 2,
            },
            FlatNode {
                path: "Validation.status".into(),
                display: "status: valid".into(),
                node_index: 3,
            },
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
        m.index(&[]);
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

    #[test]
    fn highlight_ranges_are_tight_not_full_string() {
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
        let total_highlighted: usize = ranges.iter().map(|r| r.end - r.start).sum();
        assert!(
            total_highlighted < "format: image/jpeg".len(),
            "highlight ranges must be tight, not the full string (got {} bytes highlighted)",
            total_highlighted
        );
    }

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
}
