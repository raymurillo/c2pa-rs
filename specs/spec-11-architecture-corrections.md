# Spec 11 — Architecture Corrections

**Phase:** 4 (sequential — requires spec-10 merged and `cargo build` clean)  
**Depends on:** spec-10  
**Produces:** stable `SourceId` keying; per-file dir expansion; async-safe dir
enumeration; `Auth::apply` returns `Result`; `App::with_loaded_for_tests` constructor.

---

## Goal

Correct four structural defects that were identified in the architecture review:

1. `HashMap<usize, …>` keying breaks when sources are removed or reordered.
2. `DirSource` presents a directory as a single monolithic source instead of
   expanding it into individual per-file entries.
3. Sync `walkdir` I/O blocks the tokio thread pool.
4. `Auth::apply()` silently falls back from Digest to Basic instead of returning
   an error the caller can surface through the `AppState::Error` overlay.

Items 1–3 touch `app.rs`, `manifest/loader.rs`, and `main.rs`. Item 4 must be in
this spec (not spec-10) because changing `Auth::apply`'s return type and updating
`RemoteClient::fetch` to call `apply()?` must happen in the same commit — an
intermediate state where `apply` returns `Result` but `fetch` still calls `apply()`
without `?` would not compile.

---

## Files to modify

- `src/app.rs` — introduce `SourceId`; update fields and handlers; add `App::add_dir()`;
  add `App::with_loaded_for_tests`
- `src/manifest/loader.rs` — add `DirSource::entries_async()`; remove `ManifestSource`
  impl for `DirSource`
- `src/main.rs` — use `App::add_dir()` for directory arguments
- `src/remote/auth.rs` — change `apply` to return `Result<RequestBuilder>`
- `src/remote/client.rs` — update `fetch` to call `auth.apply(builder)?`

---

## B1 — Introduce a stable `SourceId` key

### New type

Add to `src/app.rs` (above the `App` struct):

```rust
use std::sync::atomic::{AtomicU64, Ordering};

/// Stable identity for a manifest source.
///
/// Assigned at `add_source()` time from a monotonically increasing counter.
/// Unlike a `Vec` index, a `SourceId` remains valid if other sources are
/// removed or if the `sources` Vec is reordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceId(u64);

static NEXT_SOURCE_ID: AtomicU64 = AtomicU64::new(0);

impl SourceId {
    fn next() -> Self {
        Self(NEXT_SOURCE_ID.fetch_add(1, Ordering::Relaxed))
    }
}
```

### `App` field changes

```rust
// Before
pub sources: Vec<Arc<dyn ManifestSource>>,
pub loaded: HashMap<usize, LoadState>,
pub selected_left: usize,
pub compare_selection: Option<usize>,

// After
pub sources: Vec<(SourceId, Arc<dyn ManifestSource>)>,
pub loaded: HashMap<SourceId, LoadState>,
pub selected_left: Option<SourceId>,   // None when sources is empty
pub compare_selection: Option<SourceId>,
```

`selected_left` changes from `usize` to `Option<SourceId>` to eliminate
off-by-one panics when the source list is empty.

### Helper methods on `App`

```rust
impl App {
    /// Register a new source and return its stable `SourceId`.
    pub fn add_source(&mut self, src: Arc<dyn ManifestSource>) -> SourceId {
        let id = SourceId::next();
        self.sources.push((id, src));
        if self.selected_left.is_none() {
            self.selected_left = Some(id);
        }
        id
    }

    /// Return the `Arc<dyn ManifestSource>` for the given id, if present.
    pub fn source_by_id(&self, id: SourceId) -> Option<&Arc<dyn ManifestSource>> {
        self.sources.iter().find_map(|(sid, src)| (*sid == id).then_some(src))
    }

    /// Return the list-position index of the given `SourceId` within `sources`.
    pub fn index_of(&self, id: SourceId) -> Option<usize> {
        self.sources.iter().position(|(sid, _)| *sid == id)
    }

    /// Return the `SourceId` at the given list-position index.
    pub fn id_at(&self, idx: usize) -> Option<SourceId> {
        self.sources.get(idx).map(|(id, _)| *id)
    }
}
```

### Test-state injection constructor

Integration tests and snapshot tests (`tests/`) need to inject loaded manifest
nodes without going through the async load path.  Adding this now (before
spec-13 narrows field visibility) means tests never need direct field write access:

```rust
#[cfg(test)]
impl App {
    /// Construct an `App` pre-loaded with a single source and its manifest nodes.
    ///
    /// Intended exclusively for unit and snapshot tests; do not use in
    /// production code paths.
    pub fn with_loaded_for_tests(
        config: Config,
        label: &str,
        nodes: Vec<crate::manifest::tree::DisplayNode>,
    ) -> Self {
        use crate::manifest::loader::MockManifestSource;
        let mut app = App::new(config).unwrap();
        let mut mock = MockManifestSource::new();
        let label_str = label.to_string();
        mock.expect_label().return_const(label_str);
        mock.expect_is_remote().return_const(false);
        let id = app.add_source(Arc::new(mock));
        app.loaded.insert(id, LoadState::Loaded(nodes));
        app
    }
}
```

### Update all key-event handlers

Every place that previously used `self.selected_left` as a raw `usize` index must:
- Use `self.selected_left` (now `Option<SourceId>`) for identity comparisons.
- Use `self.index_of(id)` to compute display position.
- Use `self.id_at(idx)` when navigating by position (arrow keys).

File-list Up/Down navigation:

```rust
KeyCode::Down => {
    if let Some(current_id) = self.selected_left {
        if let Some(idx) = self.index_of(current_id) {
            let next = (idx + 1).min(self.sources.len().saturating_sub(1));
            self.selected_left = self.id_at(next);
        }
    } else if !self.sources.is_empty() {
        self.selected_left = self.id_at(0);
    }
}
```

Apply the same pattern for `compare_selection`.

Background load tasks capture `SourceId` rather than `usize`:

```rust
// Before
let idx = i;
tokio::spawn(async move {
    let result = src.load(&client).await;
    tx.send((idx, result)).unwrap();
});

// After
let id = sid;
tokio::spawn(async move {
    let result = src.load(&client).await;
    tx.send((id, result)).unwrap();
});
```

Channel type: `mpsc::Sender<(SourceId, Result<Vec<DisplayNode>>)>`.

---

## B2 — Expand `DirSource` into individual `FileSource` entries

### Problem

`DirSource` currently implements `ManifestSource` and loads every file in the
directory as one monolithic `Vec<DisplayNode>`.  This means:

- The whole directory appears as one row in the file list.
- Individual files cannot be selected, expanded, or reloaded independently.

### Before removing the `ManifestSource` impl — audit all call sites

Search for every location where `DirSource` is used as `dyn ManifestSource` or
`Box<dyn ManifestSource>`:

```sh
rg -n "DirSource" c2pa-tui/src/ c2pa-tui/tests/
```

The impl must only be removed after confirming no production or test code still
wraps a `DirSource` in `Arc<dyn ManifestSource>` directly.  As of spec-09 the
only production call site is `main.rs`, which spec-11 replaces with `App::add_dir`.

### Fix: `App::add_dir()`

Remove the `ManifestSource` impl on `DirSource`. Keep `DirSource` itself for
directory enumeration (its `entries()` and `entries_async()` methods remain).

Add to `src/app.rs`:

```rust
/// Expand a directory into individual `FileSource` entries and register each one.
///
/// This is the async variant that offloads blocking `walkdir` I/O to the tokio
/// blocking thread pool.  Returns the stable `SourceId`s assigned to each file.
pub async fn add_dir(
    &mut self,
    path: std::path::PathBuf,
) -> crate::error::Result<Vec<SourceId>> {
    let entries = crate::manifest::loader::DirSource::new(path)
        .entries_async()
        .await?;
    Ok(entries
        .into_iter()
        .map(|file_src| self.add_source(Arc::new(file_src)))
        .collect())
}
```

### Update `main.rs`

Replace the directory branch with a call to `add_dir` inside the existing
`rt.block_on(async { … })` block:

```rust
if path.is_dir() {
    if let Err(e) = app.add_dir(path.clone()).await {
        eprintln!("warning: could not read directory {:?}: {e}", path);
    }
} else {
    app.add_source(Arc::new(FileSource::new(path)));
}
```

---

## B3 — Move `walkdir` enumeration off the async runtime

`DirSource::entries()` calls `walkdir`, which performs blocking filesystem I/O.
When invoked from an async context this blocks the tokio thread pool.

Add an async wrapper in `src/manifest/loader.rs`:

```rust
impl DirSource {
    /// Async variant of [`DirSource::entries`] that offloads the blocking
    /// `walkdir` traversal to the tokio blocking thread pool.
    pub async fn entries_async(&self) -> crate::error::Result<Vec<FileSource>> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || DirSource::new(path).entries())
            .await
            .map_err(|e| {
                crate::error::AppError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                ))
            })?
    }
}
```

The sync `entries()` method remains for unit tests that don't need an async
runtime.

### Tests

```rust
#[tokio::test]
async fn entries_async_returns_same_results_as_sync() {
    let dir = DirSource::new("tests/fixtures/".into());
    let sync_entries = dir.entries().unwrap();
    let async_entries = dir.entries_async().await.unwrap();
    assert_eq!(
        sync_entries.iter().map(|e| &e.path).collect::<Vec<_>>(),
        async_entries.iter().map(|e| &e.path).collect::<Vec<_>>(),
    );
}

#[tokio::test]
async fn add_dir_creates_individual_sources() {
    let mut app = App::new(Config::default()).unwrap();
    let ids = app.add_dir("tests/fixtures/".into()).await.unwrap();
    assert!(ids.len() >= 2, "directory should expand to multiple sources");
    assert_eq!(app.sources.len(), ids.len());
    let id_set: std::collections::HashSet<_> = ids.iter().copied().collect();
    assert_eq!(id_set.len(), ids.len(), "each SourceId must be unique");
}
```

---

## A4 — `Auth::apply` returns `Result<RequestBuilder>`

`Auth::apply()` currently emits a `tracing::warn!` when `Digest` falls back to
Basic, which TUI users never see.

**Why this is here, not in spec-10:** changing `apply`'s return type and
updating its sole production caller (`RemoteClient::fetch`) must happen
atomically.  Splitting them across two specs would leave a non-compiling
intermediate state.

### Change `Auth::apply` in `src/remote/auth.rs`

```rust
/// Apply this auth method to a `reqwest::RequestBuilder`.
///
/// Returns `Err(AppError::Auth)` for the `Digest` variant because
/// `reqwest` does not implement Digest authentication natively.
/// Use `basic:user:pass` instead.
pub fn apply(
    &self,
    builder: reqwest::RequestBuilder,
) -> crate::error::Result<reqwest::RequestBuilder> {
    match self {
        Auth::None => Ok(builder),
        Auth::Basic { username, password } => Ok(builder.basic_auth(username, Some(password))),
        Auth::Bearer { token } => Ok(builder.bearer_auth(token)),
        Auth::Digest { .. } => Err(crate::error::AppError::Auth(
            "Digest auth is not supported by reqwest; use basic:user:pass instead".into(),
        )),
    }
}
```

### Update `RemoteClient::fetch` in `src/remote/client.rs`

```rust
// Before
let builder = auth.apply(builder);

// After
let builder = auth.apply(builder)?;
```

### Update existing tests

Any test that calls `auth.apply(builder)` must now handle the `Result`:

```rust
// Before
let builder = auth.apply(builder);

// After
let builder = auth.apply(builder).unwrap(); // in tests where Digest is not used
// or
let result = auth.apply(builder);           // in tests checking Digest behaviour
```

### New test for Digest rejection

```rust
#[test]
fn digest_apply_returns_error() {
    let auth = Auth::Digest {
        username: "u".into(),
        password: "p".into(),
    };
    let client = reqwest::Client::new();
    let builder = client.get("https://example.com");
    let result = auth.apply(builder);
    assert!(result.is_err());
    assert!(matches!(result.unwrap_err(), crate::error::AppError::Auth(_)));
}
```

---

## Done criteria

```
cargo build
cargo test
cargo fmt -- --check
cargo clippy -- -D warnings
```

Confirm that a directory argument produces one row per file in the TUI file list:

```sh
cargo run -- tests/fixtures/
```

Confirm that `--auth digest:user:pass` surfaces an error overlay rather than
silently downgrading:

```sh
cargo run -- --auth digest:user:pass https://example.invalid/
# Expected: AppState::Error overlay with "Digest auth is not supported"
```
