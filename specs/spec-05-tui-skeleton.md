# Spec 05 — TUI Skeleton & App State Machine

**Phase:** 1 (concurrent with spec-01, spec-02, spec-03, spec-04)  
**Depends on:** spec-00 foundation committed and compiling  
**Produces:** `app.rs` fully implemented, `ui/layout.rs` complete, crossterm
terminal lifecycle wired up in `app.rs`. The event loop runs but all widget
draw calls remain stubs (filled in by spec-06, spec-07, spec-08).

---

## Goal

Build the event loop core: initialize the terminal in raw mode, dispatch input
events to `App` state transitions, run async source loading in background tasks,
drive the ratatui render loop, and restore the terminal cleanly on exit or panic.

No `.unwrap()` in `App` methods — propagate all errors via `Result`. Initialize
`tracing_subscriber` in `App::run` (reads `RUST_LOG`, writes to stderr) before the
TUI takes over stdout.

This spec does NOT implement any widget drawing — `ui::draw` stays as a stub.
The value here is a working, interactive skeleton: you can run the binary, see a
blank TUI, and quit with `q`.

---

## Files to modify

- `src/app.rs` — full implementation
- `src/ui/layout.rs` — layout constants and `split_layout` helper
- `src/ui/mod.rs` — update `draw()` stub to call sub-module stubs with correct areas

---

## `src/ui/layout.rs`

```rust
use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Horizontal split: left pane (file list) + right pane (detail/compare).
pub fn split_horizontal(area: Rect, left_pct: u16) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(left_pct),
            Constraint::Percentage(100 - left_pct),
        ])
        .split(area);
    (chunks[0], chunks[1])
}

/// Vertical split: main area + status bar (1 line).
pub fn split_status(area: Rect) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(area);
    (chunks[0], chunks[1])
}

/// Centered floating rect for overlays (search bar, filter bar, error).
/// `width_pct` and `height_pct` are percentages of `area`.
pub fn centered_popup(area: Rect, width_pct: u16, height_pct: u16) -> Rect {
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_pct) / 2),
            Constraint::Percentage(width_pct),
            Constraint::Percentage((100 - width_pct) / 2),
        ])
        .split(area);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height_pct) / 2),
            Constraint::Percentage(height_pct),
            Constraint::Percentage((100 - height_pct) / 2),
        ])
        .split(horizontal[1]);
    vertical[1]
}
```

---

## `src/app.rs`

### `App::new`

```rust
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
    })
}
```

### `App::run` — main TUI event loop

```rust
pub async fn run(mut self) -> Result<()> {
    // Initialize tracing to stderr before we take over stdout with the TUI.
    // Users can set RUST_LOG=debug to see structured logs alongside the UI.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    // Setup terminal
    crossterm::terminal::enable_raw_mode()
        .map_err(|e| AppError::Terminal(e.to_string()))?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture,
    ).map_err(|e| AppError::Terminal(e.to_string()))?;

    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)
        .map_err(|e| AppError::Terminal(e.to_string()))?;

    // Install panic hook that restores terminal before printing panic message
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

    let result = self.event_loop(&mut terminal).await;

    // Always restore terminal
    crossterm::terminal::disable_raw_mode()
        .map_err(|e| AppError::Terminal(e.to_string()))?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture,
    ).map_err(|e| AppError::Terminal(e.to_string()))?;
    terminal.show_cursor()
        .map_err(|e| AppError::Terminal(e.to_string()))?;

    result
}
```

### `event_loop`

```rust
async fn event_loop(
    &mut self,
    terminal: &mut ratatui::Terminal<ratatui::backend::CrosstermBackend<std::io::Stdout>>,
) -> Result<()> {
    use crossterm::event::{EventStream, Event, KeyCode, KeyModifiers, MouseEventKind};
    use futures::StreamExt;
    use tokio::sync::mpsc;

    // Channel for background load results: (source_index, result)
    let (load_tx, mut load_rx) = mpsc::unbounded_channel::<(usize, Result<Vec<DisplayNode>>)>();

    let mut event_stream = EventStream::new();

    loop {
        terminal.draw(|f| crate::ui::draw(f, self))
            .map_err(|e| AppError::Terminal(e.to_string()))?;

        tokio::select! {
            // Terminal event
            Some(Ok(event)) = event_stream.next() => {
                if self.handle_event(event, &load_tx).await? {
                    break; // quit
                }
            }
            // Background load completed
            Some((idx, result)) = load_rx.recv() => {
                self.handle_load_result(idx, result);
            }
        }
    }

    Ok(())
}
```

Add `futures = "0.3"` to `Cargo.toml` for `StreamExt`.

### `handle_event` — input dispatch

```rust
async fn handle_event(
    &mut self,
    event: crossterm::event::Event,
    load_tx: &tokio::sync::mpsc::UnboundedSender<(usize, Result<Vec<DisplayNode>>)>,
) -> Result<bool> {
    use crossterm::event::{Event, KeyCode, KeyModifiers, MouseEventKind, MouseButton};

    match event {
        Event::Key(key) => {
            // Global quit: q or Ctrl+C
            if matches!(key.code, KeyCode::Char('q'))
                || (key.code == KeyCode::Char('c')
                    && key.modifiers.contains(KeyModifiers::CONTROL))
            {
                return Ok(true); // signal quit
            }

            match &self.state {
                AppState::Browse => self.handle_browse_key(key, load_tx).await?,
                AppState::Searching { .. } => self.handle_search_key(key),
                AppState::Filtering { .. } => self.handle_filter_key(key),
                AppState::Error { .. } => {
                    // Any key dismisses error overlay
                    self.state = AppState::Browse;
                }
                AppState::Loading { .. } => {
                    // Only Ctrl+C / q handled above; ignore other keys while loading
                }
                AppState::Comparing => self.handle_compare_key(key),
            }
        }
        Event::Mouse(mouse) if self.config.mouse_enabled => {
            self.handle_mouse(mouse);
        }
        Event::Resize(_, _) => {
            // ratatui handles resize automatically on next draw
        }
        _ => {}
    }

    Ok(false)
}
```

### Key handlers (Browse mode)

```rust
fn handle_browse_key(
    &mut self,
    key: crossterm::event::KeyEvent,
    load_tx: &tokio::sync::mpsc::UnboundedSender<(usize, Result<Vec<DisplayNode>>)>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + '_>> {
    // use async block and box it, OR make the parent handle_browse_key non-async
    // and spawn tasks. Simpler: make it sync and spawn tasks inline.
    todo!("implement key dispatch")
}
```

Implement as a regular (non-async) method that spawns tokio tasks for loading.
Key actions in Browse mode:

| Key | Action |
|---|---|
| `↑` / `k` | `selected_left = selected_left.saturating_sub(1)` |
| `↓` / `j` | `selected_left = min(selected_left + 1, sources.len().saturating_sub(1))` |
| `Enter` | trigger load of `sources[selected_left]` (see "Loading flow" below) |
| `r` | same as Enter but force-reload (remove from `loaded` cache first) |
| `Tab` | toggle focus (add `focused_pane: Pane` to App — see below) |
| `/` | `self.state = AppState::Searching { query: String::new() }` |
| `f` | `self.state = AppState::Filtering { query: String::new() }` |
| `c` | if `compare_selection.is_none()`, set to `Some(selected_left)`; else switch to `AppState::Comparing` |
| `Esc` | `compare_selection = None` |
| `?` | toggle help overlay (add `show_help: bool` to App) |

### Loading flow

When the user presses Enter on a source:

1. If already in `loaded`, do nothing (it's cached).
2. Set `state = AppState::Loading { source_index: idx }`.
3. Spawn a tokio task that calls `source.load(&client).await` and sends the result
   on `load_tx`.

```rust
fn trigger_load(
    &mut self,
    idx: usize,
    force: bool,
    load_tx: &tokio::sync::mpsc::UnboundedSender<(usize, Result<Vec<DisplayNode>>)>,
) {
    if !force && self.loaded.contains_key(&idx) {
        return;
    }
    self.loaded.remove(&idx);
    self.state = AppState::Loading { source_index: idx };

    // Clone what the task needs (sources aren't Clone — use Arc or index into a shared vec)
    // Simplest approach: store sources as Arc<Vec<Box<dyn ManifestSource>>>
    // OR: spawn the load inline using a channel and a handle to the client.
    // Implementation detail: the foundation stub uses Vec directly.
    // You will need to change sources to Arc<[Box<dyn ManifestSource>]> or similar
    // to share ownership with the spawned task.
    // Alternatively, expose a load method via an index + shared Arc<Mutex<...>>.
    // Choose whichever is cleanest.
    let tx = load_tx.clone();
    // ... spawn task
    todo!("implement trigger_load task spawn")
}
```

> **Design decision:** Pick an ownership strategy for sharing sources with spawned
> tasks. Options: `Arc<Vec<Box<dyn ManifestSource>>>`, message passing, or using
> `tokio::task::spawn_blocking`. Document your choice in a comment.

### `handle_load_result`

```rust
fn handle_load_result(&mut self, idx: usize, result: Result<Vec<DisplayNode>>) {
    match result {
        Ok(nodes) => {
            self.loaded.insert(idx, nodes);
            // If this was the loading source, return to Browse
            if self.state == (AppState::Loading { source_index: idx }) {
                self.state = AppState::Browse;
            }
        }
        Err(e) => {
            self.state = AppState::Error { message: e.to_string() };
        }
    }
}
```

### Search and filter key handlers

```rust
fn handle_search_key(&mut self, key: crossterm::event::KeyEvent) {
    use crossterm::event::KeyCode;
    match key.code {
        KeyCode::Esc => self.state = AppState::Browse,
        KeyCode::Char(c) => {
            if let AppState::Searching { query } = &mut self.state {
                query.push(c);
            }
        }
        KeyCode::Backspace => {
            if let AppState::Searching { query } = &mut self.state {
                query.pop();
            }
        }
        _ => {}
    }
}

fn handle_filter_key(&mut self, key: crossterm::event::KeyEvent) {
    use crossterm::event::KeyCode;
    match key.code {
        KeyCode::Esc => self.state = AppState::Browse,
        KeyCode::Enter => {
            if let AppState::Filtering { query } = &self.state.clone() {
                match FieldFilter::from_query(query) {
                    Ok(f) => {
                        self.filter = f;
                        self.state = AppState::Browse;
                    }
                    Err(e) => {
                        self.state = AppState::Error { message: e.to_string() };
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
            self.state = AppState::Browse;
        }
        _ => {}
    }
}
```

### Mouse handler (stub for now — detail in spec-06)

```rust
fn handle_mouse(&mut self, _event: crossterm::event::MouseEvent) {
    // Delegated to spec-06 (panes)
}
```

### `focused_pane` and `show_help` additions to `App`

Add these fields to the `App` struct (update both the struct definition and `App::new`):

```rust
pub focused_pane: Pane,
pub show_help: bool,
```

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pane {
    FileList,
    Detail,
}
```

---

## `src/ui/mod.rs` — update draw stub

Update the stub to call layout helpers and pass the correct areas to sub-module stubs:

```rust
use ratatui::Frame;
use crate::app::App;
use crate::ui::layout::{split_horizontal, split_status};

pub fn draw(frame: &mut Frame, app: &App) {
    let (main_area, status_area) = split_status(frame.area());
    let (list_area, detail_area) = split_horizontal(main_area, app.config.left_pane_pct);

    file_list::draw(frame, list_area, app);
    detail::draw(frame, detail_area, app);
    status_bar::draw(frame, status_area, app);

    // Overlays drawn last (on top)
    match &app.state {
        crate::app::AppState::Searching { .. } => search_bar::draw(frame, frame.area(), app),
        crate::app::AppState::Filtering { .. } => filter_bar::draw(frame, frame.area(), app),
        crate::app::AppState::Comparing => compare::draw(frame, detail_area, app),
        crate::app::AppState::Error { message } => draw_error_overlay(frame, frame.area(), message),
        _ => {}
    }
}

fn draw_error_overlay(frame: &mut Frame, area: ratatui::layout::Rect, message: &str) {
    use ratatui::widgets::{Block, Borders, Paragraph};
    use ratatui::style::{Color, Style};
    use crate::ui::layout::centered_popup;

    let popup_area = centered_popup(area, 60, 20);
    frame.render_widget(
        Paragraph::new(format!("Error: {message}\n\nPress any key to dismiss."))
            .block(Block::default().borders(Borders::ALL).title("Error")
                .border_style(Style::default().fg(Color::Red))),
        popup_area,
    );
}
```

---

## `src/main.rs` — wire into `App::run`

Leave full CLI implementation to spec-09, but make the binary runnable:

```rust
fn main() {
    let config = c2pa_tui::config::Config::default();
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let app = c2pa_tui::app::App::new(config).expect("app init");
    if let Err(e) = rt.block_on(app.run()) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
```

---

## Done criteria

```
cargo build
cargo run                    # blank TUI starts, q quits, terminal restored
cargo fmt -- --check
cargo clippy -- -D warnings
```

No panics on normal usage paths. The binary must not leave the terminal in raw
mode if it crashes — the panic hook must work.
