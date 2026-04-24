/// Runtime configuration for c2pa-tui.
#[derive(Debug, Clone)]
pub struct Config {
    /// Color theme.
    pub theme: Theme,
    /// Whether to enable mouse events.
    pub mouse_enabled: bool,
    /// Width percentage for the left file-list pane (1–99, default 25).
    pub left_pane_pct: u16,
    /// HTTP authentication credentials for remote manifest requests.
    pub auth: crate::remote::Auth,
    /// Optional initial field filter applied on startup.
    pub initial_filter: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            mouse_enabled: true,
            left_pane_pct: 25,
            auth: crate::remote::Auth::None,
            initial_filter: None,
        }
    }
}

/// Terminal color theme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Theme {
    /// Dark background palette.
    Dark,
    /// Light background palette.
    Light,
    /// Monochrome (no color).
    Mono,
}

impl Theme {
    /// Style for a focused pane border.
    pub fn border_focused(&self) -> ratatui::style::Style {
        use ratatui::style::{Color, Modifier, Style};
        match self {
            Theme::Dark => Style::default().fg(Color::Yellow),
            Theme::Light => Style::default().fg(Color::Blue),
            Theme::Mono => Style::default().add_modifier(Modifier::BOLD),
        }
    }

    /// Style for an unfocused pane border.
    pub fn border_normal(&self) -> ratatui::style::Style {
        ratatui::style::Style::default()
    }

    /// Style for the selected/highlighted list row.
    pub fn highlight(&self) -> ratatui::style::Style {
        use ratatui::style::{Color, Modifier, Style};
        match self {
            Theme::Dark => Style::default().bg(Color::DarkGray),
            Theme::Light => Style::default().bg(Color::Gray),
            Theme::Mono => Style::default().add_modifier(Modifier::REVERSED),
        }
    }

    /// Style for search match highlights.
    pub fn match_highlight(&self) -> ratatui::style::Style {
        use ratatui::style::{Color, Modifier, Style};
        match self {
            Theme::Dark => Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            Theme::Light => Style::default()
                .fg(Color::Blue)
                .add_modifier(Modifier::BOLD),
            Theme::Mono => Style::default().add_modifier(Modifier::UNDERLINED),
        }
    }

    /// Style for a field that changed between left and right manifests.
    pub fn diff_changed(&self) -> ratatui::style::Style {
        use ratatui::style::{Color, Modifier, Style};
        match self {
            Theme::Mono => Style::default().add_modifier(Modifier::BOLD),
            _ => Style::default().fg(Color::Yellow),
        }
    }

    /// Style for a field present only in the left manifest.
    pub fn diff_only_left(&self) -> ratatui::style::Style {
        use ratatui::style::{Color, Modifier, Style};
        match self {
            Theme::Mono => Style::default().add_modifier(Modifier::DIM),
            _ => Style::default().fg(Color::Red),
        }
    }

    /// Style for a field present only in the right manifest.
    pub fn diff_only_right(&self) -> ratatui::style::Style {
        use ratatui::style::{Color, Modifier, Style};
        match self {
            Theme::Mono => Style::default().add_modifier(Modifier::ITALIC),
            _ => Style::default().fg(Color::Green),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_values() {
        let cfg = Config::default();
        // Dark is the most common terminal background — default theme
        assert_eq!(cfg.theme, Theme::Dark);
        // Mouse enabled by default for richer UX; users may opt out via config
        assert!(cfg.mouse_enabled);
        // Left pane must occupy a valid percentage of the terminal width (1–99)
        assert!(
            cfg.left_pane_pct >= 1 && cfg.left_pane_pct <= 99,
            "left_pane_pct={} is outside the valid 1–99 range",
            cfg.left_pane_pct
        );
        // No credentials by default — user must explicitly configure auth
        assert!(matches!(cfg.auth, crate::remote::Auth::None));
        // No filter pre-applied on startup
        assert!(cfg.initial_filter.is_none());
    }

    #[test]
    fn theme_equality() {
        assert_eq!(Theme::Dark, Theme::Dark);
        assert_eq!(Theme::Light, Theme::Light);
        assert_eq!(Theme::Mono, Theme::Mono);
        assert_ne!(Theme::Dark, Theme::Light);
        assert_ne!(Theme::Light, Theme::Mono);
    }

    #[test]
    fn theme_clone() {
        let t = Theme::Mono;
        assert_eq!(t.clone(), Theme::Mono);
    }
}
