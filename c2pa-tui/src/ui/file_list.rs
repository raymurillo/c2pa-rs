use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, ListState},
    Frame,
};

use crate::app::{App, LoadState, Pane};

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let focused = app.focused_pane == Pane::FileList;
    let border_style = if focused {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };

    let items: Vec<ListItem> = app
        .sources
        .iter()
        .map(|(sid, src)| {
            let icon = match app.loaded.get(sid) {
                Some(LoadState::Loading) => "[~]",
                Some(LoadState::Loaded(_)) => "[✓]",
                None => "[ ]",
            };
            let suffix = if src.is_remote() { " (remote)" } else { "" };
            let label = format!("{} {}{}", icon, src.label(), suffix);
            let style = if Some(*sid) == app.selected_left {
                Style::default().add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            ListItem::new(label).style(style)
        })
        .collect();

    // Translate the selected SourceId back to a list position for the
    // ListState cursor.  None when `sources` is empty or the selection is
    // stale — ratatui handles `None` as "no selection".
    let mut list_state = ListState::default();
    list_state.select(app.selected_left.and_then(|id| app.index_of(id)));

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title("Files")
                .border_style(border_style),
        )
        .highlight_style(Style::default().bg(Color::DarkGray));

    frame.render_stateful_widget(list, area, &mut list_state);
}
