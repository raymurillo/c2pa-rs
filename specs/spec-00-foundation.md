# Spec 00 — Foundation

**Phase:** 0 (must be completed before all other specs)  
**Depends on:** nothing  
**Produces:** a compilable workspace skeleton that every Phase 1+ spec builds on

---

## Goal

Create the `c2pa-tui` workspace from scratch: Cargo manifest, error types, config,
and stub implementations of every public type and trait. All stubs must compile so
parallel sessions can add `use c2pa_tui::...` and write against real type signatures.

---

## Files to create

```
c2pa-tui/
├── Cargo.toml
├── src/
│   ├── main.rs
│   ├── lib.rs
│   ├── error.rs
│   ├── config.rs
│   ├── app.rs
│   ├── manifest/
│   │   ├── mod.rs
│   │   ├── loader.rs   ← stubs only
│   │   ├── tree.rs     ← stubs only
│   │   └── filter.rs   ← stubs only
│   ├── remote/
│   │   ├── mod.rs
│   │   ├── client.rs   ← stubs only
│   │   └── auth.rs     ← stubs only
│   ├── search/
│   │   ├── mod.rs
│   │   └── matcher.rs  ← stubs only
│   ├── compare/
│   │   ├── mod.rs
│   │   └── diff.rs     ← stubs only
│   └── ui/
│       ├── mod.rs      ← stubs only
│       ├── layout.rs   ← stubs only
│       ├── file_list.rs   ← stubs only
│       ├── detail.rs      ← stubs only
│       ├── compare.rs     ← stubs only
│       ├── search_bar.rs  ← stubs only
│       ├── filter_bar.rs  ← stubs only
│       └── status_bar.rs  ← stubs only
└── tests/
    └── fixtures/       ← copy sample C2PA-signed files here
```

---

## `Cargo.toml`

```toml
[package]
name = "c2pa-tui"
version = "0.1.0"
edition = "2021"
rust-version = "1.88.0"
description = "Terminal UI for browsing and comparing C2PA manifests"
license = "MIT OR Apache-2.0"

[[bin]]
name = "c2pa-tui"
path = "src/main.rs"

[lib]
name = "c2pa_tui"
path = "src/lib.rs"

[dependencies]
c2pa          = { version = "0.80", default-features = false, features = ["v1_api"] }
ratatui       = "0.29"
crossterm     = { version = "0.28", features = ["event-stream"] }
tokio         = { version = "1", features = ["full"] }
reqwest       = { version = "0.12", default-features = false, features = ["rustls-tls", "json"] }
clap          = { version = "4", features = ["derive"] }
thiserror     = "2"
serde_json    = "1"
nucleo        = "0.5"
tui-tree-widget = "0.22"
async-trait   = "0.1"
walkdir       = "2"
url           = "2"
glob          = "0.3"
tracing       = "0.1"
anyhow        = "1"

[dev-dependencies]
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
insta    = { version = "1", features = ["filters"] }
wiremock = "0.6"
mockall  = "0.13"
proptest = "1"
tempfile = "3"
tokio    = { version = "1", features = ["full"] }
```

> **Note:** Verify exact crate versions on crates.io before writing — pin to the
> latest compatible versions. For `c2pa`, check whether `v1_api` is the correct
> feature name in version 0.80 (look at `sdk/Cargo.toml` features in the main repo).

---

## Code quality requirements (applies to all specs)

- **No `unwrap()` in library/production code.** `.expect()` is allowed only in test
  code and pre-TUI startup in `main()`.
- **`cargo fmt`** before marking any spec done.
- **`cargo clippy -- -D warnings`** — zero warnings.
- **Rustdoc `///` on every `pub` item** — at minimum a one-line description.
- **`tracing::instrument`** on every `async fn` in the public API.
- **Iterators** over manual `for` loops that build `Vec`s.

## `src/error.rs` — complete, not a stub

```rust
/// All errors that can occur within c2pa-tui.
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

    #[error("directory walk error: {0}")]
    Walk(#[from] walkdir::Error),

    #[error("invalid URL: {0}")]
    Url(#[from] url::ParseError),

    #[error("invalid glob pattern: {0}")]
    Glob(#[from] glob::PatternError),
}

pub type Result<T> = std::result::Result<T, AppError>;
```

---

## `src/config.rs` — complete

```rust
#[derive(Debug, Clone)]
pub struct Config {
    pub theme: Theme,
    pub mouse_enabled: bool,
    pub left_pane_pct: u16,   // 1–99, default 25
    pub auth: crate::remote::Auth,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Theme {
    Dark,
    Light,
    Mono,
}
```

---

## `src/manifest/tree.rs` — type declarations + stub impls

```rust
use serde_json::Value;

/// A single node in the rendered manifest tree.
///
/// Leaf nodes have an empty `children` vec. Interior nodes (sections, arrays,
/// objects) carry their content in `children` and use `NodeValue::Missing` for
/// their own value.
#[derive(Debug, Clone)]
pub struct DisplayNode {
    pub key: String,
    pub value: NodeValue,
    pub children: Vec<DisplayNode>,
}

#[derive(Debug, Clone)]
pub enum NodeValue {
    Str(String),
    Json(Value),
    Bytes(usize),
    Missing,
}

/// Flat representation used by the search engine.
#[derive(Debug, Clone)]
pub struct FlatNode {
    pub path: String,   // dot-joined key path, e.g. "assertions.c2pa.actions.action"
    pub display: String,
    pub node_index: usize,
}

/// Convert a `ManifestStore` into a flat list of top-level `DisplayNode`s.
///
/// Each manifest in the store becomes one root node whose children are the
/// Claim, Assertions, Ingredients, and Validation sections.
/// Stub: returns empty vec. Implemented in spec-01.
pub fn store_to_nodes(store: &c2pa::ManifestStore) -> Vec<DisplayNode> {
    todo!("spec-01: implement store_to_nodes")
}

/// Flatten a DisplayNode tree to a Vec<FlatNode> for search indexing.
pub fn flatten(nodes: &[DisplayNode]) -> Vec<FlatNode> {
    let mut out = Vec::new();
    flatten_inner(nodes, "", &mut out);
    out
}

fn flatten_inner(nodes: &[DisplayNode], prefix: &str, out: &mut Vec<FlatNode>) {
    for node in nodes {
        let path = if prefix.is_empty() {
            node.key.clone()
        } else {
            format!("{}.{}", prefix, node.key)
        };
        let idx = out.len();
        out.push(FlatNode {
            path: path.clone(),
            display: format!("{}: {}", node.key, node.value.as_str()),
            node_index: idx,
        });
        flatten_inner(&node.children, &path, out);
    }
}

impl NodeValue {
    pub fn as_str(&self) -> String {
        match self {
            NodeValue::Str(s) => s.clone(),
            NodeValue::Json(v) => v.to_string(),
            NodeValue::Bytes(n) => format!("<{n} bytes>"),
            NodeValue::Missing => "<missing>".into(),
        }
    }
}
```

---

## `src/manifest/loader.rs` — trait + type stubs

```rust
use async_trait::async_trait;
use std::path::PathBuf;
use url::Url;
use crate::error::Result;
use crate::manifest::tree::DisplayNode;
use crate::remote::{RemoteClient, Auth};

/// Abstraction over all manifest origins: local files, directories, and remote URLs.
///
/// Implementors must be `Send + Sync` so they can be stored in `App` and loaded
/// from background tokio tasks.
#[async_trait]
#[mockall::automock]
pub trait ManifestSource: Send + Sync {
    /// Human-readable label shown in the file list pane.
    fn label(&self) -> &str;
    /// Load and parse the manifest, returning a `DisplayNode` tree.
    async fn load(&self, client: &RemoteClient) -> Result<Vec<DisplayNode>>;
    /// Returns `true` for sources that can be re-fetched (e.g. HTTP URLs).
    fn is_remote(&self) -> bool { false }
}

pub struct FileSource {
    pub path: PathBuf,
    label: String,
}

impl FileSource {
    pub fn new(path: PathBuf) -> Self {
        let label = path.display().to_string();
        Self { path, label }
    }
}

#[async_trait]
impl ManifestSource for FileSource {
    fn label(&self) -> &str { &self.label }
    async fn load(&self, _client: &RemoteClient) -> Result<Vec<DisplayNode>> {
        todo!("spec-01: implement FileSource::load")
    }
}

pub struct DirSource {
    pub path: PathBuf,
    label: String,
}

impl DirSource {
    pub fn new(path: PathBuf) -> Self {
        let label = path.display().to_string();
        Self { path, label }
    }
    /// Enumerate all supported files. Stub: returns empty vec. Implemented in spec-01.
    pub fn entries(&self) -> crate::error::Result<Vec<FileSource>> {
        todo!("spec-01: implement DirSource::entries")
    }
}

#[async_trait]
impl ManifestSource for DirSource {
    fn label(&self) -> &str { &self.label }
    async fn load(&self, client: &RemoteClient) -> Result<Vec<DisplayNode>> {
        todo!("spec-01: implement DirSource::load")
    }
}

pub struct RemoteSource {
    pub url: Url,
    pub auth: Auth,
    label: String,
}

impl RemoteSource {
    pub fn new(url: Url, auth: Auth) -> Self {
        let label = url.to_string();
        Self { url, auth, label }
    }
}

#[async_trait]
impl ManifestSource for RemoteSource {
    fn label(&self) -> &str { &self.label }
    fn is_remote(&self) -> bool { true }
    async fn load(&self, _client: &RemoteClient) -> Result<Vec<DisplayNode>> {
        todo!("spec-02: implement RemoteSource::load")
    }
}
```

---

## `src/manifest/filter.rs` — stubs

```rust
use glob::Pattern;
use crate::error::Result;
use crate::manifest::tree::DisplayNode;

#[derive(Debug, Clone, Default)]
pub struct FieldFilter {
    pub include_paths: Vec<Pattern>,
    pub exclude_paths: Vec<Pattern>,
}

impl FieldFilter {
    pub fn from_query(q: &str) -> Result<Self> {
        todo!("spec-01: implement FieldFilter::from_query")
    }
    pub fn apply(&self, nodes: Vec<DisplayNode>) -> Vec<DisplayNode> {
        todo!("spec-01: implement FieldFilter::apply")
    }
}
```

---

## `src/remote/auth.rs` — type declaration + stubs

```rust
use crate::error::Result;

/// HTTP authentication method to apply to remote manifest requests.
#[derive(Debug, Clone, Default)]
pub enum Auth {
    #[default]
    None,
    Basic { username: String, password: String },
    Bearer { token: String },
    Digest { username: String, password: String },
}

impl Auth {
    pub fn from_spec(s: &str) -> Result<Self> {
        todo!("spec-02: implement Auth::from_spec")
    }
    pub fn apply(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        todo!("spec-02: implement Auth::apply")
    }
}
```

---

## `src/remote/client.rs` — stub

```rust
use crate::error::Result;

#[derive(Debug, Clone)]
pub struct RemoteClient {
    inner: reqwest::Client,
}

impl RemoteClient {
    pub fn new() -> Result<Self> {
        todo!("spec-02: implement RemoteClient::new")
    }
    pub fn client(&self) -> &reqwest::Client {
        &self.inner
    }
}

impl Default for RemoteClient {
    fn default() -> Self {
        Self { inner: reqwest::Client::new() }
    }
}
```

---

## `src/search/matcher.rs` — type declarations + stubs

```rust
use std::ops::Range;
use crate::manifest::tree::FlatNode;

pub struct Matcher;

#[derive(Debug, Clone)]
pub struct MatchResult {
    pub node_index: usize,
    pub score: u32,
    pub highlight_ranges: Vec<Range<usize>>,
}

impl Matcher {
    pub fn new() -> Self { todo!("spec-03: implement Matcher::new") }
    pub fn index(&mut self, _nodes: &[FlatNode]) { todo!("spec-03") }
    pub fn query(&mut self, _pattern: &str) -> Vec<MatchResult> { todo!("spec-03") }
}
```

---

## `src/compare/diff.rs` — type declarations + stubs

```rust
use crate::manifest::tree::DisplayNode;

#[derive(Debug, Clone)]
pub struct ManifestDiff {
    pub left_label: String,
    pub right_label: String,
    pub fields: Vec<FieldDiff>,
}

#[derive(Debug, Clone)]
pub enum FieldDiff {
    Equal   { path: String, value: String },
    Changed { path: String, left: String, right: String },
    OnlyLeft  { path: String, value: String },
    OnlyRight { path: String, value: String },
}

pub fn diff(
    left_label: &str,
    left: &[DisplayNode],
    right_label: &str,
    right: &[DisplayNode],
) -> ManifestDiff {
    todo!("spec-04: implement diff()")
}
```

---

## `src/app.rs` — App struct + AppState stubs

```rust
use std::collections::HashMap;
use crate::manifest::loader::ManifestSource;
use crate::manifest::tree::DisplayNode;
use crate::manifest::filter::FieldFilter;
use crate::search::matcher::Matcher;
use crate::remote::client::RemoteClient;
use crate::config::Config;
use crate::error::Result;

#[derive(Debug, Clone, PartialEq)]
pub enum AppState {
    Browse,
    Searching { query: String },
    Filtering { query: String },
    Comparing,
    Loading { source_index: usize },
    Error { message: String },
}

pub struct App {
    pub sources: Vec<Box<dyn ManifestSource>>,
    pub loaded: HashMap<usize, Vec<DisplayNode>>,
    pub selected_left: usize,
    pub compare_selection: Option<usize>,
    pub filter: FieldFilter,
    pub matcher: Matcher,
    pub state: AppState,
    pub config: Config,
    pub client: RemoteClient,
}

impl App {
    pub fn new(config: Config) -> Result<Self> {
        todo!("spec-05: implement App::new")
    }
    pub async fn run(self) -> Result<()> {
        todo!("spec-05: implement App::run (TUI event loop)")
    }
    pub fn add_source(&mut self, source: Box<dyn ManifestSource>) {
        self.sources.push(source);
    }
}
```

---

## `src/ui/` stubs

All UI files should contain a single stub function that takes `&App` and returns `()`.

Example `src/ui/mod.rs`:
```rust
pub mod layout;
pub mod file_list;
pub mod detail;
pub mod compare;
pub mod search_bar;
pub mod filter_bar;
pub mod status_bar;

use ratatui::Frame;
use crate::app::App;

pub fn draw(_frame: &mut Frame, _app: &App) {
    todo!("spec-06: implement draw()")
}
```

For each sub-module, add a single stub draw function with a `todo!("spec-XX")` marker.

---

## `src/lib.rs`

```rust
pub mod app;
pub mod config;
pub mod error;
pub mod manifest;
pub mod remote;
pub mod search;
pub mod compare;
pub(crate) mod ui;
```

---

## `src/main.rs` — stub

```rust
fn main() {
    todo!("spec-09: implement main() with clap + App::run()")
}
```

---

## `src/manifest/mod.rs`

```rust
pub mod filter;
pub mod loader;
pub mod tree;
```

## `src/remote/mod.rs`

```rust
pub mod auth;
pub mod client;

pub use auth::Auth;
pub use client::RemoteClient;
```

## `src/search/mod.rs`

```rust
pub mod matcher;
pub use matcher::{Matcher, MatchResult};
```

## `src/compare/mod.rs`

```rust
pub mod diff;
pub use diff::{ManifestDiff, FieldDiff, diff};
```

---

## `tests/fixtures/`

Copy at least three signed test assets (different formats) into `tests/fixtures/`:
- A JPEG with a simple manifest (e.g. from the `c2pa-rs` test suite under `sdk/tests/fixtures/`)
- A PNG with a manifest
- A PDF or MP4 with a manifest (if available)

---

## Done criteria

```
cargo build                      # zero errors
cargo test                       # zero tests, but must compile cleanly
cargo fmt -- --check             # no formatting changes needed
cargo clippy -- -D warnings      # zero warnings
```

No `todo!()` panics should be reachable at compile time — that's fine for a stub;
the goal is a clean build so parallel sessions can branch from this commit.
