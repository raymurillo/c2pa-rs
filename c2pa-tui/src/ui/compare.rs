use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Row, Table};
use ratatui::Frame;

use crate::app::{App, LoadState};
use crate::compare::diff::{diff, FieldDiff};

/// Render the side-by-side manifest comparison pane.
///
/// Shows a two-column table with field paths and per-manifest values.
/// Colour-codes rows: yellow = changed, red = only left, green = only right.
/// Equal rows are shown only when `app.show_all_diffs` is true.
pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let theme = &app.config.theme;

    let (left_idx, right_idx) = match app.compare_selection {
        Some(r) => (app.selected_left, r),
        None => {
            draw_placeholder(frame, area, "Select two files with 'c' to compare");
            return;
        }
    };

    let left_nodes = match app.loaded.get(&left_idx) {
        Some(LoadState::Loaded(n)) => n.clone(),
        _ => {
            draw_placeholder(
                frame,
                area,
                "Left manifest not loaded — press Enter to load",
            );
            return;
        }
    };

    let right_nodes = match app.loaded.get(&right_idx) {
        Some(LoadState::Loaded(n)) => n.clone(),
        _ => {
            draw_placeholder(
                frame,
                area,
                "Right manifest not loaded — press Enter to load",
            );
            return;
        }
    };

    let left_label = app
        .sources
        .get(left_idx)
        .map(|s| s.label().to_owned())
        .unwrap_or_else(|| format!("source {left_idx}"));

    let right_label = app
        .sources
        .get(right_idx)
        .map(|s| s.label().to_owned())
        .unwrap_or_else(|| format!("source {right_idx}"));

    let manifest_diff = diff(&left_label, &left_nodes, &right_label, &right_nodes);
    let show_all = app.show_all_diffs;

    // Split header from table body vertically.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    // Column header line showing both labels.
    let header_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(30),
            Constraint::Percentage(35),
            Constraint::Percentage(35),
        ])
        .split(chunks[0]);

    frame.render_widget(ratatui::widgets::Paragraph::new("Field"), header_chunks[0]);
    frame.render_widget(
        ratatui::widgets::Paragraph::new(Line::from(vec![Span::styled(
            truncate(&left_label, header_chunks[1].width as usize),
            Style::default().add_modifier(Modifier::BOLD),
        )])),
        header_chunks[1],
    );
    frame.render_widget(
        ratatui::widgets::Paragraph::new(Line::from(vec![Span::styled(
            truncate(&right_label, header_chunks[2].width as usize),
            Style::default().add_modifier(Modifier::BOLD),
        )])),
        header_chunks[2],
    );

    let diff_count = manifest_diff.diff_count();
    let title = if diff_count == 0 {
        "Compare — identical".to_owned()
    } else if show_all {
        format!("Compare — {diff_count} differences (showing all)  [a] hide equal")
    } else {
        format!("Compare — {diff_count} differences  [a] show equal")
    };

    let rows: Vec<Row> = manifest_diff
        .fields
        .iter()
        .filter(|f| show_all || !matches!(f, FieldDiff::Equal { .. }))
        .map(|f| match f {
            FieldDiff::Equal { path, value } => Row::new(vec![
                Cell::from(path.as_str()),
                Cell::from(value.as_str()),
                Cell::from(value.as_str()),
            ])
            .style(Style::default().add_modifier(Modifier::DIM)),
            FieldDiff::Changed { path, left, right } => Row::new(vec![
                Cell::from(path.as_str()),
                Cell::from(left.as_str()),
                Cell::from(right.as_str()),
            ])
            .style(theme.diff_changed()),
            FieldDiff::OnlyLeft { path, value } => Row::new(vec![
                Cell::from(path.as_str()),
                Cell::from(value.as_str()),
                Cell::from(""),
            ])
            .style(theme.diff_only_left()),
            FieldDiff::OnlyRight { path, value } => Row::new(vec![
                Cell::from(path.as_str()),
                Cell::from(""),
                Cell::from(value.as_str()),
            ])
            .style(theme.diff_only_right()),
        })
        .collect();

    let empty_msg = if show_all {
        "No fields"
    } else {
        "No differences found  [a] show all"
    };

    if rows.is_empty() {
        draw_placeholder(frame, chunks[1], empty_msg);
    } else {
        let table = Table::new(
            rows,
            [
                Constraint::Percentage(30),
                Constraint::Percentage(35),
                Constraint::Percentage(35),
            ],
        )
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(Color::Cyan)),
        )
        .column_spacing(1)
        .row_highlight_style(Style::default().add_modifier(Modifier::BOLD));

        frame.render_widget(table, chunks[1]);
    }
}

fn draw_placeholder(frame: &mut Frame, area: Rect, msg: &str) {
    frame.render_widget(
        ratatui::widgets::Paragraph::new(msg).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Compare")
                .border_style(Style::default().fg(Color::Cyan)),
        ),
        area,
    );
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_owned()
    } else {
        let mut t: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}
