use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::Paragraph,
    Frame,
};

use crate::app::{App, AppState};

pub fn draw(frame: &mut Frame, area: Rect, app: &mut App) {
    let hints = match &app.state {
        AppState::Browse => {
            if app.loading_count > 0 {
                "Loading…  ↑/↓:nav  q:quit"
            } else {
                "↑/↓:nav  Enter:load  Tab:focus  /:search  f:filter  c:compare  r:reload  ?:help  q:quit"
            }
        }
        AppState::Searching { .. } => "Type to search  Esc:cancel",
        AppState::Filtering { .. } => {
            "Type glob filter (e.g. assertions.*)  Enter:apply  Esc:cancel"
        }
        AppState::Comparing => "Comparing  Esc:exit compare",
        AppState::Error { .. } => "Error — press any key to dismiss",
    };

    frame.render_widget(
        Paragraph::new(hints).style(Style::default().fg(Color::DarkGray)),
        area,
    );
}
