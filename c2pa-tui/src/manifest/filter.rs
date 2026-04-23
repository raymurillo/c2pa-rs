use glob::Pattern;

use crate::error::Result;
use crate::manifest::tree::DisplayNode;

/// Filter that includes or excludes manifest fields by glob path patterns.
#[derive(Debug, Clone, Default)]
pub struct FieldFilter {
    /// Glob patterns for paths to include.
    pub include_paths: Vec<Pattern>,
    /// Glob patterns for paths to exclude.
    pub exclude_paths: Vec<Pattern>,
}

impl FieldFilter {
    /// Parse a filter query string into a `FieldFilter`.
    ///
    /// Stub: not yet implemented. Implemented in spec-01.
    pub fn from_query(q: &str) -> Result<Self> {
        let _ = q;
        todo!("spec-01: implement FieldFilter::from_query")
    }

    /// Apply this filter to a node list, returning only matching nodes.
    ///
    /// Stub: not yet implemented. Implemented in spec-01.
    pub fn apply(&self, nodes: Vec<DisplayNode>) -> Vec<DisplayNode> {
        let _ = nodes;
        todo!("spec-01: implement FieldFilter::apply")
    }
}
