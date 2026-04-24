use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders},
    Frame,
};
use tui_tree_widget::{Tree, TreeItem};

use crate::app::{App, LoadState, Pane};
use crate::manifest::filter::FieldFilter;
use crate::manifest::tree::{DisplayNode, NodeValue};

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

/// Apply the active field filter, borrowing `nodes` directly when the filter
/// is the default pass-through to avoid a full tree clone every frame.
fn filtered_nodes(filter: &FieldFilter, nodes: &[DisplayNode]) -> Option<Vec<DisplayNode>> {
    if filter.include_paths.is_empty() && filter.exclude_paths.is_empty() {
        None // borrow the original slice; no allocation
    } else {
        Some(filter.apply(nodes.to_vec()))
    }
}

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let focused = app.focused_pane == Pane::Detail;
    let border_style = if focused {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    // Borrow label as &str to avoid a heap allocation on every frame.
    let title: &str = app
        .sources
        .get(app.selected_left)
        .map(|s| s.label())
        .unwrap_or("Detail");

    // Split the borrow: `raw` comes from `app.loaded`, `filter_buf` (if
    // needed) is computed from `app.filter`.  Two different fields, so the
    // borrow checker is happy even though `app` is `&mut`.
    let raw_nodes = match app.loaded.get(&app.selected_left) {
        Some(LoadState::Loaded(nodes)) => Some(nodes.as_slice()),
        _ => None,
    };

    let filter_buf;
    let nodes: &[DisplayNode] = match raw_nodes {
        None => &[],
        Some(raw) => match filtered_nodes(&app.filter, raw) {
            None => raw,
            Some(filtered) => {
                filter_buf = filtered;
                &filter_buf
            }
        },
    };

    let items: Vec<_> = nodes
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
