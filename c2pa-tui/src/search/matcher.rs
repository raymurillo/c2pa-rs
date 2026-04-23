use std::ops::Range;

use crate::manifest::tree::FlatNode;

/// Fuzzy / substring matcher backed by the `nucleo` crate.
pub struct Matcher;

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
    ///
    /// Stub: not yet implemented. Implemented in spec-03.
    pub fn new() -> Self {
        todo!("spec-03: implement Matcher::new")
    }

    /// Index a slice of `FlatNode`s for subsequent querying.
    ///
    /// Stub: not yet implemented. Implemented in spec-03.
    pub fn index(&mut self, _nodes: &[FlatNode]) {
        todo!("spec-03")
    }

    /// Run a fuzzy query against the current index and return ranked results.
    ///
    /// Stub: not yet implemented. Implemented in spec-03.
    pub fn query(&mut self, _pattern: &str) -> Vec<MatchResult> {
        todo!("spec-03")
    }
}
