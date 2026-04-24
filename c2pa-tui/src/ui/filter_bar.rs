use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use crate::app::{App, AppState, LoadState};
use crate::manifest::filter::FieldFilter;
use crate::ui::layout::centered_popup;

/// Render the field filter bar overlay.
pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let popup_area = centered_popup(area, 60, 40);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(popup_area);

    let query = if let AppState::Filtering { query } = &app.state {
        query.as_str()
    } else {
        ""
    };

    frame.render_widget(
        Paragraph::new(format!("> {query}")).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Filter (glob, e.g. assertions.*)")
                .border_style(Style::default().fg(Color::Cyan)),
        ),
        chunks[0],
    );

    // Parse the query once; reuse the result for both the label and the preview
    // list to avoid compiling glob patterns twice per frame.
    let filter_result = FieldFilter::from_query(query);

    let preview_label = match &filter_result {
        Ok(_) => "Preview (Enter to apply, Esc to cancel):".to_string(),
        Err(e) => format!("Invalid pattern: {e}"),
    };
    frame.render_widget(Paragraph::new(preview_label), chunks[1]);

    let preview_items: Vec<ListItem> = match filter_result {
        Ok(filter) => {
            if let Some(LoadState::Loaded(nodes)) = app.loaded.get(&app.selected_left) {
                // apply_ref borrows nodes rather than cloning the whole tree up
                // front — only surviving nodes are allocated.
                filter
                    .apply_ref(nodes)
                    .iter()
                    .map(|n| ListItem::new(format!("  {} ({})", n.key, n.children.len())))
                    .collect()
            } else {
                vec![ListItem::new("  (no manifest loaded)")]
            }
        }
        Err(_) => vec![],
    };

    frame.render_widget(
        List::new(preview_items).block(Block::default().borders(Borders::TOP)),
        chunks[2],
    );
}
