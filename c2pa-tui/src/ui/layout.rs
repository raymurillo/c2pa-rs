use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Pre-computed layout rects for one terminal frame.
///
/// Stored in `App::layout_cache` and recomputed only when the terminal is
/// resized, avoiding repeated `Layout::split` allocations every frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CachedLayout {
    pub main_area: Rect,
    pub status_area: Rect,
    pub list_area: Rect,
    pub detail_area: Rect,
}

impl CachedLayout {
    /// Compute layout rects for `area` using `left_pane_pct` for the split.
    pub fn compute(area: Rect, left_pane_pct: u16) -> Self {
        let (main_area, status_area) = split_status(area);
        let (list_area, detail_area) = split_horizontal(main_area, left_pane_pct);
        Self {
            main_area,
            status_area,
            list_area,
            detail_area,
        }
    }
}

/// Horizontal split: left pane (file list) + right pane (detail/compare).
pub fn split_horizontal(area: Rect, left_pct: u16) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(left_pct),
            Constraint::Percentage(100 - left_pct),
        ])
        .split(area);
    (chunks[0], chunks[1])
}

/// Vertical split: main area + status bar (1 line).
pub fn split_status(area: Rect) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);
    (chunks[0], chunks[1])
}

/// Centered floating rect for overlays (search bar, filter bar, error).
/// `width_pct` and `height_pct` are percentages of `area`.
pub fn centered_popup(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_pct) / 2),
            Constraint::Percentage(width_pct),
            Constraint::Percentage((100 - width_pct) / 2),
        ])
        .split(area);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height_pct) / 2),
            Constraint::Percentage(height_pct),
            Constraint::Percentage((100 - height_pct) / 2),
        ])
        .split(horizontal[1]);
    vertical[1]
}

/// Centered floating rect sized to an exact number of content lines.
///
/// Unlike [`centered_popup`], which uses a height percentage that can clip
/// content on short terminals, this variant uses `Constraint::Length` so the
/// popup is exactly `height_lines + 2` rows tall (content + top/bottom border)
/// and is clamped to `area.height`.  The vertical margins absorb the
/// remainder, keeping the popup centered.
pub fn centered_popup_exact(area: Rect, width_pct: u16, height_lines: u16) -> Rect {
    // +2 for the top and bottom border rows drawn by the widget's Block.
    let total_height = (height_lines + 2).min(area.height);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_pct) / 2),
            Constraint::Percentage(width_pct),
            Constraint::Percentage((100 - width_pct) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(total_height),
            Constraint::Min(0),
        ])
        .split(horizontal[1])[1]
}
