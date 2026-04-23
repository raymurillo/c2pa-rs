# Spec 06 — File List Pane, Detail Pane & Status Bar

**Phase:** 2 (concurrent with spec-07, spec-08)  
**Depends on:** spec-00 through spec-05 merged; specifically:
- `App`, `AppState`, `Pane` from spec-05
- `DisplayNode`, `NodeValue` from spec-01
- `ManifestSource` from spec-01
- `layout::split_horizontal`, `split_status` from spec-05

**Produces:** `ui/file_list.rs`, `ui/detail.rs`, `ui/status_bar.rs` fully implemented;
`App::handle_mouse` filled in for pane-level mouse events.

---

## Goal

Implement the two primary display panes and the bottom status bar. The file list
shows all loaded sources with their load state. The detail pane renders the
`DisplayNode` tree for the selected source using `tui-tree-widget`. The status bar
shows context-sensitive key hints.

---

## Files to modify

- `src/ui/file_list.rs`
- `src/ui/detail.rs`
- `src/ui/status_bar.rs`
- `src/app.rs` — fill in `handle_mouse`, add `detail_tree_state` field

---

## `src/ui/file_list.rs`

Render a `ratatui::widgets::List` of all sources. Each item shows:
- A status icon: `[✓]` loaded, `[~]` loading, `[!]` error, `[ ]` not loaded
- The source label (truncated to fit)
- For remote sources: `(remote)` suffix

The selected item is highlighted with the focused/unfocused border style.

```rust
use ratatui::{Frame, layout::Rect, widgets::{Block, Borders, List, ListItem, ListState},
    style::{Color, Style, Modifier}};
use crate::app::{App, AppState, Pane};

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let focused = app.focused_pane == Pane::FileList;
    let border_style = if focused {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let items: Vec<ListItem> = app.sources.iter().enumerate().map(|(i, src)| {
        let icon = match &app.state {
            AppState::Loading { source_index } if *source_index == i => "[~]",
            _ if app.loaded.contains_key(&i) => "[✓]",
            _ => "[ ]",
        };
        // Check error state: if the loaded entry contains a single error node, show [!]
        // For simplicity, use "[✓]" for all loaded entries — error nodes are shown in detail pane
        let suffix = if src.is_remote() { " (remote)" } else { "" };
        let label = format!("{} {}{}", icon, src.label(), suffix);
        let style = if i == app.selected_left {
            Style::default().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        ListItem::new(label).style(style)
    }).collect();

    let mut list_state = ListState::default();
    list_state.select(Some(app.selected_left));

    let list = List::new(items)
        .block(Block::default()
            .borders(Borders::ALL)
            .title("Files")
            .border_style(border_style))
        .highlight_style(Style::default().bg(Color::DarkGray));

    frame.render_stateful_widget(list, area, &mut list_state);
}
```

---

## `src/ui/detail.rs`

Use `tui-tree-widget` to render the `DisplayNode` tree for `app.loaded[app.selected_left]`.

### Mapping `DisplayNode` to `tui_tree_widget::TreeItem`

Write a helper:

```rust
fn node_to_tree_item<'a>(node: &'a DisplayNode) -> tui_tree_widget::TreeItem<'a, String> {
    let text = match &node.value {
        NodeValue::Missing => node.key.clone(),
        other => format!("{}: {}", node.key, other.as_str()),
    };

    if node.children.is_empty() {
        tui_tree_widget::TreeItem::new_leaf(node.key.clone(), text)
    } else {
        let children: Vec<_> = node.children.iter()
            .map(node_to_tree_item)
            .collect();
        tui_tree_widget::TreeItem::new(node.key.clone(), text, children)
            .expect("unique keys within siblings")
    }
}
```

> **Note:** `tui-tree-widget` requires unique identifiers within a sibling list.
> If two sibling nodes share the same key (e.g. multiple `[0]`, `[1]` in an array),
> the identifier must be unique. Use `format!("{}_{}", node.key, index)` as the
> identifier if collisions are possible. The `display` text can still show just the key.

### `App` additions needed (add to `src/app.rs`)

Add a `detail_tree_state: tui_tree_widget::TreeState<String>` field to `App`.
Initialize in `App::new` with `TreeState::default()`.

When a new source is loaded (`handle_load_result`), call
`self.detail_tree_state = TreeState::default()` to reset the expand/collapse state.

Also add Space key handling in Browse mode to expand/collapse:

```rust
KeyCode::Char(' ') => {
    // Toggle selected tree node — tui-tree-widget handles this via TreeState
    self.detail_tree_state.toggle_selected();
}
```

### `draw` function

```rust
use ratatui::{Frame, layout::Rect, widgets::{Block, Borders}, style::{Color, Style}};
use tui_tree_widget::Tree;
use crate::app::{App, Pane};
use crate::manifest::tree::DisplayNode;

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let focused = app.focused_pane == Pane::Detail;
    let border_style = if focused {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let title = app.sources.get(app.selected_left)
        .map(|s| s.label().to_string())
        .unwrap_or_else(|| "Detail".into());

    let nodes = app.loaded.get(&app.selected_left)
        .map(|n| apply_filter_and_search(app, n))
        .unwrap_or_default();

    let items: Vec<_> = nodes.iter().map(node_to_tree_item).collect();

    let tree = Tree::new(&items)
        .expect("tree items")
        .block(Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(border_style))
        .highlight_style(Style::default().bg(Color::DarkGray));

    frame.render_stateful_widget(tree, area, &mut app.detail_tree_state.clone());
    // Note: render_stateful_widget requires `&mut state`. Since `app` is `&App`,
    // you'll need to either make `detail_tree_state` interior-mutable (RefCell)
    // or change the draw signature to take `&mut App`. Use `RefCell` to keep
    // the draw functions consistent.
}
```

> **Design note:** `tui_tree_widget::Tree::render_stateful_widget` needs `&mut TreeState`.
> Since `draw(frame, area, app: &App)` takes a shared reference, wrap `detail_tree_state`
> in `RefCell<TreeState<String>>` in the `App` struct so it can be mutably borrowed
> during rendering without changing the `draw` signature to `&mut App`.

### `apply_filter_and_search`

```rust
fn apply_filter_and_search(app: &App, nodes: &[DisplayNode]) -> Vec<DisplayNode> {
    // 1. Apply field filter
    let filtered = app.filter.apply(nodes.to_vec());

    // 2. If searching, further filter to only nodes that have a match
    //    (spec-07 will wire in the Matcher; for now just return filtered)
    filtered
}
```

The search highlighting integration is handled by spec-07's search bar overlay.
This function just does filtering for now — spec-07 will extend it.

---

## `src/ui/status_bar.rs`

Render a single line of context-sensitive key hints.

```rust
use ratatui::{Frame, layout::Rect, widgets::Paragraph, style::{Color, Style}};
use crate::app::{App, AppState};

pub fn draw(frame: &mut Frame, area: Rect, app: &App) {
    let hints = match &app.state {
        AppState::Browse => "↑/↓:nav  Enter:load  Tab:focus  /:search  f:filter  c:compare  r:reload  ?:help  q:quit",
        AppState::Searching { .. } => "Type to search  Esc:cancel",
        AppState::Filtering { .. } => "Type glob filter (e.g. assertions.*)  Enter:apply  Esc:cancel",
        AppState::Comparing => "Comparing  Esc:exit compare",
        AppState::Loading { .. } => "Loading…  q:quit",
        AppState::Error { .. } => "Error — press any key to dismiss",
    };

    frame.render_widget(
        Paragraph::new(hints)
            .style(Style::default().fg(Color::DarkGray)),
        area,
    );
}
```

---

## Mouse handling — `App::handle_mouse`

Fill in the `handle_mouse` stub in `src/app.rs`:

```rust
pub fn handle_mouse(&mut self, event: crossterm::event::MouseEvent) {
    use crossterm::event::{MouseEventKind, MouseButton};

    match event.kind {
        MouseEventKind::ScrollDown => {
            match self.focused_pane {
                Pane::FileList => {
                    self.selected_left = (self.selected_left + 1)
                        .min(self.sources.len().saturating_sub(1));
                }
                Pane::Detail => {
                    // tui-tree-widget scroll — see note below
                    self.detail_tree_state.borrow_mut().scroll_down(1);
                }
            }
        }
        MouseEventKind::ScrollUp => {
            match self.focused_pane {
                Pane::FileList => {
                    self.selected_left = self.selected_left.saturating_sub(1);
                }
                Pane::Detail => {
                    self.detail_tree_state.borrow_mut().scroll_up(1);
                }
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            // Determine which pane was clicked by comparing event.column
            // against the layout split. This requires knowing the terminal size.
            // Use app.last_terminal_size (add this field) to track it.
            // Simplified: use column < (terminal_width * left_pane_pct / 100)
            if let Some((w, _)) = self.last_terminal_size {
                let split = w as u16 * self.config.left_pane_pct / 100;
                if event.column < split {
                    self.focused_pane = Pane::FileList;
                    // Calculate row within the list (subtract border)
                    let row = event.row.saturating_sub(1) as usize;
                    if row < self.sources.len() {
                        self.selected_left = row;
                    }
                } else {
                    self.focused_pane = Pane::Detail;
                }
            }
        }
        _ => {}
    }
}
```

Add `last_terminal_size: Option<(u16, u16)>` to `App`. Update it in `event_loop`
on `Event::Resize(w, h)` and after each `terminal.draw()`.

---

## Snapshot tests (add to `tests/snapshot_ui.rs`)

Use `ratatui::backend::TestBackend` for deterministic buffer rendering.

```rust
use ratatui::{Terminal, backend::TestBackend};
use c2pa_tui::app::{App, AppState};
use c2pa_tui::config::Config;

fn make_test_terminal(width: u16, height: u16) -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(width, height)).unwrap()
}

#[test]
fn file_list_renders_single_item() {
    let mut app = App::new(Config::default()).unwrap();
    // Add a fake source
    // ... (use a TestSource that implements ManifestSource)
    let mut terminal = make_test_terminal(80, 24);
    terminal.draw(|f| c2pa_tui::ui::draw(f, &app)).unwrap();
    let buffer = terminal.backend().buffer().clone();
    insta::assert_snapshot!(buffer_to_string(&buffer));
}

#[test]
fn file_list_shows_loading_indicator() {
    let mut app = App::new(Config::default()).unwrap();
    // Set state to Loading for source 0
    app.state = AppState::Loading { source_index: 0 };
    // ... render and snapshot
}
```

Use `MockManifestSource` from `mockall` (generated by `#[automock]` on the trait
in spec-00) rather than a hand-rolled `TestSource`. Example:

```rust
use c2pa_tui::manifest::loader::MockManifestSource;

fn make_mock_source(label: &str, nodes: Vec<DisplayNode>) -> MockManifestSource {
    let mut src = MockManifestSource::new();
    let label = label.to_string();
    src.expect_label().return_const(label);
    src.expect_is_remote().return_const(false);
    src.expect_load().return_once(move |_| Ok(nodes));
    src
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

Visual verification: run `cargo run` with a sample file path; the file list should
show the file, and after Enter the detail pane should render the manifest tree.
