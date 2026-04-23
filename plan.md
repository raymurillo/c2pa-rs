# c2pa-tui — Implementation Plan

## Overview

A standalone Rust workspace providing a terminal user interface (TUI) for loading,
viewing, filtering, and comparing C2PA manifests from local files, directories, and
remote HTTP endpoints.

**Crate name:** `c2pa-tui`  
**Binary:** `c2pa-tui`  
**Rust edition:** 2021  
**MSRV:** 1.88.0 (aligned with `c2pa` SDK)  

---

## Workspace layout

```
c2pa-tui/
├── Cargo.toml                  # workspace root
├── Cargo.lock
├── src/
│   ├── main.rs                 # clap arg parsing → App::run()
│   ├── app.rs                  # top-level App, event loop, global state
│   ├── error.rs                # AppError (thiserror)
│   ├── config.rs               # runtime config (auth, colours, keybinds)
│   ├── manifest/
│   │   ├── mod.rs
│   │   ├── loader.rs           # ManifestSource impls (file, dir, remote)
│   │   ├── tree.rs             # ManifestStore → DisplayNode tree
│   │   └── filter.rs           # field filter / projection logic
│   ├── remote/
│   │   ├── mod.rs
│   │   ├── client.rs           # reqwest async client, retry, timeout
│   │   └── auth.rs             # Auth enum (None, Basic, Bearer, Digest)
│   ├── search/
│   │   ├── mod.rs
│   │   └── matcher.rs          # fuzzy/substring engine (nucleo)
│   ├── compare/
│   │   ├── mod.rs
│   │   └── diff.rs             # field-level diff across two manifests
│   └── ui/
│       ├── mod.rs              # draw() top-level, widget dispatch
│       ├── layout.rs           # split layout constants and helpers
│       ├── file_list.rs        # left-pane: file/URL list widget
│       ├── detail.rs           # right-pane: tree view of one manifest
│       ├── compare.rs          # right-pane: side-by-side diff view
│       ├── search_bar.rs       # inline search overlay widget
│       ├── filter_bar.rs       # field-filter overlay widget
│       └── status_bar.rs       # bottom status / key hint bar
└── tests/
    ├── fixtures/               # small signed JPEG/PNG/PDF test assets
    ├── integration_loader.rs   # load real files → assert tree shape
    ├── integration_remote.rs   # wiremock server → assert remote load
    └── snapshot_ui.rs          # insta ratatui buffer snapshots
```

---

## Key dependencies

| Crate | Purpose |
|---|---|
| `ratatui` | TUI framework |
| `crossterm` | terminal backend |
| `c2pa` | manifest parsing (path dep or crates.io) |
| `tokio` (full) | async runtime |
| `reqwest` (rustls-tls) | HTTP client for remote manifests |
| `clap` (derive) | CLI argument parsing |
| `thiserror` | structured domain-specific error types |
| `anyhow` | error context wrapping at call sites |
| `serde_json` | raw JSON rendering in detail view |
| `nucleo` | fuzzy/substring matching engine |
| `tui-tree-widget` | collapsible tree widget for ratatui |
| `tracing` | structured, leveled logging with spans |
| `tracing-subscriber` | log output formatting (stderr, RUST_LOG) |
| `insta` (dev) | snapshot tests for TUI buffer output |
| `wiremock` (dev) | mock HTTP server for remote tests |
| `mockall` (dev) | mock trait implementations for unit tests |
| `proptest` (dev) | property-based testing |
| `tempfile` (dev) | temporary files in tests |
| `walkdir` | recursive directory traversal |

---

## Module details

### `error.rs`

```rust
#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("c2pa error: {0}")]
    C2pa(#[from] c2pa::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("authentication error: {0}")]
    Auth(String),

    #[error("unsupported file type: {0}")]
    UnsupportedFormat(String),

    #[error("manifest not found in {0}")]
    NoManifest(String),

    #[error("terminal error: {0}")]
    Terminal(String),

    #[error("walk error: {0}")]
    Walk(#[from] walkdir::Error),
}

pub type Result<T> = std::result::Result<T, AppError>;
```

---

### Trait definitions

#### `ManifestSource` — `manifest/loader.rs`

```rust
/// Abstraction over all manifest origins (file, directory, remote URL).
#[async_trait::async_trait]
pub trait ManifestSource: Send + Sync {
    /// Human-readable label shown in the file list pane.
    fn label(&self) -> &str;

    /// Load and return the manifest store for this source.
    async fn load(&self, client: &RemoteClient) -> Result<c2pa::ManifestStore>;

    /// Whether this source can be refreshed (remote sources → true).
    fn is_remote(&self) -> bool { false }
}
```

Implementations:
- `FileSource(PathBuf)` — single local file
- `DirSource(PathBuf)` — yields one `FileSource` per supported file found via `walkdir`
- `RemoteSource { url: Url, auth: Auth }` — fetches asset bytes over HTTP

#### `DisplayNode` — `manifest/tree.rs`

```rust
/// One node in the rendered manifest tree.
pub struct DisplayNode {
    pub key: String,
    pub value: NodeValue,
    pub children: Vec<DisplayNode>,
}

pub enum NodeValue {
    Str(String),
    Json(serde_json::Value),
    Bytes(usize),          // binary blobs shown as byte count
    Missing,               // field present but empty
}
```

`ManifestStore → Vec<DisplayNode>` conversion flattens the C2PA structure into:
- **Claim** (label, instance id, format, title)
- **Claim signature** (issuer, time, alg)
- **Assertions** (each assertion as an expandable subtree)
- **Ingredients** (recursive, showing thumbnail hash + manifest ref)
- **Validation status** (per-assertion pass/fail + error messages)

#### `FieldFilter` — `manifest/filter.rs`

```rust
pub struct FieldFilter {
    pub include_paths: Vec<glob::Pattern>,  // e.g. "assertions.*"
    pub exclude_paths: Vec<glob::Pattern>,
}

impl FieldFilter {
    pub fn apply(&self, nodes: Vec<DisplayNode>) -> Vec<DisplayNode>;
    pub fn from_query(q: &str) -> Result<Self>;
}
```

#### `Matcher` — `search/matcher.rs`

```rust
pub struct Matcher {
    engine: nucleo::Nucleo<usize>,  // item = index into flat node list
}

impl Matcher {
    pub fn new() -> Self;
    pub fn index(&mut self, nodes: &[FlatNode]);
    pub fn query(&mut self, pattern: &str) -> Vec<MatchResult>;
}

pub struct MatchResult {
    pub node_index: usize,
    pub score: u32,
    pub highlight_ranges: Vec<Range<usize>>,
}
```

#### `ManifestDiff` — `compare/diff.rs`

```rust
pub struct ManifestDiff {
    pub left_label: String,
    pub right_label: String,
    pub fields: Vec<FieldDiff>,
}

pub enum FieldDiff {
    Equal  { path: String, value: String },
    Changed{ path: String, left: String, right: String },
    OnlyLeft { path: String, value: String },
    OnlyRight{ path: String, value: String },
}

pub fn diff(left: &[DisplayNode], right: &[DisplayNode]) -> ManifestDiff;
```

#### `Auth` — `remote/auth.rs`

```rust
pub enum Auth {
    None,
    Basic { username: String, password: String },
    Bearer { token: String },
    Digest { username: String, password: String },
}

impl Auth {
    /// Apply auth headers / `reqwest::RequestBuilder` decoration.
    pub fn apply(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder;
    /// Parse from CLI flag string: `basic:user:pass`, `bearer:<token>`, `digest:user:pass`.
    pub fn from_str(s: &str) -> Result<Self>;
}
```

---

### `app.rs` — App state machine

```
States:
  Browse       — normal left/right pane navigation
  Searching    — search bar active, results highlighted in detail pane
  Filtering    — filter bar active
  Comparing    — two manifests selected, diff view in right pane
  Loading      — async spinner while a remote/dir source loads
  Error(msg)   — transient error overlay
```

The `App` struct holds:
- `sources: Vec<Box<dyn ManifestSource>>`
- `loaded: HashMap<usize, Vec<DisplayNode>>` — cached after first load
- `selected_left: usize` — file list cursor
- `compare_selection: Option<usize>` — second file for diff mode
- `filter: FieldFilter`
- `matcher: Matcher`
- `state: AppState`
- `tokio_rt: tokio::runtime::Handle` — for spawning background loads

---

### `ui/` — rendering

All widgets are pure functions `fn draw(frame: &mut Frame, area: Rect, state: &App)`.
No widget holds mutable state — all state lives in `App`.

**Layout** (80/20 vertical split, configurable):
```
┌─────────────────────────────────────────────────────────┐
│ ◀ Files (25%)          │ Detail / Compare (75%) ▶       │
│                        │                                 │
│  [✓] image.jpg         │  ▾ Claim                       │
│  [✓] video.mp4         │    title: "My Photo"           │
│  [ ] remote/asset      │    format: image/jpeg           │
│  …                     │  ▾ Assertions (3)              │
│                        │    ▸ c2pa.actions               │
│                        │    ▾ c2pa.hash.data             │
│                        │      alg: sha256                │
│                        │      hash: a3f…                 │
├────────────────────────────────────────────────────────-─┤
│ /search  f:filter  c:compare  r:reload  ?:help  q:quit   │
└─────────────────────────────────────────────────────────┘
```

**Compare layout** replaces right pane with two equal columns + diff highlights.

---

### `main.rs` — CLI surface

```
c2pa-tui [OPTIONS] [PATHS_OR_URLS...]

Arguments:
  [PATHS_OR_URLS]   Files, directories, or HTTP URLs to load on startup

Options:
  --auth <SPEC>     Auth spec: none|basic:u:p|bearer:<tok>|digest:u:p
  --filter <GLOB>   Initial field filter (e.g. "assertions.*")
  --no-mouse        Disable mouse support
  --theme <NAME>    Color theme: dark (default) | light | mono
  -h, --help
  -V, --version
```

---

## Key interactions (keyboard + mouse)

| Key | Action |
|---|---|
| `↑/↓` or scroll | move file list cursor |
| `Enter` / click | load/select file, expand tree node |
| `Tab` | switch focus left ↔ right pane |
| `/` | open search bar (fuzzy within current manifest) |
| `f` | open filter bar (field path glob) |
| `c` | mark file for compare; second `c` opens diff view |
| `r` | reload selected source (remote re-fetch) |
| `Space` | collapse/expand tree node |
| `Esc` | close overlay / cancel compare |
| `q` | quit |
| Mouse click | focus pane, select item, expand node |
| Mouse scroll | scroll within focused pane |

---

## Public API surface

Since this is a binary, the library surface is intentionally small.
`lib.rs` exposes only what integration tests need:

```rust
pub mod app;           // App, AppState
pub mod error;         // AppError, Result
pub mod manifest;      // ManifestSource, DisplayNode, FieldFilter
pub mod remote;        // RemoteClient, Auth
pub mod search;        // Matcher, MatchResult
pub mod compare;       // ManifestDiff, FieldDiff, diff()
```

The `ui` module is **not** re-exported (terminal-only, not embeddable).

---

## Error handling strategy

- `AppError` is the single error type throughout; `Result<T> = Result<T, AppError>`.
- Async load errors are surfaced as `AppState::Error(msg)` — the TUI shows an
  overlay and the user can dismiss without crashing.
- `c2pa::Error` variants that indicate no manifest present (`NoManifest`) are
  shown as an informational node in the tree, not as a fatal error.
- All `?` propagation in `main.rs` is caught and printed to stderr before
  restoring the terminal, ensuring the terminal is never left in raw mode on crash.

---

## Test strategy

### Unit tests (in-module `#[cfg(test)]`)

Write tests **before** implementation in each module (TDD). Use `mockall` to mock
`ManifestSource` in tests rather than hand-rolling fake implementations.

| Module | What to test |
|---|---|
| `manifest/tree.rs` | `ManifestStore → DisplayNode` conversion for each top-level section |
| `manifest/filter.rs` | glob include/exclude correctly prunes nodes |
| `search/matcher.rs` | fuzzy match returns correct indices and highlight ranges |
| `compare/diff.rs` | `Equal`, `Changed`, `OnlyLeft`, `OnlyRight` cases |
| `remote/auth.rs` | `Auth::from_str` parses all variants; `apply` sets correct headers |
| `error.rs` | `#[from]` conversions compile and downcast correctly |

Property-based tests with `proptest` are required for all data-transformation modules
(`filter`, `diff`, `search`): see individual spec files for exact strategies.

### Integration tests (`tests/`)

| Test file | What to test |
|---|---|
| `integration_loader.rs` | Load fixture JPEG/PNG/PDF → assert tree has expected assertion keys |
| `integration_loader.rs` | Load a directory → assert correct number of sources discovered |
| `integration_remote.rs` | Start `wiremock` server serving a fixture asset → `RemoteSource::load()` returns valid tree |
| `integration_remote.rs` | `wiremock` returns 401 → `AppError::Auth` is returned |
| `integration_remote.rs` | `wiremock` returns 404 → `AppError::NoManifest` is returned |

### Snapshot tests (`tests/snapshot_ui.rs`)

Use `insta` with `ratatui::backend::TestBackend` to render widgets into a fixed-
size buffer and snapshot the resulting string. Cover:

- File list with one item selected
- Detail tree fully expanded
- Detail tree with filter applied (some nodes hidden)
- Search bar overlay with matches highlighted
- Compare view with a changed field highlighted
- Error overlay

---

## Code quality requirements

These apply to every module across all specs:

- **No `unwrap()` in production code.** Use `?`, `.context(...)`, or map to `AppError`.
  `.expect()` is allowed only in test code and `main()` startup before the TUI is active.
- **No `panic!` for normal error flow.** Errors propagate via `Result`.
- **`cargo fmt`** — all code must be formatted before a spec is considered done.
- **`cargo clippy -- -D warnings`** — zero clippy warnings required.
- **Rustdoc on all public items** — every `pub` fn, struct, enum, and trait method
  must have a `///` doc comment. Trait methods need at minimum a one-line description.
- **Structured logging** — use `tracing::instrument` on every `async fn` in the
  public API surface. Use `tracing::debug!` for normal operations, `tracing::warn!`
  for recoverable errors, `tracing::error!` for unexpected failures.
- **Iterators over manual loops** — prefer iterator adaptors (`map`, `filter`,
  `fold`) over `for` loops that build collections.

## Implementation order

1. **Scaffolding** — workspace `Cargo.toml`, `src/main.rs`, `src/error.rs`, `src/config.rs`
2. **Manifest loading** — `ManifestSource` trait + `FileSource` + `DirSource`; unit tests
3. **Tree conversion** — `DisplayNode` builder from `ManifestStore`; unit tests
4. **Remote loading** — `RemoteClient`, `Auth`, `RemoteSource`; wiremock tests
5. **TUI skeleton** — ratatui loop, crossterm setup/teardown, `App` state machine
6. **File list pane** — left pane with keyboard nav and mouse click
7. **Detail pane** — `tui-tree-widget` rendering of `DisplayNode` tree
8. **Search** — search bar + `Matcher` integration; highlight rendering
9. **Filter** — filter bar + `FieldFilter` application on tree
10. **Compare** — diff engine + side-by-side compare view
11. **Status bar + keybinds** — polish, help overlay
12. **Snapshot tests** — insta snapshots for all major views
13. **CLI polish** — `--theme`, `--no-mouse`, startup path args
