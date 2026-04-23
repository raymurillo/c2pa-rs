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

use crate::app::App;

/// Render the full TUI layout for one frame.
///
/// Stub: not yet implemented. Implemented in spec-06.
pub fn draw(_frame: &mut Frame, _app: &App) {
    todo!("spec-06: implement draw()")
}
