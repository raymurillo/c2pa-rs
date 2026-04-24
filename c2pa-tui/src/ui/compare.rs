use std::borrow::Cow;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Row, Table};
use ratatui::Frame;

use crate::app::{App, LoadState};
use crate::compare::diff::{diff, FieldDiff};

/// Render the side-by-side manifest comparison pane.
///
/// The diff result is **cached** in `App::compare_diff_cache` and recomputed
/// only when the cache is cold (first render, after a reload of either source,
/// or after the comparison pair changes).  Subsequent frames at ~60 fps read
/// the cached result without any heap allocation.
///
/// Colour-codes rows: yellow = changed, red = only left, green = only right.
/// Equal rows are shown only when `app.show_all_diffs` is true.
pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let (left_id, right_id) = match (app.selected_left, app.compare_selection) {
        (Some(l), Some(r)) => (l, r),
        _ => {
            draw_placeholder(frame, area, "Select two files with 'c' to compare");
            return;
        }
    };

    // Labels are needed for both the column headers and (when cold) diff computation.
    let left_label = app
        .source_by_id(left_id)
        .map(|s| s.label().to_owned())
        .unwrap_or_else(|| left_id.to_string());
    let right_label = app
        .source_by_id(right_id)
        .map(|s| s.label().to_owned())
        .unwrap_or_else(|| right_id.to_string());

    // Populate the cache if it is cold — clones happen at most once per
    // comparison pair or per reload, never on every frame.
    if app.compare_diff_cache.is_none() {
        let left_nodes = match app.loaded.get(&left_id) {
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
        let right_nodes = match app.loaded.get(&right_id) {
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
        app.compare_diff_cache = Some(diff(&left_label, &left_nodes, &right_label, &right_nodes));
    }

    // SAFETY: populated just above.
    let manifest_diff = app.compare_diff_cache.as_ref().unwrap();
    let show_all = app.show_all_diffs;
    let theme = &app.config.theme;

    // Split a 1-line header strip from the scrollable table body.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    // Column header line showing both source labels.
    let header_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(30),
            Constraint::Percentage(35),
            Constraint::Percentage(35),
        ])
        .split(chunks[0]);

    let bold = Style::default().add_modifier(Modifier::BOLD);
    frame.render_widget(ratatui::widgets::Paragraph::new("Field"), header_chunks[0]);
    frame.render_widget(
        ratatui::widgets::Paragraph::new(Line::from(vec![Span::styled(
            truncate(&left_label, header_chunks[1].width as usize),
            bold,
        )])),
        header_chunks[1],
    );
    frame.render_widget(
        ratatui::widgets::Paragraph::new(Line::from(vec![Span::styled(
            truncate(&right_label, header_chunks[2].width as usize),
            bold,
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

/// Truncate `s` to at most `max_chars` Unicode scalar values, appending `…`
/// if truncation occurs.
///
/// Returns a borrowed slice when no truncation is needed — zero allocation on
/// the common path (short labels that fit within the column width).
fn truncate(s: &str, max_chars: usize) -> Cow<'_, str> {
    // Walk char boundaries; stop as soon as we've seen `max_chars` chars.
    for (char_count, (byte_idx, _)) in s.char_indices().enumerate() {
        if char_count == max_chars {
            // We hit the limit at `byte_idx` — everything from here gets cut.
            let mut out = s[..byte_idx].to_owned();
            out.push('…');
            return Cow::Owned(out);
        }
    }
    // Entire string fits — borrow without allocating.
    Cow::Borrowed(s)
}

#[cfg(test)]
mod tests {
    use super::truncate;

    #[test]
    fn truncate_short_string_borrows() {
        let s = "hello";
        let result = truncate(s, 10);
        assert!(matches!(result, std::borrow::Cow::Borrowed(_)));
        assert_eq!(result, "hello");
    }

    #[test]
    fn truncate_exact_length_borrows() {
        let s = "hello";
        let result = truncate(s, 5);
        assert!(matches!(result, std::borrow::Cow::Borrowed(_)));
        assert_eq!(result, "hello");
    }

    #[test]
    fn truncate_over_limit_appends_ellipsis() {
        let result = truncate("hello world", 5);
        assert_eq!(result, "hello…");
    }

    #[test]
    fn truncate_multibyte_chars_counted_correctly() {
        // "café" is 4 chars but 5 bytes (é = 2 bytes)
        let result = truncate("café extra", 4);
        assert_eq!(result, "café…");
    }

    #[test]
    fn truncate_zero_limit_returns_ellipsis() {
        let result = truncate("hello", 0);
        assert_eq!(result, "…");
    }

    #[test]
    fn truncate_empty_string_borrows() {
        let result = truncate("", 5);
        assert!(matches!(result, std::borrow::Cow::Borrowed(_)));
        assert_eq!(result, "");
    }
}
