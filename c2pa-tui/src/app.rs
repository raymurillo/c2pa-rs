use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tui_tree_widget::TreeState;

use crate::compare::diff::ManifestDiff;
use crate::config::Config;
use crate::error::{AppError, Result};
use crate::manifest::filter::FieldFilter;
use crate::manifest::loader::ManifestSource;
use crate::manifest::tree::DisplayNode;
use crate::remote::client::RemoteClient;
use crate::search::matcher::{MatchResult, Matcher};
use crate::ui::layout::CachedLayout;

// ---------------------------------------------------------------------------
// SourceId
// ---------------------------------------------------------------------------

/// Stable identity for a manifest source.
///
/// Assigned at `add_source()` time from a process-wide monotonically
/// increasing counter.  Unlike a `Vec` index, a `SourceId` remains valid if
/// other sources are removed or if the `sources` `Vec` is reordered.
///
/// ## Stability contract
///
/// - **Process-local**: values are assigned by an `AtomicU64` within the
///   current process; they are *not* stable across runs, *not* portable
///   across `App` instances, and MUST NOT be persisted to disk, logs, or
///   any other long-lived store.
/// - **Opaque**: the wrapped integer is an implementation detail.  Use
///   [`SourceId`]'s `Display`/`Debug` impls for human-readable output;
///   callers should not read or construct the numeric value directly.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceId(u64);

static NEXT_SOURCE_ID: AtomicU64 = AtomicU64::new(0);

impl SourceId {
    /// Allocate a new id from the process-wide counter.
    ///
    /// Uses `Ordering::Relaxed` because only atomic increment of the counter
    /// is required — no happens-before synchronisation with other memory is
    /// needed to establish uniqueness.
    fn next() -> Self {
        Self(NEXT_SOURCE_ID.fetch_add(1, Ordering::Relaxed))
    }
}

/// Prefixed with `src#` so the output is self-describing in error messages
/// and fallback labels, and cannot be confused with a raw integer identifier.
impl std::fmt::Display for SourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "src#{}", self.0)
    }
}

impl std::fmt::Debug for SourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Match Display so logs and {:?} output stay consistent — prevents
        // accidentally surfacing the raw number via tracing spans.
        std::fmt::Display::fmt(self, f)
    }
}

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
    /// tasks without requiring `ManifestSource: Clone`.  Each entry is paired
    /// with the stable [`SourceId`] assigned at registration time.
    pub sources: Vec<(SourceId, Arc<dyn ManifestSource>)>,
    /// Per-source loading state keyed by [`SourceId`].  A missing entry means
    /// "not yet requested".  Keying by id (not index) means the map survives
    /// reordering or removal of other sources.
    pub loaded: HashMap<SourceId, LoadState>,
    /// Identity of the source currently shown in the detail pane.
    /// `None` when `sources` is empty — prevents off-by-one panics.
    pub selected_left: Option<SourceId>,
    /// Identity of the right-side source for comparison, if any.
    pub compare_selection: Option<SourceId>,
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
    /// When in compare mode, whether equal rows are also shown (not just diffs).
    pub show_all_diffs: bool,
    /// Cached diff result for the current comparison pair.
    /// `None` means the cache is cold and must be (re)computed on the next draw.
    /// Invalidated whenever either compared source reloads or the pair changes.
    pub compare_diff_cache: Option<ManifestDiff>,
    /// Cached layout rects, invalidated on terminal resize.
    pub layout_cache: Option<(ratatui::layout::Rect, CachedLayout)>,
    /// Expand/collapse and scroll state for the detail tree.
    pub detail_tree_state: TreeState<String>,
    /// When `true` leaf nodes with empty/zero values are hidden from the detail tree.
    pub hide_empty: bool,
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

/// Walk `nodes` and collect the `Vec<String>` identifier paths for every
/// interior node (those with children), matching the dot-joined scheme used by
/// `node_to_tree_item` in the detail pane.
///
/// `depth` guards against stack overflow on pathological inputs; recursion
/// stops beyond 256 levels (consistent with `prune_to_matches` in detail.rs).
fn collect_all_interior_ids(
    nodes: &[DisplayNode],
    dot_prefix: &str,
    ancestor_path: &[String],
    depth: usize,
    out: &mut Vec<Vec<String>>,
) {
    if depth > 256 {
        return;
    }
    for node in nodes {
        if node.children.is_empty() {
            continue;
        }
        let id = if dot_prefix.is_empty() {
            node.key.clone()
        } else {
            format!("{}.{}", dot_prefix, node.key)
        };
        let mut path = ancestor_path.to_vec();
        path.push(id.clone());
        // Recurse before moving `path` into `out` so that the borrow of `path`
        // as `ancestor_path` is released first.  Push order (post-order here)
        // is irrelevant because `TreeState::open` inserts into a HashSet.
        collect_all_interior_ids(&node.children, &id, &path, depth + 1, out);
        out.push(path); // move — avoids one Vec clone per interior node
    }
}

impl App {
    /// Construct a new `App` from the given config.
    pub fn new(config: Config) -> Result<Self> {
        let client = RemoteClient::new()?;
        Ok(Self {
            sources: Vec::new(),
            loaded: HashMap::new(),
            selected_left: None,
            compare_selection: None,
            filter: FieldFilter::default(),
            matcher: Matcher::new(),
            state: AppState::Browse,
            config,
            client,
            focused_pane: Pane::FileList,
            show_help: false,
            show_all_diffs: false,
            compare_diff_cache: None,
            layout_cache: None,
            detail_tree_state: TreeState::default(),
            hide_empty: false,
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

        let (load_tx, mut load_rx) =
            mpsc::unbounded_channel::<(SourceId, Result<Vec<DisplayNode>>)>();

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
                Some((id, result)) = load_rx.recv() => {
                    self.handle_load_result(id, result);
                }
            }
        }

        Ok(())
    }

    async fn handle_event(
        &mut self,
        event: crossterm::event::Event,
        load_tx: &tokio::sync::mpsc::UnboundedSender<(SourceId, Result<Vec<DisplayNode>>)>,
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
        load_tx: &tokio::sync::mpsc::UnboundedSender<(SourceId, Result<Vec<DisplayNode>>)>,
    ) -> Result<()> {
        use crossterm::event::KeyCode;
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if self.focused_pane == Pane::Detail {
                    self.detail_tree_state.key_up();
                } else {
                    self.select_prev();
                    self.reindex_for_selected();
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.focused_pane == Pane::Detail {
                    self.detail_tree_state.key_down();
                } else {
                    self.select_next();
                    self.reindex_for_selected();
                }
            }
            KeyCode::Enter => {
                if let Some(id) = self.selected_left {
                    self.trigger_load(id, false, load_tx);
                }
            }
            KeyCode::Char('r') => {
                if let Some(id) = self.selected_left {
                    self.trigger_load(id, true, load_tx);
                }
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
            KeyCode::Char('c') => {
                // Always bust the cache: either a new left bookmark is being set or
                // a new comparison is starting with a potentially different pair.
                self.compare_diff_cache = None;
                match self.compare_selection {
                    None => self.compare_selection = self.selected_left,
                    Some(_) => self.state = AppState::Comparing,
                }
            }
            KeyCode::Esc => {
                self.compare_selection = None;
            }
            KeyCode::Char('?') => {
                self.show_help = !self.show_help;
            }
            KeyCode::Char(' ') if self.focused_pane == Pane::Detail => {
                self.detail_tree_state.toggle_selected();
            }
            KeyCode::Char('h') if self.focused_pane == Pane::Detail => {
                self.detail_tree_state.key_left();
            }
            KeyCode::Char('l') if self.focused_pane == Pane::Detail => {
                self.detail_tree_state.key_right();
            }
            KeyCode::Char('E') if self.focused_pane == Pane::Detail => {
                self.expand_all();
            }
            KeyCode::Char('W') if self.focused_pane == Pane::Detail => {
                self.detail_tree_state.close_all();
            }
            KeyCode::Char('e') if self.focused_pane == Pane::Detail => {
                self.hide_empty = !self.hide_empty;
            }
            _ => {}
        }
        Ok(())
    }

    fn expand_all(&mut self) {
        let Some(id) = self.selected_left else {
            return;
        };
        if let Some(LoadState::Loaded(nodes)) = self.loaded.get(&id) {
            // Intentionally uses the raw (unfiltered) node tree so that
            // toggling off a field filter or hide_empty after pressing E
            // reveals a fully-expanded tree rather than a partially-expanded one.
            // Phantom IDs (for currently-filtered nodes) are silently ignored
            // by tui-tree-widget's TreeState.
            let mut ids = Vec::new();
            collect_all_interior_ids(nodes, "", &[], 0, &mut ids);
            for id in ids {
                self.detail_tree_state.open(id);
            }
        }
    }

    fn trigger_load(
        &mut self,
        id: SourceId,
        force: bool,
        load_tx: &tokio::sync::mpsc::UnboundedSender<(SourceId, Result<Vec<DisplayNode>>)>,
    ) {
        match self.loaded.get(&id) {
            Some(LoadState::Loaded(_)) if !force => return,
            Some(LoadState::Loading) => return,
            _ => {}
        }

        // Resolve source before mutating state so we never enter Loading
        // without a corresponding background task.
        let Some(src) = self.source_by_id(id).cloned() else {
            return;
        };
        self.loaded.insert(id, LoadState::Loading);
        self.loading_count += 1;

        let client = self.client.clone();
        let tx = load_tx.clone();
        tokio::spawn(async move {
            let result = src.load(&client).await;
            let _ = tx.send((id, result));
        });
    }

    fn handle_load_result(&mut self, id: SourceId, result: Result<Vec<DisplayNode>>) {
        self.loading_count = self.loading_count.saturating_sub(1);
        match result {
            Ok(nodes) => {
                self.loaded.insert(id, LoadState::Loaded(nodes));
                // Only reset expand/collapse state when the loaded source is
                // the one currently on display; background loads for other
                // sources should not collapse the tree the user is browsing.
                if self.selected_left == Some(id) {
                    self.detail_tree_state = TreeState::default();
                    // Re-index so the matcher is ready the moment the user
                    // presses '/' — avoids the index cost on the first keystroke.
                    self.reindex_for_selected();
                }
                // Invalidate the cached diff if the newly loaded source is either
                // half of the current comparison pair — stale data must not linger.
                if self.selected_left == Some(id) || self.compare_selection == Some(id) {
                    self.compare_diff_cache = None;
                }
            }
            Err(e) => {
                self.loaded.remove(&id);
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
        let loaded_nodes = self
            .selected_left
            .and_then(|id| self.loaded.get(&id))
            .and_then(|s| match s {
                LoadState::Loaded(nodes) => Some(nodes.as_slice()),
                _ => None,
            });
        match loaded_nodes {
            Some(nodes) => {
                let flat = crate::manifest::tree::flatten(nodes);
                self.matcher.index(&flat);
            }
            None => {
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
        match key.code {
            KeyCode::Esc => {
                self.compare_selection = None;
                self.compare_diff_cache = None;
                self.state = AppState::Browse;
            }
            KeyCode::Char('a') => {
                self.show_all_diffs = !self.show_all_diffs;
            }
            _ => {}
        }
    }

    fn handle_mouse(&mut self, event: crossterm::event::MouseEvent) {
        use crossterm::event::{MouseButton, MouseEventKind};

        match event.kind {
            MouseEventKind::ScrollDown => match self.focused_pane {
                Pane::FileList => {
                    self.select_next();
                }
                Pane::Detail => {
                    self.detail_tree_state.scroll_down(1);
                }
            },
            MouseEventKind::ScrollUp => match self.focused_pane {
                Pane::FileList => {
                    self.select_prev();
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
                        if let Some(id) = self.id_at(row) {
                            self.selected_left = Some(id);
                        }
                    } else {
                        self.focused_pane = Pane::Detail;
                    }
                }
            }
            _ => {}
        }
    }

    // -----------------------------------------------------------------------
    // SourceId helpers
    // -----------------------------------------------------------------------

    /// Register a new manifest source and return its stable [`SourceId`].
    ///
    /// If this is the first source added, `selected_left` is initialised to
    /// the new id so the file list has a valid cursor.
    pub fn add_source(&mut self, src: Arc<dyn ManifestSource>) -> SourceId {
        let id = SourceId::next();
        self.sources.push((id, src));
        if self.selected_left.is_none() {
            self.selected_left = Some(id);
        }
        id
    }

    /// Expand a directory into individual [`FileSource`](crate::manifest::loader::FileSource)
    /// entries and register each one.
    ///
    /// Offloads the blocking `walkdir` traversal to the tokio blocking thread
    /// pool so that this call never stalls the async runtime.  Returns the
    /// stable `SourceId`s assigned to each file, in directory-enumeration order.
    pub async fn add_dir(&mut self, path: std::path::PathBuf) -> Result<Vec<SourceId>> {
        let entries = crate::manifest::loader::DirSource::new(path)
            .entries_async()
            .await?;
        let mut ids = Vec::with_capacity(entries.len());
        for file_src in entries {
            ids.push(self.add_source(Arc::new(file_src)));
        }
        Ok(ids)
    }

    /// Return the `Arc<dyn ManifestSource>` for the given id, if present.
    ///
    /// The linear scan is acceptable here — `sources` is bounded by the number
    /// of files the user selected on the command line (typically O(10)) and
    /// this path is not in the per-frame draw hot loop.
    pub fn source_by_id(&self, id: SourceId) -> Option<&Arc<dyn ManifestSource>> {
        self.sources
            .iter()
            .find_map(|(sid, src)| (*sid == id).then_some(src))
    }

    /// Return the list-position index of the given [`SourceId`] within `sources`.
    #[inline]
    pub fn index_of(&self, id: SourceId) -> Option<usize> {
        self.sources.iter().position(|(sid, _)| *sid == id)
    }

    /// Return the [`SourceId`] at the given list-position index.
    #[inline]
    pub fn id_at(&self, idx: usize) -> Option<SourceId> {
        self.sources.get(idx).map(|(id, _)| *id)
    }

    /// Move the file-list cursor by `delta` positions, saturating at both
    /// ends.  Positive values move down (next), negative values move up
    /// (previous).  A no-op if `sources` is empty.
    ///
    /// Unifies next/prev navigation into a single linear scan — each arrow
    /// key press touches `sources` exactly once (not twice, as separate
    /// `index_of`/`id_at` calls would).
    fn move_selection(&mut self, delta: isize) {
        let len = self.sources.len();
        if len == 0 {
            self.selected_left = None;
            return;
        }
        let current_idx = self
            .selected_left
            .and_then(|id| self.index_of(id))
            .unwrap_or(0);
        // Saturating arithmetic on isize first, then clamped to the valid
        // index range [0, len - 1].  `len - 1` is safe because `len > 0`.
        let next = (current_idx as isize)
            .saturating_add(delta)
            .clamp(0, (len - 1) as isize) as usize;
        self.selected_left = self.id_at(next);
    }

    #[inline]
    fn select_next(&mut self) {
        self.move_selection(1);
    }

    #[inline]
    fn select_prev(&mut self) {
        self.move_selection(-1);
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

    // --- collect_all_interior_ids ---

    fn leaf_dn(key: &str) -> DisplayNode {
        DisplayNode {
            key: key.to_owned(),
            value: crate::manifest::tree::NodeValue::Str("v".into()),
            children: vec![],
        }
    }

    fn branch_dn(key: &str, children: Vec<DisplayNode>) -> DisplayNode {
        DisplayNode {
            key: key.to_owned(),
            value: crate::manifest::tree::NodeValue::Missing,
            children,
        }
    }

    fn collect(nodes: &[DisplayNode]) -> Vec<Vec<String>> {
        let mut out = Vec::new();
        collect_all_interior_ids(nodes, "", &[], 0, &mut out);
        out
    }

    #[test]
    fn collect_ids_empty_input() {
        assert!(collect(&[]).is_empty());
    }

    #[test]
    fn collect_ids_only_leaves() {
        let nodes = vec![leaf_dn("a"), leaf_dn("b")];
        assert!(collect(&nodes).is_empty());
    }

    #[test]
    fn collect_ids_single_root_interior() {
        let nodes = vec![branch_dn("Root", vec![leaf_dn("child")])];
        let ids = collect(&nodes);
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0], vec!["Root"]);
    }

    #[test]
    fn collect_ids_nested_paths_match_node_to_tree_item_scheme() {
        // The dot-joined path used here must match the `id` computed inside
        // `node_to_tree_item` in detail.rs — that's the contract.
        let nodes = vec![branch_dn(
            "Manifest",
            vec![branch_dn("Claim", vec![leaf_dn("title")])],
        )];
        let ids = collect(&nodes);
        // Both "Manifest" and "Manifest.Claim" are interior nodes.
        assert_eq!(ids.len(), 2);
        let id_set: std::collections::HashSet<Vec<String>> = ids.into_iter().collect();
        assert!(id_set.contains(&vec!["Manifest".to_owned()]));
        assert!(id_set.contains(&vec!["Manifest".to_owned(), "Manifest.Claim".to_owned()]));
    }

    #[test]
    fn collect_ids_depth_guard_stops_at_256() {
        fn deep(depth: usize) -> DisplayNode {
            if depth == 0 {
                leaf_dn("leaf")
            } else {
                branch_dn("n", vec![deep(depth - 1)])
            }
        }
        let nodes = vec![deep(300)];
        let ids = collect(&nodes);
        // Must not overflow; depth guard caps at 256 levels.
        assert!(ids.len() <= 257, "should not recurse beyond depth 256");
    }

    #[test]
    fn load_state_loading_then_loaded() {
        let config = Config::default();
        let mut app = App::new(config).expect("app init");
        let id = SourceId::next();
        assert!(!app.loaded.contains_key(&id));

        app.loaded.insert(id, LoadState::Loading);
        assert!(matches!(app.loaded.get(&id), Some(LoadState::Loading)));

        app.loaded.insert(id, LoadState::Loaded(vec![]));
        assert!(matches!(app.loaded.get(&id), Some(LoadState::Loaded(_))));
    }

    // --- SourceId behaviour ---

    #[test]
    fn source_id_is_unique_across_allocations() {
        let a = SourceId::next();
        let b = SourceId::next();
        let c = SourceId::next();
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, c);
    }

    #[test]
    fn add_source_returns_unique_ids_and_sets_initial_selection() {
        use crate::manifest::loader::MockManifestSource;
        let mut app = App::new(Config::default()).unwrap();
        assert!(app.selected_left.is_none());

        let id0 = app.add_source(Arc::new({
            let mut m = MockManifestSource::new();
            m.expect_label().return_const("a".to_owned());
            m.expect_is_remote().return_const(false);
            m
        }));
        let id1 = app.add_source(Arc::new({
            let mut m = MockManifestSource::new();
            m.expect_label().return_const("b".to_owned());
            m.expect_is_remote().return_const(false);
            m
        }));

        assert_ne!(id0, id1);
        // First add initialises selection; subsequent adds leave it alone.
        assert_eq!(app.selected_left, Some(id0));
        assert_eq!(app.index_of(id0), Some(0));
        assert_eq!(app.index_of(id1), Some(1));
        assert_eq!(app.id_at(0), Some(id0));
        assert_eq!(app.id_at(1), Some(id1));
    }

    #[test]
    fn index_of_returns_none_for_unknown_id() {
        let app = App::new(Config::default()).unwrap();
        let stray = SourceId::next();
        assert!(app.index_of(stray).is_none());
        assert!(app.id_at(0).is_none());
    }

    #[test]
    fn select_next_prev_bound_to_sources() {
        use crate::manifest::loader::MockManifestSource;
        let mut app = App::new(Config::default()).unwrap();
        let mk = || {
            let mut m = MockManifestSource::new();
            m.expect_label().return_const("x".to_owned());
            m.expect_is_remote().return_const(false);
            Arc::new(m) as Arc<dyn ManifestSource>
        };
        let id0 = app.add_source(mk());
        let id1 = app.add_source(mk());
        let id2 = app.add_source(mk());

        app.selected_left = Some(id0);
        app.select_next();
        assert_eq!(app.selected_left, Some(id1));
        app.select_prev();
        assert_eq!(app.selected_left, Some(id0));
        // Prev at head saturates.
        app.select_prev();
        assert_eq!(app.selected_left, Some(id0));
        // Next at tail saturates.
        app.selected_left = Some(id2);
        app.select_next();
        assert_eq!(app.selected_left, Some(id2));
    }

    #[test]
    fn move_selection_is_noop_on_empty_sources() {
        let mut app = App::new(Config::default()).unwrap();
        // Empty app — selected_left is None, and stays None regardless of delta.
        app.move_selection(1);
        assert!(app.selected_left.is_none());
        app.move_selection(-1);
        assert!(app.selected_left.is_none());
        app.move_selection(isize::MAX);
        assert!(app.selected_left.is_none());
    }

    #[test]
    fn move_selection_clamps_large_deltas() {
        use crate::manifest::loader::MockManifestSource;
        let mut app = App::new(Config::default()).unwrap();
        let mk = || {
            let mut m = MockManifestSource::new();
            m.expect_label().return_const("x".to_owned());
            m.expect_is_remote().return_const(false);
            Arc::new(m) as Arc<dyn ManifestSource>
        };
        let first = app.add_source(mk());
        let _mid = app.add_source(mk());
        let last = app.add_source(mk());

        app.selected_left = Some(first);
        app.move_selection(isize::MAX);
        assert_eq!(app.selected_left, Some(last), "clamp to tail");

        app.move_selection(isize::MIN);
        assert_eq!(app.selected_left, Some(first), "clamp to head");
    }

    #[test]
    fn move_selection_recovers_from_stale_selected_left() {
        use crate::manifest::loader::MockManifestSource;
        let mut app = App::new(Config::default()).unwrap();
        let mut m = MockManifestSource::new();
        m.expect_label().return_const("x".to_owned());
        m.expect_is_remote().return_const(false);
        let id = app.add_source(Arc::new(m));

        // Simulate a stale id (e.g. source was removed in a future feature).
        app.selected_left = Some(SourceId::next());
        app.move_selection(1);
        // Should fall back to position 0, which is the real live source.
        assert_eq!(app.selected_left, Some(id));
    }

    // --- SourceId formatting ---

    #[test]
    fn source_id_display_is_prefixed() {
        // Allocate a fresh id, then confirm Display/Debug are stable and
        // carry the `src#` prefix so log output is self-describing.
        let id = SourceId::next();
        let disp = format!("{id}");
        let dbg = format!("{id:?}");
        assert!(disp.starts_with("src#"), "display missing prefix: {disp}");
        assert_eq!(disp, dbg, "Debug must match Display");
    }

    proptest::proptest! {
        /// Every [`SourceId`] returned by `next()` must be unique within a
        /// single burst.  The guarantee is provided by the `AtomicU64`
        /// counter; this test pins the invariant against future refactors
        /// (e.g. switching to a hash or a reused pool) that could silently
        /// break it.
        #[test]
        fn source_id_next_is_unique(n in 1usize..512) {
            let ids: std::collections::HashSet<SourceId> =
                (0..n).map(|_| SourceId::next()).collect();
            proptest::prop_assert_eq!(ids.len(), n);
        }
    }
}
