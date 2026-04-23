# Spec 07 — Search Bar & Filter Bar Overlays

**Phase:** 2 (concurrent with spec-06, spec-08)  
**Depends on:** spec-00 through spec-05 merged; additionally needs:
- `Matcher`, `MatchResult` from spec-03
- `FieldFilter` from spec-01
- `AppState::Searching`, `AppState::Filtering` from spec-05
- `centered_popup` layout helper from spec-05

**Produces:** `ui/search_bar.rs` and `ui/filter_bar.rs` fully implemented;
search result highlighting wired into the detail pane's `apply_filter_and_search`;
`App` extended with match result cache.

---

## Goal

Implement two modal overlays that float over the detail pane:

1. **Search bar** — fuzzy/substring search within the current manifest's fields.
   Highlights matching nodes in the detail tree and scrolls to the first match.

2. **Filter bar** — glob-pattern field filter. Shows a preview of which top-level
   sections will survive the filter before the user commits with Enter.

Both overlays share the same visual pattern: a centered popup with a text input
line and a results area below it.

---

## `App` additions (add to `src/app.rs`)

```rust
// Current search matches for the active query — updated as the user types
pub search_results: Vec<c2pa_tui::search::MatchResult>,
// Cursor within search_results (for navigating between matches)
pub search_cursor: usize,
```

Update `handle_search_key` in `app.rs` so that every Char/Backspace keystroke
re-runs the matcher:

```rust
fn handle_search_key(&mut self, key: crossterm::event::KeyEvent) {
    use crossterm::event::KeyCode;
    match key.code {
        KeyCode::Esc => {
            self.state = AppState::Browse;
            self.search_results.clear();
            self.search_cursor = 0;
        }
        KeyCode::Char(c) => {
            if let AppState::Searching { query } = &mut self.state {
                query.push(c);
            }
            self.reindex_and_search();
        }
        KeyCode::Backspace => {
            if let AppState::Searching { query } = &mut self.state {
                query.pop();
            }
            self.reindex_and_search();
        }
        KeyCode::Down | KeyCode::Tab => {
            if !self.search_results.is_empty() {
                self.search_cursor = (self.search_cursor + 1) % self.search_results.len();
            }
        }
        KeyCode::Up => {
            if !self.search_results.is_empty() {
                self.search_cursor = self.search_cursor
                    .checked_sub(1)
                    .unwrap_or(self.search_results.len() - 1);
            }
        }
        _ => {}
    }
}

fn reindex_and_search(&mut self) {
    if let Some(nodes) = self.loaded.get(&self.selected_left) {
        let flat = c2pa_tui::manifest::tree::flatten(nodes);
        self.matcher.index(&flat);
        let query = if let AppState::Searching { query } = &self.state {
            query.clone()
        } else {
            String::new()
        };
        self.search_results = self.matcher.query(&query);
        self.search_cursor = 0;
    }
}
```

---

## `src/ui/search_bar.rs`

The search bar is a floating popup rendered on top of everything else.

Layout:
```
┌─ Search ───────────────────────────────────┐
│ > jpeg_                                    │
├────────────────────────────────────────────┤
│  format: image/jpeg               score:98 │
│  assertions.c2pa.hash.data alg: sha256     │
│  …                                         │
└────────────────────────────────────────────┘
```

```rust
use ratatui::{
    Frame, layout::Rect,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use crate::app::{App, AppState};
use crate::ui::layout::centered_popup;

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let popup_area = centered_popup(area, 70, 50);

    // Split popup: 3 lines for input, rest for results
    use ratatui::layout::{Layout, Direction, Constraint};
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(popup_area);

    let query = if let AppState::Searching { query } = &app.state { query.as_str() } else { "" };

    // Input box
    frame.render_widget(
        Paragraph::new(format!("> {}", query))
            .block(Block::default().borders(Borders::ALL).title("Search")
                .border_style(Style::default().fg(Color::Yellow))),
        chunks[0],
    );

    // Results list with highlight spans
    let flat_nodes = app.loaded.get(&app.selected_left)
        .map(|n| crate::manifest::tree::flatten(n))
        .unwrap_or_default();

    let items: Vec<ListItem> = app.search_results.iter().enumerate().map(|(i, result)| {
        let node = &flat_nodes[result.node_index];
        let style = if i == app.search_cursor {
            Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };

        // Build a Line with highlight spans
        let line = build_highlighted_line(&node.display, &result.highlight_ranges, style);
        ListItem::new(line)
    }).collect();

    let mut list_state = ListState::default();
    list_state.select(Some(app.search_cursor));

    frame.render_stateful_widget(
        List::new(items)
            .block(Block::default().borders(Borders::ALL)
                .title(format!("{} matches", app.search_results.len()))),
        chunks[1],
        &mut list_state,
    );
}

/// Build a ratatui Line with match ranges highlighted in yellow bold.
fn build_highlighted_line(
    display: &str,
    ranges: &[std::ops::Range<usize>],
    base_style: Style,
) -> ratatui::text::Line<'static> {
    if ranges.is_empty() {
        return Line::from(Span::styled(display.to_string(), base_style));
    }

    let highlight_style = base_style.fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let mut spans = Vec::new();
    let mut last = 0usize;
    let bytes = display.as_bytes();

    for range in ranges {
        // Text before this match
        if last < range.start {
            spans.push(Span::styled(display[last..range.start].to_string(), base_style));
        }
        // Matched text
        if range.start < range.end && range.end <= display.len() {
            spans.push(Span::styled(display[range.start..range.end].to_string(), highlight_style));
        }
        last = range.end;
    }
    // Text after last match
    if last < display.len() {
        spans.push(Span::styled(display[last..].to_string(), base_style));
    }

    Line::from(spans)
}
```

---

## `src/ui/filter_bar.rs`

The filter bar shows the current glob query and a live preview of which top-level
nodes will survive after the filter is applied.

```rust
use ratatui::{
    Frame, layout::Rect,
    widgets::{Block, Borders, List, ListItem, Paragraph},
    style::{Color, Style},
    layout::{Layout, Direction, Constraint},
};
use crate::app::{App, AppState};
use crate::ui::layout::centered_popup;
use crate::manifest::filter::FieldFilter;

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let popup_area = centered_popup(area, 60, 40);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Length(1), Constraint::Min(1)])
        .split(popup_area);

    let query = if let AppState::Filtering { query } = &app.state { query.as_str() } else { "" };

    // Input box
    frame.render_widget(
        Paragraph::new(format!("> {}", query))
            .block(Block::default().borders(Borders::ALL).title("Filter (glob, e.g. assertions.*)")
                .border_style(Style::default().fg(Color::Cyan))),
        chunks[0],
    );

    // Parse preview
    let preview_label = match FieldFilter::from_query(query) {
        Ok(_) => "Preview (Enter to apply, Esc to cancel):".to_string(),
        Err(e) => format!("Invalid pattern: {e}"),
    };
    frame.render_widget(Paragraph::new(preview_label), chunks[1]);

    // Show top-level node keys that would survive the filter
    let preview_items: Vec<ListItem> = if let Ok(filter) = FieldFilter::from_query(query) {
        if let Some(nodes) = app.loaded.get(&app.selected_left) {
            let filtered = filter.apply(nodes.clone());
            filtered.iter().map(|n| {
                ListItem::new(format!("  {} ({})", n.key, n.children.len()))
            }).collect()
        } else {
            vec![ListItem::new("  (no manifest loaded)")]
        }
    } else {
        vec![]
    };

    frame.render_widget(
        List::new(preview_items)
            .block(Block::default().borders(Borders::TOP)),
        chunks[2],
    );
}
```

---

## Wiring search results into detail pane

Extend `apply_filter_and_search` in `src/ui/detail.rs` (from spec-06) to also
filter by search results when a search is active:

```rust
fn apply_filter_and_search(app: &App, nodes: &[DisplayNode]) -> Vec<DisplayNode> {
    use crate::app::AppState;
    use crate::manifest::tree::flatten;

    // 1. Apply field filter
    let filtered = app.filter.apply(nodes.to_vec());

    // 2. If searching with results, retain only nodes whose flat index is in results
    if matches!(&app.state, AppState::Searching { .. }) && !app.search_results.is_empty() {
        let matched_indices: std::collections::HashSet<usize> =
            app.search_results.iter().map(|r| r.node_index).collect();

        // Flat index the filtered tree and keep only nodes in matched_indices.
        // FlatNode::path is the full dot-joined path (e.g. "Claim.title"), which
        // is what we must use for path matching — not just node.key.
        let flat = flatten(&filtered);
        let matched_paths: std::collections::HashSet<String> = flat.iter()
            .filter(|n| matched_indices.contains(&n.node_index))
            .map(|n| n.path.clone())
            .collect();

        prune_to_matches(&filtered, &matched_paths, "")
    } else {
        filtered
    }
}

/// Keep a node if its full dot-path or any descendant's full dot-path is in `keep_paths`.
///
/// `prefix` is the dot-joined path of the current node's ancestors, used to
/// reconstruct the full path at each level. This avoids false matches caused by
/// checking only `node.key` (the local name) rather than the full path.
fn prune_to_matches(
    nodes: &[DisplayNode],
    keep_paths: &std::collections::HashSet<String>,
    prefix: &str,
) -> Vec<DisplayNode> {
    let mut result = Vec::new();
    for node in nodes {
        let path = if prefix.is_empty() {
            node.key.clone()
        } else {
            format!("{}.{}", prefix, node.key)
        };
        let children = prune_to_matches(&node.children, keep_paths, &path);
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
```

---

## Snapshot tests (add to `tests/snapshot_ui.rs`)

```rust
#[test]
fn search_bar_renders_with_query() {
    let mut app = make_app_with_loaded_manifest();
    app.state = AppState::Searching { query: "jpeg".into() };
    app.reindex_and_search(); // trigger match computation
    let mut terminal = make_test_terminal(100, 30);
    terminal.draw(|f| c2pa_tui::ui::draw(f, &app)).unwrap();
    insta::assert_snapshot!(buffer_to_string(terminal.backend().buffer()));
}

#[test]
fn filter_bar_renders_preview() {
    let mut app = make_app_with_loaded_manifest();
    app.state = AppState::Filtering { query: "assertions.*".into() };
    let mut terminal = make_test_terminal(100, 30);
    terminal.draw(|f| c2pa_tui::ui::draw(f, &app)).unwrap();
    insta::assert_snapshot!(buffer_to_string(terminal.backend().buffer()));
}

#[test]
fn filter_bar_shows_error_for_invalid_glob() {
    let mut app = make_app_with_loaded_manifest();
    app.state = AppState::Filtering { query: "[invalid".into() };
    let mut terminal = make_test_terminal(100, 30);
    terminal.draw(|f| c2pa_tui::ui::draw(f, &app)).unwrap();
    insta::assert_snapshot!(buffer_to_string(terminal.backend().buffer()));
}
```

---

## Done criteria

```
cargo build
cargo test --test snapshot_ui
cargo fmt -- --check
cargo clippy -- -D warnings
```

Manual verification: run the binary, press `/`, type a field name, see highlights
appear in the results list. Press `f`, type `assertions.*`, see preview show only
the Assertions node, press Enter, see the detail pane prune to only assertions.
