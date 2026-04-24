/// Side-by-side comparison pane widget.
pub mod compare;
/// Manifest detail pane widget.
pub mod detail;
/// File-list pane widget.
pub mod file_list;
/// Field filter bar overlay.
pub mod filter_bar;
/// Application layout calculation.
pub mod layout;
/// Fuzzy search bar overlay.
pub mod search_bar;
/// Status bar widget.
pub mod status_bar;

use ratatui::Frame;

use crate::app::{App, AppState};
use crate::ui::layout::{centered_popup, CachedLayout};

/// Render the full TUI layout for one frame.
pub fn draw(frame: &mut Frame, app: &mut App) {
    // Reuse cached layout rects; recompute only when the terminal area changed.
    let area = frame.area();
    let layout = match app.layout_cache {
        Some((cached_area, ref l)) if cached_area == area => *l,
        _ => {
            let l = CachedLayout::compute(area, app.config.left_pane_pct);
            app.layout_cache = Some((area, l));
            l
        }
    };

    file_list::draw(frame, layout.list_area, app);
    detail::draw(frame, layout.detail_area, app);
    status_bar::draw(frame, layout.status_area, app);

    // Overlays drawn last (on top of everything else).
    // Borrow app.state to avoid cloning heap-allocated String fields.
    match &app.state {
        AppState::Searching { .. } => search_bar::draw(frame, area, app),
        AppState::Filtering { .. } => filter_bar::draw(frame, area, app),
        AppState::Comparing => compare::draw(frame, layout.detail_area, app),
        AppState::Error { message } => draw_error_overlay(frame, area, message),
        AppState::Browse => {}
    }

    if app.show_help {
        draw_help_overlay(frame, area);
    }
}

/// Key-binding rows shown in the help overlay.
///
/// Static slice of `&str` — converted to `Line`s at render time so ratatui
/// owns no heap allocation between frames.
const HELP_LINES: &[&str] = &[
    "Key bindings",
    "",
    "↑/↓ or j/k    Navigate file list",
    "Enter          Load selected file",
    "r              Reload (force re-fetch)",
    "Tab            Switch focus (list ↔ detail)",
    "Space          Expand/collapse tree node",
    "/              Open search bar",
    "f              Open filter bar",
    "c              Mark for compare (press twice on different files)",
    "Esc            Cancel / close overlay",
    "a              (Compare) toggle equal rows",
    "?              Toggle this help",
    "q / Ctrl+C     Quit",
];

/// Render the help overlay with key bindings.
fn draw_help_overlay(frame: &mut Frame, area: ratatui::layout::Rect) {
    use ratatui::text::Line;
    use ratatui::widgets::{Block, Borders, Paragraph};

    let popup = centered_popup(area, 60, 70);
    let lines: Vec<Line> = HELP_LINES.iter().map(|s| Line::from(*s)).collect();
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title("Help")),
        popup,
    );
}

/// Render a modal error overlay.  `message` is already formatted for display.
fn draw_error_overlay(frame: &mut Frame, area: ratatui::layout::Rect, message: &str) {
    use ratatui::style::{Color, Style};
    use ratatui::widgets::{Block, Borders, Paragraph};

    let popup_area = centered_popup(area, 60, 20);
    frame.render_widget(
        Paragraph::new(message).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Error")
                .border_style(Style::default().fg(Color::Red)),
        ),
        popup_area,
    );
}
