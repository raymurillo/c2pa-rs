use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::config::Config;
use crate::error::Result;
use crate::manifest::filter::FieldFilter;
use crate::manifest::loader::ManifestSource;
use crate::manifest::tree::DisplayNode;
use crate::remote::client::RemoteClient;
use crate::search::matcher::Matcher;

/// High-level state machine for the TUI.
#[derive(Debug, Clone, PartialEq)]
pub enum AppState {
    /// Normal file-list / detail browsing.
    Browse,
    /// Fuzzy search overlay is active.
    Searching { query: String },
    /// Field filter bar is active.
    Filtering { query: String },
    /// Side-by-side manifest comparison view is active.
    Comparing,
    /// A recoverable error is displayed in the status bar.
    Error { message: String },
}

/// Top-level application context passed to every draw call and event handler.
pub struct App {
    /// Sources are stored as `Arc` so they can be cloned into background tokio
    /// tasks without requiring `ManifestSource: Clone`.
    pub sources: Vec<Arc<dyn ManifestSource>>,
    /// Parsed node trees keyed by source index.
    pub loaded: HashMap<usize, Vec<DisplayNode>>,
    /// Indices of sources currently being loaded in background tasks.
    pub loading_indices: HashSet<usize>,
    /// Index of the source currently shown in the detail pane.
    pub selected_left: usize,
    /// Index of the right-side source for comparison, if any.
    pub compare_selection: Option<usize>,
    /// Active field filter.
    pub filter: FieldFilter,
    /// Fuzzy matcher over flattened nodes.
    pub matcher: Matcher,
    /// Current UI state.
    pub state: AppState,
    /// Runtime configuration.
    pub config: Config,
    /// Shared HTTP client.
    pub client: RemoteClient,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_state_equality() {
        assert_eq!(AppState::Browse, AppState::Browse);
        assert_eq!(AppState::Comparing, AppState::Comparing);
        assert_ne!(AppState::Browse, AppState::Comparing);
    }

    #[test]
    fn app_state_searching_equality() {
        let a = AppState::Searching {
            query: "foo".into(),
        };
        let b = AppState::Searching {
            query: "foo".into(),
        };
        let c = AppState::Searching {
            query: "bar".into(),
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn app_state_error_equality() {
        let a = AppState::Error {
            message: "oops".into(),
        };
        let b = AppState::Error {
            message: "oops".into(),
        };
        assert_eq!(a, b);
    }
}

impl App {
    /// Construct a new `App` from the given config.
    ///
    /// Stub: not yet implemented. Implemented in spec-05.
    pub fn new(config: Config) -> Result<Self> {
        let _ = config;
        todo!("spec-05: implement App::new")
    }

    /// Enter the TUI event loop and block until the user quits.
    ///
    /// Stub: not yet implemented. Implemented in spec-05.
    pub async fn run(self) -> Result<()> {
        todo!("spec-05: implement App::run (TUI event loop)")
    }

    /// Register a new manifest source.
    pub fn add_source(&mut self, source: Arc<dyn ManifestSource>) {
        self.sources.push(source);
    }
}
