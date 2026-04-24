use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use tui_tree_widget::TreeState;

use crate::config::Config;
use crate::error::{AppError, Result};
use crate::manifest::filter::FieldFilter;
use crate::manifest::loader::ManifestSource;
use crate::manifest::tree::DisplayNode;
use crate::remote::client::RemoteClient;
use crate::search::matcher::{MatchResult, Matcher};
use crate::ui::layout::CachedLayout;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extension trait that converts any `Display` error into `AppError::Terminal`.
trait IntoTerminalErr<T> {
    fn terminal(self) -> Result<T>;
}

impl<T, E: std::fmt::Display> IntoTerminalErr<T> for std::result::Result<T, E> {
    fn terminal(self) -> Result<T> {
        self.map_err(|e| AppError::Terminal(e.to_string()))
    }
}

// ---------------------------------------------------------------------------
// State types
// ---------------------------------------------------------------------------

/// Which pane currently has keyboard focus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pane {
    FileList,
    Detail,
}

/// Cheap discriminant used to dispatch key events without cloning `AppState`.
#[derive(Debug, PartialEq, Eq)]
enum StateKind {
    Browse,
    Searching,
    Filtering,
    Comparing,
    Error,
}

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
    /// A recoverable error overlay is displayed.
    /// `message` is already formatted for display (includes prompt to dismiss).
    Error { message: String },
}

impl AppState {
    fn kind(&self) -> StateKind {
        match self {
            AppState::Browse => StateKind::Browse,
            AppState::Searching { .. } => StateKind::Searching,
            AppState::Filtering { .. } => StateKind::Filtering,
            AppState::Comparing => StateKind::Comparing,
            AppState::Error { .. } => StateKind::Error,
        }
    }
}

/// Per-source loading state stored in `App::loaded`.
#[derive(Debug)]
pub enum LoadState {
    /// A background task is in flight for this index.
    Loading,
    /// Load completed successfully.
    Loaded(Vec<DisplayNode>),
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

/// Top-level application context passed to every draw call and event handler.
pub struct App {
    /// Sources are stored as `Arc` so they can be cloned into background tokio
    /// tasks without requiring `ManifestSource: Clone`.
    pub sources: Vec<Arc<dyn ManifestSource>>,
    /// Per-source loading state.  A missing entry means "not yet requested".
    pub loaded: HashMap<usize, LoadState>,
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
    /// Which pane has keyboard focus.
    pub focused_pane: Pane,
    /// Whether the help overlay is visible.
    pub show_help: bool,
    /// Cached layout rects, invalidated on terminal resize.
    pub layout_cache: Option<(ratatui::layout::Rect, CachedLayout)>,
    /// Expand/collapse and scroll state for the detail tree.
    pub detail_tree_state: TreeState<String>,
    /// Number of sources currently being loaded in background tasks.
    pub loading_count: usize,
    /// Current search matches for the active query — updated as the user types.
    pub search_results: Vec<MatchResult>,
    /// Cursor within search_results (for navigating between matches).
    pub search_cursor: usize,
    /// `node_index` values from `search_results`, kept in sync to avoid a
    /// per-frame `HashSet` allocation in the detail-pane draw path.
    pub search_result_indices: HashSet<usize>,
}

impl App {
    /// Construct a new `App` from the given config.
    pub fn new(config: Config) -> Result<Self> {
        let client = RemoteClient::new()?;
        Ok(Self {
            sources: Vec::new(),
            loaded: HashMap::new(),
            selected_left: 0,
            compare_selection: None,
            filter: FieldFilter::default(),
            matcher: Matcher::new(),
            state: AppState::Browse,
            config,
            client,
            focused_pane: Pane::FileList,
            show_help: false,
            layout_cache: None,
            detail_tree_state: TreeState::default(),
            loading_count: 0,
            search_results: Vec::new(),
            search_cursor: 0,
            search_result_indices: HashSet::new(),
        })
    }

    /// Enter the TUI event loop and block until the user quits.
    pub async fn run(mut self) -> Result<()> {
        // Initialize tracing to stderr before we take over stdout with the TUI.
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_writer(std::io::stderr)
            .try_init();

        // Install panic hook exactly once across the process lifetime so that
        // repeated calls (e.g. in tests) don't stack-wrap the hook.
        static PANIC_HOOK: std::sync::OnceLock<()> = std::sync::OnceLock::new();
        PANIC_HOOK.get_or_init(|| {
            let default_panic = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                let _ = crossterm::terminal::disable_raw_mode();
                let _ = crossterm::execute!(
                    std::io::stderr(),
                    crossterm::terminal::LeaveAlternateScreen,
                    crossterm::event::DisableMouseCapture,
                );
                default_panic(info);
            }));
        });

        crossterm::terminal::enable_raw_mode().terminal()?;
        let mut stdout = std::io::stdout();
        crossterm::execute!(
            stdout,
            crossterm::terminal::EnterAlternateScreen,
            crossterm::event::EnableMouseCapture,
        )
        .terminal()?;

        let backend = ratatui::backend::CrosstermBackend::new(stdout);
        let mut terminal = ratatui::Terminal::new(backend).terminal()?;

        let result = self.event_loop(&mut terminal).await;

        // Always restore terminal, even if event_loop returned an error.
        crossterm::terminal::disable_raw_mode().terminal()?;
        crossterm::execute!(
            terminal.backend_mut(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture,
        )
        .terminal()?;
        terminal.show_cursor().terminal()?;

        result
    }

    async fn event_loop(
        &mut self,
        terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
    ) -> Result<()> {
        use crossterm::event::EventStream;
        use futures::StreamExt;
        use tokio::sync::mpsc;

        let (load_tx, mut load_rx) = mpsc::unbounded_channel::<(usize, Result<Vec<DisplayNode>>)>();

        let mut event_stream = EventStream::new();

        loop {
            // Draw once per event/load-result. tokio::select! blocks until
            // one arm fires, so this never spins: each iteration is driven by
            // real work.
            terminal.draw(|f| crate::ui::draw(f, self)).terminal()?;

            tokio::select! {
                Some(Ok(event)) = event_stream.next() => {
                    if self.handle_event(event, &load_tx).await? {
                        break;
                    }
                }
                Some((idx, result)) = load_rx.recv() => {
                    self.handle_load_result(idx, result);
                }
            }
        }

        Ok(())
    }

    async fn handle_event(
        &mut self,
        event: crossterm::event::Event,
        load_tx: &tokio::sync::mpsc::UnboundedSender<(usize, Result<Vec<DisplayNode>>)>,
    ) -> Result<bool> {
        use crossterm::event::{Event, KeyCode, KeyModifiers};

        match event {
            Event::Key(key) => {
                if matches!(key.code, KeyCode::Char('q'))
                    || (key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL))
                {
                    return Ok(true);
                }

                // Dispatch via a cheap copy of the state discriminant so we
                // avoid cloning the heap-allocated String fields inside AppState.
                match self.state.kind() {
                    StateKind::Browse => self.handle_browse_key(key, load_tx)?,
                    StateKind::Searching => self.handle_search_key(key),
                    StateKind::Filtering => self.handle_filter_key(key),
                    StateKind::Error => {
                        self.state = AppState::Browse;
                    }
                    StateKind::Comparing => self.handle_compare_key(key),
                }
            }
            Event::Mouse(mouse) if self.config.mouse_enabled => {
                self.handle_mouse(mouse);
            }
            Event::Resize(_, _) => {
                self.layout_cache = None;
            }
            _ => {}
        }

        Ok(false)
    }

    fn handle_browse_key(
        &mut self,
        key: crossterm::event::KeyEvent,
        load_tx: &tokio::sync::mpsc::UnboundedSender<(usize, Result<Vec<DisplayNode>>)>,
    ) -> Result<()> {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected_left = self.selected_left.saturating_sub(1);
                self.reindex_for_selected();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let max = self.sources.len().saturating_sub(1);
                self.selected_left = (self.selected_left + 1).min(max);
                self.reindex_for_selected();
            }
            KeyCode::Enter => {
                let idx = self.selected_left;
                self.trigger_load(idx, false, load_tx);
            }
            KeyCode::Char('r') => {
                let idx = self.selected_left;
                self.trigger_load(idx, true, load_tx);
            }
            KeyCode::Tab => {
                self.focused_pane = match self.focused_pane {
                    Pane::FileList => Pane::Detail,
                    Pane::Detail => Pane::FileList,
                };
            }
            KeyCode::Char('/') => {
                // Ensure the index reflects the current selection before the
                // overlay opens (catches mouse-driven selection changes that
                // bypass the Up/Down handlers above).
                self.reindex_for_selected();
                self.state = AppState::Searching {
                    query: String::new(),
                };
            }
            KeyCode::Char('f') => {
                self.state = AppState::Filtering {
                    query: String::new(),
                };
            }
            KeyCode::Char('c') => match self.compare_selection {
                None => self.compare_selection = Some(self.selected_left),
                Some(_) => self.state = AppState::Comparing,
            },
            KeyCode::Esc => {
                self.compare_selection = None;
            }
            KeyCode::Char('?') => {
                self.show_help = !self.show_help;
            }
            KeyCode::Char(' ') if self.focused_pane == Pane::Detail => {
                self.detail_tree_state.toggle_selected();
            }
            _ => {}
        }
        Ok(())
    }

    fn trigger_load(
        &mut self,
        idx: usize,
        force: bool,
        load_tx: &tokio::sync::mpsc::UnboundedSender<(usize, Result<Vec<DisplayNode>>)>,
    ) {
        match self.loaded.get(&idx) {
            Some(LoadState::Loaded(_)) if !force => return,
            Some(LoadState::Loading) => return,
            _ => {}
        }

        // Resolve source before mutating state so we never enter Loading
        // without a corresponding background task.
        let Some(src) = self.sources.get(idx).cloned() else {
            return;
        };
        self.loaded.insert(idx, LoadState::Loading);
        self.loading_count += 1;

        let client = self.client.clone();
        let tx = load_tx.clone();
        tokio::spawn(async move {
            let result = src.load(&client).await;
            let _ = tx.send((idx, result));
        });
    }

    fn handle_load_result(&mut self, idx: usize, result: Result<Vec<DisplayNode>>) {
        self.loading_count = self.loading_count.saturating_sub(1);
        match result {
            Ok(nodes) => {
                self.loaded.insert(idx, LoadState::Loaded(nodes));
                // Only reset expand/collapse state when the loaded source is
                // the one currently on display; background loads for other
                // sources should not collapse the tree the user is browsing.
                if idx == self.selected_left {
                    self.detail_tree_state = TreeState::default();
                    // Re-index so the matcher is ready the moment the user
                    // presses '/' — avoids the index cost on the first keystroke.
                    self.reindex_for_selected();
                }
            }
            Err(e) => {
                self.loaded.remove(&idx);
                // Pre-format the display text once rather than on every frame.
                self.state = AppState::Error {
                    message: format!("Error: {e}\n\nPress any key to dismiss."),
                };
            }
        }
    }

    fn handle_search_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Esc => {
                self.state = AppState::Browse;
                self.search_results.clear();
                self.search_result_indices.clear();
                self.search_cursor = 0;
            }
            KeyCode::Char(c) => {
                if let AppState::Searching { query } = &mut self.state {
                    query.push(c);
                }
                self.reindex_and_search();
            }
            KeyCode::Backspace => {
                if let AppState::Searching { query } = &mut self.state {
                    query.pop();
                }
                self.reindex_and_search();
            }
            KeyCode::Down | KeyCode::Tab if !self.search_results.is_empty() => {
                self.search_cursor = (self.search_cursor + 1) % self.search_results.len();
            }
            KeyCode::Up if !self.search_results.is_empty() => {
                self.search_cursor = self
                    .search_cursor
                    .checked_sub(1)
                    .unwrap_or(self.search_results.len() - 1);
            }
            _ => {}
        }
    }

    /// (Re-)index the flat nodes for the currently selected manifest.
    ///
    /// Call this whenever `selected_left` changes or a manifest finishes
    /// loading.  Does **not** re-run the query; call [`Self::reindex_and_search`]
    /// for that.  If no manifest is loaded for the selected source the matcher
    /// is cleared so stale results from a previous selection are not shown.
    pub fn reindex_for_selected(&mut self) {
        match self.loaded.get(&self.selected_left) {
            Some(LoadState::Loaded(nodes)) => {
                let flat = crate::manifest::tree::flatten(nodes);
                self.matcher.index(&flat);
            }
            _ => {
                // No manifest loaded; clear stale index from a previous selection.
                self.matcher.index(&[]);
            }
        }
    }

    /// Run the active search query against the already-indexed nodes.
    ///
    /// Updates `search_results`, `search_result_indices`, and `search_cursor`.
    /// Preserves the cursor on the same node when possible so that refining
    /// a query does not jump the selection back to the top.
    ///
    /// Does **not** re-index; call [`Self::reindex_for_selected`] first if
    /// the node set has changed.
    pub fn reindex_and_search(&mut self) {
        let query = match &self.state {
            AppState::Searching { query } => query.clone(),
            _ => String::new(),
        };

        // Remember which node was selected so we can try to keep it visible.
        let prev_node_index = self
            .search_results
            .get(self.search_cursor)
            .map(|r| r.node_index);

        self.search_results = self.matcher.query(&query);

        // Preserve cursor on the same node; fall back to the top.
        self.search_cursor = prev_node_index
            .and_then(|idx| self.search_results.iter().position(|r| r.node_index == idx))
            .unwrap_or(0);

        // Keep the index set in sync so detail::draw avoids a per-frame alloc.
        self.search_result_indices = self.search_results.iter().map(|r| r.node_index).collect();
    }

    fn handle_filter_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Esc => self.state = AppState::Browse,
            KeyCode::Enter => {
                // Extract and validate the query first to release the borrow
                // on self.state before mutating self.filter / self.state.
                let filter_result = if let AppState::Filtering { query } = &self.state {
                    Some(FieldFilter::from_query(query))
                } else {
                    None
                };
                if let Some(result) = filter_result {
                    match result {
                        Ok(f) => {
                            self.filter = f;
                            self.state = AppState::Browse;
                        }
                        Err(e) => {
                            self.state = AppState::Error {
                                message: format!("Error: {e}\n\nPress any key to dismiss."),
                            };
                        }
                    }
                }
            }
            KeyCode::Char(c) => {
                if let AppState::Filtering { query } = &mut self.state {
                    query.push(c);
                }
            }
            KeyCode::Backspace => {
                if let AppState::Filtering { query } = &mut self.state {
                    query.pop();
                }
            }
            _ => {}
        }
    }

    fn handle_compare_key(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::KeyCode;
        if key.code == KeyCode::Esc {
            self.compare_selection = None;
            self.state = AppState::Browse;
        }
    }

    fn handle_mouse(&mut self, event: crossterm::event::MouseEvent) {
        use crossterm::event::{MouseButton, MouseEventKind};

        match event.kind {
            MouseEventKind::ScrollDown => match self.focused_pane {
                Pane::FileList => {
                    self.selected_left =
                        (self.selected_left + 1).min(self.sources.len().saturating_sub(1));
                }
                Pane::Detail => {
                    self.detail_tree_state.scroll_down(1);
                }
            },
            MouseEventKind::ScrollUp => match self.focused_pane {
                Pane::FileList => {
                    self.selected_left = self.selected_left.saturating_sub(1);
                }
                Pane::Detail => {
                    self.detail_tree_state.scroll_up(1);
                }
            },
            MouseEventKind::Down(MouseButton::Left) => {
                // Use the cached layout rects for hit-testing: always consistent
                // with what was last rendered, and avoids a separate ioctl/syscall
                // to query the terminal size.
                if let Some((_, ref layout)) = self.layout_cache {
                    if event.column < layout.list_area.right() {
                        self.focused_pane = Pane::FileList;
                        // Subtract the top border row to get the item index.
                        let row = event.row.saturating_sub(layout.list_area.top() + 1) as usize;
                        if row < self.sources.len() {
                            self.selected_left = row;
                        }
                    } else {
                        self.focused_pane = Pane::Detail;
                    }
                }
            }
            _ => {}
        }
    }

    /// Register a new manifest source.
    pub fn add_source(&mut self, source: Arc<dyn ManifestSource>) {
        self.sources.push(source);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_state_variants_are_mutually_distinct() {
        let states = [
            AppState::Browse,
            AppState::Searching { query: "q".into() },
            AppState::Filtering { query: "q".into() },
            AppState::Comparing,
            AppState::Error {
                message: "e".into(),
            },
        ];
        for (i, a) in states.iter().enumerate() {
            for (j, b) in states.iter().enumerate() {
                if i == j {
                    assert_eq!(a, b, "state[{i}] should equal itself");
                } else {
                    assert_ne!(a, b, "state[{i}] should differ from state[{j}]");
                }
            }
        }
    }

    #[test]
    fn data_variant_equality_is_field_sensitive() {
        assert_ne!(
            AppState::Searching {
                query: "foo".into()
            },
            AppState::Searching {
                query: "bar".into()
            },
        );
        assert_ne!(
            AppState::Filtering {
                query: "foo".into()
            },
            AppState::Filtering {
                query: "bar".into()
            },
        );
        assert_ne!(
            AppState::Error {
                message: "network timeout".into()
            },
            AppState::Error {
                message: "permission denied".into()
            },
        );
    }

    #[test]
    fn state_kind_matches_variant() {
        assert_eq!(AppState::Browse.kind(), StateKind::Browse);
        assert_eq!(
            AppState::Searching { query: "x".into() }.kind(),
            StateKind::Searching
        );
        assert_eq!(
            AppState::Filtering { query: "x".into() }.kind(),
            StateKind::Filtering
        );
        assert_eq!(AppState::Comparing.kind(), StateKind::Comparing);
        assert_eq!(
            AppState::Error {
                message: "oops".into()
            }
            .kind(),
            StateKind::Error
        );
    }

    #[test]
    fn load_state_loading_then_loaded() {
        let config = Config::default();
        let mut app = App::new(config).expect("app init");
        assert!(!app.loaded.contains_key(&0));

        app.loaded.insert(0, LoadState::Loading);
        assert!(matches!(app.loaded.get(&0), Some(LoadState::Loading)));

        app.loaded.insert(0, LoadState::Loaded(vec![]));
        assert!(matches!(app.loaded.get(&0), Some(LoadState::Loaded(_))));
    }
}
