use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::app::{App, AppState};
use crate::ui::layout::centered_popup;

/// Render the fuzzy search overlay bar.
pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let popup_area = centered_popup(area, 70, 50);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(1)])
        .split(popup_area);

    let query = if let AppState::Searching { query } = &app.state {
        query.as_str()
    } else {
        ""
    };

    frame.render_widget(
        Paragraph::new(format!("> {query}")).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Search")
                .border_style(Style::default().fg(Color::Yellow)),
        ),
        chunks[0],
    );

    // Re-use the flat node list already held by the matcher — avoids a second
    // flatten() call per frame (detail::draw already pays that cost via
    // apply_filter_and_search when searching is active).
    let items_cache = app.matcher.items();

    let items: Vec<ListItem> = app
        .search_results
        .iter()
        .enumerate()
        .filter_map(|(i, result)| {
            let node = items_cache.get(result.node_index)?;
            let base_style = if i == app.search_cursor {
                Style::default()
                    .bg(Color::DarkGray)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let line = build_highlighted_line(&node.display, &result.highlight_ranges, base_style);
            Some(ListItem::new(line))
        })
        .collect();

    // Use the actual rendered count so the title stays consistent when a
    // stale index causes some results to be silently skipped.
    let visible = items.len();

    let mut list_state = ListState::default();
    list_state.select(Some(app.search_cursor));

    frame.render_stateful_widget(
        List::new(items).block(
            Block::default()
                .borders(Borders::ALL)
                .title(format!("{visible} matches")),
        ),
        chunks[1],
        &mut list_state,
    );
}

/// Build a ratatui Line with match ranges highlighted in yellow bold.
fn build_highlighted_line(
    display: &str,
    ranges: &[std::ops::Range<usize>],
    base_style: Style,
) -> Line<'static> {
    if ranges.is_empty() {
        return Line::from(Span::styled(display.to_string(), base_style));
    }

    let highlight_style = base_style.fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let mut spans = Vec::new();
    let mut last = 0usize;

    for range in ranges {
        if last < range.start {
            spans.push(Span::styled(
                display[last..range.start].to_string(),
                base_style,
            ));
        }
        if range.start < range.end && range.end <= display.len() {
            spans.push(Span::styled(
                display[range.start..range.end].to_string(),
                highlight_style,
            ));
        }
        last = range.end;
    }
    if last < display.len() {
        spans.push(Span::styled(display[last..].to_string(), base_style));
    }

    Line::from(spans)
}
