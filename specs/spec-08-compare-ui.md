# Spec 08 — Compare View

**Phase:** 2 (concurrent with spec-06, spec-07)  
**Depends on:** spec-00 through spec-05 merged; additionally needs:
- `ManifestDiff`, `FieldDiff`, `diff()` from spec-04
- `AppState::Comparing`, `App::compare_selection` from spec-05
- `split_horizontal` from spec-05

**Produces:** `ui/compare.rs` fully implemented; `App` extended with diff caching;
compare key flow fully wired.

---

## Goal

Implement the side-by-side compare view. When the user presses `c` on a first
source and then `c` on a second, the right pane switches from the tree view to a
two-column diff view showing field-level differences between the two manifests.

---

## Files to modify

- `src/ui/compare.rs`
- `src/app.rs` — add diff cache, extend `c` key flow

---

## `App` additions (add to `src/app.rs`)

```rust
// Cached diff result; invalidated when selected sources change or reload
pub cached_diff: Option<crate::compare::ManifestDiff>,
```

Update `handle_browse_key` for the `c` key:

```rust
KeyCode::Char('c') => {
    match self.compare_selection {
        None => {
            // First press: mark current source as the left side
            self.compare_selection = Some(self.selected_left);
        }
        Some(left_idx) if left_idx == self.selected_left => {
            // Pressed c on the same source twice: cancel
            self.compare_selection = None;
        }
        Some(left_idx) => {
            // Second press on a different source: compute diff and enter Comparing state
            let right_idx = self.selected_left;
            self.cached_diff = self.compute_diff(left_idx, right_idx);
            if self.cached_diff.is_some() {
                self.state = AppState::Comparing;
            } else {
                self.state = AppState::Error {
                    message: "One or both sources are not loaded. Press Enter to load them first.".into()
                };
            }
        }
    }
}
```

### `compute_diff`

```rust
fn compute_diff(&self, left_idx: usize, right_idx: usize) -> Option<crate::compare::ManifestDiff> {
    let left_nodes = self.loaded.get(&left_idx)?;
    let right_nodes = self.loaded.get(&right_idx)?;
    let left_label = self.sources.get(left_idx).map(|s| s.label()).unwrap_or("left");
    let right_label = self.sources.get(right_idx).map(|s| s.label()).unwrap_or("right");
    Some(crate::compare::diff(left_label, left_nodes, right_label, right_nodes))
}
```

Invalidate `cached_diff` in `handle_load_result` when the newly loaded index is
`compare_selection` or `selected_left`:

```rust
// In handle_load_result, after inserting into self.loaded:
if self.compare_selection == Some(idx) || self.selected_left == idx {
    self.cached_diff = None;
}
```

---

## `src/ui/compare.rs`

Replace the right pane with a two-column layout. Each column has a header showing
the source label, and the rows below show fields colour-coded by diff status.

### Layout

```
┌── left-file.jpg ──────┬── right-file.jpg ──────┐
│ Claim.title: Photo A  │ Claim.title: Photo B    │  ← Changed (yellow)
│ Claim.format: jpeg    │ Claim.format: jpeg      │  ← Equal   (default)
│ assertions.c2pa.hash  │ assertions.c2pa.hash    │  ← Equal
│ Claim.instance_id: … ←│                         │  ← OnlyLeft (red)
│                        │ Validation.status: inv. │  ← OnlyRight (green)
└────────────────────────┴─────────────────────────┘
  5 differences
```

### Colour scheme

| Diff type | Left column style | Right column style |
|---|---|---|
| `Equal` | `Color::DarkGray` dim | `Color::DarkGray` dim |
| `Changed` | `Color::Yellow` | `Color::Yellow` |
| `OnlyLeft` | `Color::Red` | empty string, dim |
| `OnlyRight` | empty string, dim | `Color::Green` |

### Show only differences by default; toggle to show all with `a` key

Add `compare_show_all: bool` to `App` (default `false`). In `handle_compare_key`:

```rust
KeyCode::Char('a') => {
    self.compare_show_all = !self.compare_show_all;
}
```

When `compare_show_all` is false, filter the diff fields to exclude `Equal` entries
before rendering.

### `draw` function

```rust
use ratatui::{
    Frame, layout::Rect,
    widgets::{Block, Borders, Table, Row, Cell},
    style::{Color, Style, Modifier},
    layout::{Constraint},
};
use crate::app::App;
use crate::compare::FieldDiff;

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let diff = match &app.cached_diff {
        Some(d) => d,
        None => {
            // Nothing to show — render placeholder
            frame.render_widget(
                ratatui::widgets::Paragraph::new("Select two files with 'c' to compare.")
                    .block(ratatui::widgets::Block::default().borders(Borders::ALL).title("Compare")),
                area,
            );
            return;
        }
    };

    let fields: Vec<&FieldDiff> = if app.compare_show_all {
        diff.fields.iter().collect()
    } else {
        diff.differences().collect()
    };

    let rows: Vec<Row> = fields.iter().map(|field| {
        let (path, left_text, right_text, style) = match field {
            FieldDiff::Equal { path, value } =>
                (path.as_str(), value.as_str(), value.as_str(),
                 Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM)),
            FieldDiff::Changed { path, left, right } =>
                (path.as_str(), left.as_str(), right.as_str(),
                 Style::default().fg(Color::Yellow)),
            FieldDiff::OnlyLeft { path, value } =>
                (path.as_str(), value.as_str(), "",
                 Style::default().fg(Color::Red)),
            FieldDiff::OnlyRight { path, value } =>
                (path.as_str(), "", value.as_str(),
                 Style::default().fg(Color::Green)),
        };
        Row::new(vec![
            Cell::from(path.to_string()),
            Cell::from(truncate(left_text, 35)).style(style),
            Cell::from(truncate(right_text, 35)).style(style),
        ])
    }).collect();

    let diff_count = diff.diff_count();
    let show_toggle = if app.compare_show_all { "a:hide equal" } else { "a:show all" };
    let title = format!(
        "Compare — {} difference{}  {show_toggle}  Esc:exit",
        diff_count,
        if diff_count == 1 { "" } else { "s" }
    );

    let table = Table::new(rows, [
        Constraint::Percentage(30),
        Constraint::Percentage(35),
        Constraint::Percentage(35),
    ])
    .header(Row::new(vec![
        Cell::from("Field").style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from(truncate(&diff.left_label, 35)).style(Style::default().add_modifier(Modifier::BOLD)),
        Cell::from(truncate(&diff.right_label, 35)).style(Style::default().add_modifier(Modifier::BOLD)),
    ]))
    .block(Block::default().borders(Borders::ALL).title(title));

    frame.render_widget(table, area);
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
}
```

---

## Snapshot tests (add to `tests/snapshot_ui.rs`)

```rust
#[test]
fn compare_view_shows_differences() {
    let mut app = make_app_with_two_loaded_manifests();
    // Ensure compare_selection is set and state is Comparing
    app.compare_selection = Some(0);
    app.state = AppState::Comparing;
    app.cached_diff = Some(c2pa_tui::compare::diff(
        "file_a.jpg",
        &make_manifest_a_nodes(),
        "file_b.jpg",
        &make_manifest_b_nodes(),
    ));
    let mut terminal = make_test_terminal(120, 30);
    terminal.draw(|f| c2pa_tui::ui::draw(f, &app)).unwrap();
    insta::assert_snapshot!(buffer_to_string(terminal.backend().buffer()));
}

#[test]
fn compare_view_shows_all_when_toggled() {
    // Same as above but with compare_show_all = true
    // Equal rows should also appear, dimmed
}

#[test]
fn compare_view_shows_placeholder_when_no_diff() {
    let mut app = App::new(Config::default()).unwrap();
    app.state = AppState::Comparing;
    app.cached_diff = None;
    let mut terminal = make_test_terminal(120, 30);
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

Manual verification: load two different JPEG files, press `c` on each, confirm the
compare table appears with differences highlighted. Press `a` to toggle equal rows.
Press `Esc` to return to Browse mode.
