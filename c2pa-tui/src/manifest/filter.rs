use glob::Pattern;

use crate::error::{AppError, Result};
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
    /// Parse a semicolon-separated filter query into a `FieldFilter`.
    ///
    /// Tokens prefixed with `!` become exclude patterns; all others become
    /// include patterns.  If no include patterns are given the default is
    /// include-everything (`**`).
    ///
    /// Input limits (enforced before calling `glob::Pattern::new`):
    /// - Entire query ≤ 256 characters
    /// - Each token ≤ 128 characters
    /// - Each token contains ≤ 4 `{` characters
    pub fn from_query(q: &str) -> Result<Self> {
        if q.len() > 256 {
            return Err(AppError::Glob(glob::PatternError {
                pos: 0,
                msg: "query exceeds maximum length of 256 characters",
            }));
        }

        let mut include_paths = Vec::new();
        let mut exclude_paths = Vec::new();

        for token in q.split(';') {
            let token = token.trim();
            if token.is_empty() {
                continue;
            }
            if token.len() > 128 {
                return Err(AppError::Glob(glob::PatternError {
                    pos: 0,
                    msg: "token exceeds maximum length of 128 characters",
                }));
            }
            let brace_count = token.chars().filter(|&c| c == '{').count();
            if brace_count > 4 {
                return Err(AppError::Glob(glob::PatternError {
                    pos: 0,
                    msg: "token contains too many alternation groups",
                }));
            }

            if let Some(pat_str) = token.strip_prefix('!') {
                exclude_paths.push(Pattern::new(&pat_str.to_lowercase())?);
            } else {
                include_paths.push(Pattern::new(&token.to_lowercase())?);
            }
        }

        Ok(Self {
            include_paths,
            exclude_paths,
        })
    }

    /// Apply this filter to a node list, returning only matching nodes.
    ///
    /// A node is kept when:
    /// 1. Its path matches at least one include pattern (or none were given), or
    ///    any of its descendants match an include pattern (ancestor retention).
    /// 2. Its path does **not** match any exclude pattern.
    pub fn apply(&self, nodes: Vec<DisplayNode>) -> Vec<DisplayNode> {
        apply_inner(nodes, "", self)
    }

    /// Borrow-based variant of [`Self::apply`].
    ///
    /// Avoids cloning nodes that are pruned by the filter; only surviving nodes
    /// are allocated. Prefer this over `apply(nodes.clone())` in render paths.
    pub fn apply_ref(&self, nodes: &[DisplayNode]) -> Vec<DisplayNode> {
        apply_inner_ref(nodes, "", self)
    }
}

fn apply_inner(nodes: Vec<DisplayNode>, prefix: &str, filter: &FieldFilter) -> Vec<DisplayNode> {
    nodes
        .into_iter()
        .filter_map(|mut node| {
            let path = if prefix.is_empty() {
                node.key.clone()
            } else {
                format!("{}.{}", prefix, node.key)
            };
            let lpath = path.to_lowercase();

            // Exclude wins unconditionally.
            if filter.exclude_paths.iter().any(|p| p.matches(&lpath)) {
                return None;
            }

            let self_included = filter.include_paths.is_empty()
                || filter.include_paths.iter().any(|p| p.matches(&lpath));

            // Recursively filter children before deciding on the parent.
            node.children = apply_inner(node.children, &path, filter);

            // Keep if self matches OR has surviving children (ancestor retention
            // when include patterns are active).
            let kept =
                self_included || (!filter.include_paths.is_empty() && !node.children.is_empty());

            if kept {
                Some(node)
            } else {
                None
            }
        })
        .collect()
}

/// Borrow-based recursive filter: clones only nodes that survive the filter.
fn apply_inner_ref(nodes: &[DisplayNode], prefix: &str, filter: &FieldFilter) -> Vec<DisplayNode> {
    nodes
        .iter()
        .filter_map(|node| {
            let path = if prefix.is_empty() {
                node.key.clone()
            } else {
                format!("{}.{}", prefix, node.key)
            };
            let lpath = path.to_lowercase();

            if filter.exclude_paths.iter().any(|p| p.matches(&lpath)) {
                return None;
            }

            let self_included = filter.include_paths.is_empty()
                || filter.include_paths.iter().any(|p| p.matches(&lpath));

            let children = apply_inner_ref(&node.children, &path, filter);

            let kept = self_included || (!filter.include_paths.is_empty() && !children.is_empty());

            if kept {
                Some(DisplayNode {
                    key: node.key.clone(),
                    value: node.value.clone(),
                    children,
                })
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::manifest::tree::{DisplayNode, NodeValue};

    fn make_sample_tree() -> Vec<DisplayNode> {
        vec![
            DisplayNode {
                key: "Claim".into(),
                value: NodeValue::Missing,
                children: vec![DisplayNode {
                    key: "title".into(),
                    value: NodeValue::Str("test".into()),
                    children: vec![],
                }],
            },
            DisplayNode {
                key: "Assertions".into(),
                value: NodeValue::Missing,
                children: vec![DisplayNode {
                    key: "c2pa.actions".into(),
                    value: NodeValue::Missing,
                    children: vec![],
                }],
            },
            DisplayNode {
                key: "Claim Signature".into(),
                value: NodeValue::Missing,
                children: vec![],
            },
        ]
    }

    prop_compose! {
        fn arb_node()(key in "[a-z]{1,12}") -> DisplayNode {
            DisplayNode {
                key,
                value: NodeValue::Str("value".into()),
                children: vec![],
            }
        }
    }

    #[test]
    fn query_exceeding_max_length_returns_error() {
        let long = "a".repeat(257);
        assert!(FieldFilter::from_query(&long).is_err());
    }

    #[test]
    fn token_exceeding_max_length_returns_error() {
        let long_token = "a".repeat(129);
        assert!(FieldFilter::from_query(&long_token).is_err());
    }

    #[test]
    fn token_with_excess_alternation_depth_returns_error() {
        // 5 opening braces — should be rejected
        assert!(FieldFilter::from_query("a.{b.{c.{d.{e.{f}}}}}").is_err());
    }

    #[test]
    fn include_only_assertions() {
        let f = FieldFilter::from_query("assertions.*").unwrap();
        let nodes = make_sample_tree();
        let filtered = f.apply(nodes);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].key, "Assertions");
    }

    #[test]
    fn exclude_pattern_removes_node() {
        let f = FieldFilter::from_query("**;!Claim Signature").unwrap();
        let nodes = make_sample_tree();
        let filtered = f.apply(nodes);
        assert!(!filtered.iter().any(|n| n.key == "Claim Signature"));
    }

    #[test]
    fn empty_filter_returns_all_nodes() {
        let f = FieldFilter::default();
        let nodes = make_sample_tree();
        let count = nodes.len();
        assert_eq!(f.apply(nodes).len(), count);
    }

    proptest! {
        #[test]
        fn filter_never_adds_nodes(
            include in "[a-z.*]+",
            nodes in proptest::collection::vec(arb_node(), 0..10)
        ) {
            let f = FieldFilter::from_query(&include).unwrap_or_default();
            let filtered = f.apply(nodes.clone());
            prop_assert!(filtered.len() <= nodes.len());
        }

        #[test]
        fn filter_with_wildcard_includes_all(
            nodes in proptest::collection::vec(arb_node(), 1..10)
        ) {
            let f = FieldFilter::from_query("**").unwrap();
            prop_assert_eq!(f.apply(nodes.clone()).len(), nodes.len());
        }

        #[test]
        fn apply_ref_matches_apply(
            include in "[a-z.*]+",
            nodes in proptest::collection::vec(arb_node(), 0..10)
        ) {
            let f = FieldFilter::from_query(&include).unwrap_or_default();
            let via_apply = f.apply(nodes.clone());
            let via_ref   = f.apply_ref(&nodes);
            prop_assert_eq!(via_apply.len(), via_ref.len());
            for (a, b) in via_apply.iter().zip(via_ref.iter()) {
                prop_assert_eq!(&a.key, &b.key);
            }
        }
    }
}
