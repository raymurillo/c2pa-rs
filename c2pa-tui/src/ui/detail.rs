use std::collections::HashSet;

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders},
    Frame,
};
use tui_tree_widget::{Tree, TreeItem};

use crate::app::{App, AppState, LoadState, Pane};
use crate::manifest::filter::FieldFilter;
use crate::manifest::tree::{filter_empty_nodes, flatten, DisplayNode, NodeValue};

/// Convert a `DisplayNode` to a `TreeItem`, using a dot-joined path as the
/// identifier so that expand/collapse state survives tree rebuilds.
fn node_to_tree_item<'a>(node: &'a DisplayNode, path_prefix: &str) -> TreeItem<'a, String> {
    let id = if path_prefix.is_empty() {
        node.key.clone()
    } else {
        format!("{}.{}", path_prefix, node.key)
    };

    let text = match &node.value {
        NodeValue::Missing => node.key.clone(),
        other => format!("{}: {}", node.key, other.as_str()),
    };

    if node.children.is_empty() {
        TreeItem::new_leaf(id, text)
    } else {
        let children: Vec<_> = node
            .children
            .iter()
            .map(|child| node_to_tree_item(child, &id))
            .collect();
        // tui-tree-widget requires unique identifiers within a sibling list.
        // Malformed manifests can have duplicate assertion labels; rather than
        // panicking in the render path, fall back to a leaf showing the count.
        TreeItem::new(id.clone(), text.clone(), children).unwrap_or_else(|_| {
            TreeItem::new_leaf(id, format!("{} ({} items)", node.key, node.children.len()))
        })
    }
}

/// Apply the active field filter and optional search narrowing to `nodes`.
///
/// Uses the original unfiltered `nodes` slice for the flat-index lookup so
/// that search result indices (which were computed against `flatten(nodes)`)
/// remain valid even after the field filter has removed some entries.
fn apply_filter_and_search(
    filter: &FieldFilter,
    is_searching: bool,
    search_indices: &HashSet<usize>,
    nodes: &[DisplayNode],
) -> Vec<DisplayNode> {
    let filtered = filter.apply(nodes.to_vec());

    if is_searching && !search_indices.is_empty() {
        // Re-flatten the ORIGINAL nodes so index values align with the search
        // results (which were indexed against the same unfiltered flat array).
        let flat = flatten(nodes);
        let matched_paths: HashSet<String> = flat
            .iter()
            .filter(|n| search_indices.contains(&n.node_index))
            .map(|n| n.path.clone())
            .collect();
        prune_to_matches(&filtered, &matched_paths, "", 0)
    } else {
        filtered
    }
}

/// Keep a node if its full dot-path or any descendant's full dot-path is in
/// `keep_paths`.  `depth` guards against stack overflow on pathological inputs
/// (deeply-nested JSON assertions); trees beyond 256 levels are truncated.
fn prune_to_matches(
    nodes: &[DisplayNode],
    keep_paths: &HashSet<String>,
    prefix: &str,
    depth: usize,
) -> Vec<DisplayNode> {
    if depth > 256 {
        return vec![];
    }
    let mut result = Vec::new();
    for node in nodes {
        let path = if prefix.is_empty() {
            node.key.clone()
        } else {
            format!("{}.{}", prefix, node.key)
        };
        let children = prune_to_matches(&node.children, keep_paths, &path, depth + 1);
        if keep_paths.contains(&path) || !children.is_empty() {
            result.push(DisplayNode {
                key: node.key.clone(),
                value: node.value.clone(),
                children,
            });
        }
    }
    result
}

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let focused = app.focused_pane == Pane::Detail;
    let border_style = if focused {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let title: &str = app
        .sources
        .get(app.selected_left)
        .map(|s| s.label())
        .unwrap_or("Detail");

    // Read search state before borrowing app.loaded so field borrows don't
    // conflict.  search_result_indices is kept in sync by reindex_and_search
    // so there is no per-frame HashSet allocation here.
    let is_searching =
        matches!(&app.state, AppState::Searching { .. }) && !app.search_result_indices.is_empty();
    let has_filter = !app.filter.include_paths.is_empty() || !app.filter.exclude_paths.is_empty();

    let raw_nodes = match app.loaded.get(&app.selected_left) {
        Some(LoadState::Loaded(nodes)) => Some(nodes.as_slice()),
        _ => None,
    };

    // Two optional filter passes are chained here.  Each pass may or may not
    // allocate; uninitialised `MaybeUninit`-style pattern (declare binding
    // before the branch, initialise inside) keeps the borrow checker happy
    // without heap-allocating when neither filter is active.
    let filter_buf;
    let empty_buf;
    let visible_nodes: &[DisplayNode] = {
        let after_field_filter: &[DisplayNode] = match raw_nodes {
            None => &[],
            Some(raw) => {
                if has_filter || is_searching {
                    filter_buf = apply_filter_and_search(
                        &app.filter,
                        is_searching,
                        &app.search_result_indices,
                        raw,
                    );
                    &filter_buf
                } else {
                    raw
                }
            }
        };
        if app.hide_empty {
            empty_buf = filter_empty_nodes(after_field_filter.to_vec());
            &empty_buf
        } else {
            after_field_filter
        }
    };

    let items: Vec<_> = visible_nodes
        .iter()
        .map(|node| node_to_tree_item(node, ""))
        .collect();

    let tree = Tree::new(&items)
        .expect("tree items")
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(border_style),
        )
        .highlight_style(Style::default().bg(Color::DarkGray));

    frame.render_stateful_widget(tree, area, &mut app.detail_tree_state);
}
