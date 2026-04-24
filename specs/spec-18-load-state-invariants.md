# Spec 18 — LoadState Invariants

**Phase:** 5 (parallel — requires spec-13 merged and `cargo build` clean)  
**Depends on:** spec-13  
**Produces:** `LoadState::Failed`; computed `loading_count()`; eager `ext_to_mime` check in `RemoteSource`

---

## Goal

Three findings that all relate to the correctness of per-source loading state:

- **Finding 10** — `LoadState` has no `Failed` variant.  When loading fails the
  entry is removed from `loaded`, making the source appear as "not yet
  requested" after the error overlay is dismissed.  The user receives no
  persistent indication that the source failed, and the next navigation will
  trigger a silent re-load attempt.
- **Finding 11** — `loading_count` is a manually maintained counter incremented
  in `trigger_load` and decremented with `saturating_sub` in
  `handle_load_result`.  A double-delivery on the channel (or any future
  refactoring mistake) silently corrupts the count.  The count is derivable from
  `loaded` and should not be stored separately.
- **Finding 17** — `RemoteSource::load` sanitises the file extension from the
  URL (alphanumeric ≤ 10 chars) but does not validate it against `ext_to_mime`.
  A URL ending in `.exe` creates a temporary file before `FileSource::load`
  rejects it.  The check should happen earlier.

---

## Files to modify

- `src/app.rs` — `LoadState::Failed`; remove `loading_count` field; add `loading_count()` method
- `src/manifest/loader.rs` — `RemoteSource::load` eager extension check
- `src/ui/file_list.rs` — render `LoadState::Failed` with a `[!]` icon
- `src/ui/status_bar.rs` — use `App::loading_count()` method

---

## L1 — `LoadState::Failed` variant

### New enum

```rust
/// Per-source loading state stored in `App::loaded`.
#[derive(Debug)]
pub enum LoadState {
    /// A background task is in flight for this index.
    Loading,
    /// Load completed successfully.
    Loaded(Vec<DisplayNode>),
    /// Load failed; the message is already formatted for display.
    Failed(String),
}
```

### `handle_load_result` update

```rust
fn handle_load_result(&mut self, idx: usize, result: Result<Vec<DisplayNode>>) {
    match result {
        Ok(nodes) => {
            self.loaded.insert(idx, LoadState::Loaded(nodes));
            if idx == self.selected_left {
                self.detail_tree_state = TreeState::default();
                self.reindex_for_selected();
            }
            if idx == self.selected_left || Some(idx) == self.compare_selection {
                self.compare_diff_cache = None;
            }
        }
        Err(e) => {
            // Persist the error state so the file list shows [!] after dismissal.
            self.loaded.insert(idx, LoadState::Failed(e.to_string()));
            self.state = AppState::Error {
                message: e.to_string(),   // raw message per spec-17 D4
            };
        }
    }
}
```

### `trigger_load` guard update

`trigger_load` currently skips sources that are `Loaded` (unless `force` is
true) and sources that are `Loading`.  It must also skip `Failed` sources
unless `force` is true (the reload keybinding `r` should be able to retry a
failed load):

```rust
fn trigger_load(&mut self, idx: usize, force: bool, ...) {
    match self.loaded.get(&idx) {
        Some(LoadState::Loaded(_)) if !force => return,
        Some(LoadState::Loading)             => return,
        Some(LoadState::Failed(_)) if !force => return,  // NEW: skip failed unless forced
        _ => {}
    }
    // ... rest unchanged
}
```

### File list renderer update (`ui/file_list.rs`)

Add a `[!]` icon for the `Failed` state alongside the existing icons:

```rust
let icon = match app.loaded.get(&i) {
    Some(LoadState::Loading)   => "[~]",
    Some(LoadState::Loaded(_)) => "[✓]",
    Some(LoadState::Failed(_)) => "[!]",  // NEW
    None                       => "[ ]",
};
```

Optionally colour the `[!]` icon with `theme.diff_only_left()` (red) to make
failures visually prominent.

### Requirements

- After a load failure the file list shows `[!]` beside the failed source.
- Pressing `r` (force reload) on a `Failed` source triggers a new load attempt.
- Pressing `Enter` on a `Failed` source does **not** re-trigger (consistent with
  `Loaded` — `Enter` only loads unloaded sources).
- The error overlay still appears immediately on failure.
- After the error overlay is dismissed the `[!]` icon remains.

---

## L2 — Computed `loading_count()`

### Remove the stored field

Delete `loading_count: usize` from the `App` struct and its initialisation in
`App::new`.

### Add a computed method

```rust
impl App {
    /// Number of sources currently being loaded in background tasks.
    ///
    /// Computed from `loaded` rather than maintained as a separate counter to
    /// prevent the two from drifting out of sync.
    pub fn loading_count(&self) -> usize {
        self.loaded
            .values()
            .filter(|s| matches!(s, LoadState::Loading))
            .count()
    }
}
```

### Remove manual increment/decrement

- Delete `self.loading_count += 1;` from `trigger_load`.
- Delete `self.loading_count = self.loading_count.saturating_sub(1);` from
  `handle_load_result`.

### Update all read sites

All places that read `self.loading_count` (e.g. `ui/status_bar.rs`) must call
`app.loading_count()` instead.

Search the codebase for `loading_count` and update every occurrence:

```bash
rg "loading_count" src/
```

Expected sites: `app.rs` (field decl, init, increment, decrement),
`ui/status_bar.rs` (read in spinner logic).

### Requirements

- `loading_count()` returns 0 when no sources are `Loading`.
- `loading_count()` returns the exact number of `LoadState::Loading` entries.
- The spinner in the status bar continues to show while any source is loading.
- No `loading_count` field remains on `App`.

---

## L3 — Eager `ext_to_mime` check in `RemoteSource::load`

### Current

```rust
// Sanitise extension: only allow alphanumeric, max 10 chars.
let ext = if raw_ext.len() <= 10 && raw_ext.chars().all(|c| c.is_ascii_alphanumeric()) {
    raw_ext
} else {
    "bin"
};
let mut tmp = tempfile::Builder::new()
    .suffix(&format!(".{ext}"))
    .tempfile()?;
// ... writes bytes, creates FileSource, calls load() which then rejects
```

A URL like `https://example.com/payload.exe` passes the alphanumeric check,
creates a `.exe` temp file, writes to it, and only fails when `FileSource::load`
reaches the `ext_to_mime` check.

### Fix

Validate the extension against `ext_to_mime` before creating the temp file:

```rust
let raw_ext = self.url.path_segments()
    .and_then(|mut segs| segs.next_back())
    .and_then(|seg| seg.rsplit('.').next())
    .unwrap_or("bin");

let ext = if raw_ext.len() <= 10 && raw_ext.chars().all(|c| c.is_ascii_alphanumeric()) {
    raw_ext
} else {
    "bin"
};

// Validate before allocating the temp file.
if ext_to_mime(ext).is_none() {
    return Err(AppError::UnsupportedFormat(ext.to_owned()));
}

let mut tmp = tempfile::Builder::new()
    .suffix(&format!(".{ext}"))
    .tempfile()?;
```

> Note: `ext_to_mime` is a module-private function in `loader.rs`; it is
> already accessible here without any visibility change.

### Requirements

- URLs with unsupported extensions return `AppError::UnsupportedFormat` before
  any network request has been fully buffered to disk.
  (The `bytes` were already fetched; this avoids the temp file creation.)
- URLs with supported extensions behave identically to the current
  implementation.
- URLs where the extension is sanitised to `"bin"` proceed to `FileSource::load`
  which will fail with `UnsupportedFormat("bin")` — this path is unchanged.

---

## Testing Strategy

### `app.rs`

```rust
#[test]
fn failed_load_persists_in_loaded_map() {
    let config = Config::default();
    let mut app = App::new(config).unwrap();
    app.handle_load_result(0, Err(AppError::Terminal("disk full".into())));
    assert!(matches!(app.loaded.get(&0), Some(LoadState::Failed(_))));
}

#[test]
fn trigger_load_skips_failed_source_without_force() {
    // set up app with a Failed source at index 0
    // trigger_load(0, false, ...) → should not insert Loading
    // trigger_load(0, true, ...)  → should insert Loading (retry)
}

#[test]
fn loading_count_reflects_loading_entries() {
    let config = Config::default();
    let mut app = App::new(config).unwrap();
    assert_eq!(app.loading_count(), 0);
    app.loaded.insert(0, LoadState::Loading);
    assert_eq!(app.loading_count(), 1);
    app.loaded.insert(1, LoadState::Loading);
    assert_eq!(app.loading_count(), 2);
    app.loaded.insert(0, LoadState::Loaded(vec![]));
    assert_eq!(app.loading_count(), 1);
}

#[test]
fn loading_count_ignores_failed_entries() {
    let config = Config::default();
    let mut app = App::new(config).unwrap();
    app.loaded.insert(0, LoadState::Failed("oops".into()));
    assert_eq!(app.loading_count(), 0);
}
```

### `manifest/loader.rs`

```rust
#[tokio::test]
async fn remote_source_rejects_unsupported_ext_before_creating_tempfile() {
    // Build a RemoteSource with a URL ending in .exe
    // Mock the fetch response to return bytes
    // Verify AppError::UnsupportedFormat("exe") is returned
    // Verify no tempfile was left in /tmp (check count before/after via tempfile crate)
    let url = Url::parse("https://example.com/malware.exe").unwrap();
    let src = RemoteSource::new(url, Auth::None);
    let client = RemoteClient::default();
    // We can't easily intercept the network call; test via MockManifestSource
    // or by setting up a wiremock that returns bytes and asserting the error.
    let result = src.load(&client).await;
    assert!(matches!(result, Err(AppError::UnsupportedFormat(ref ext)) if ext == "exe"));
}
```

### `ui/file_list.rs`

Add a snapshot or rendering test that verifies `[!]` appears for a `Failed`
source and `[~]` for `Loading`.

---

## Edge Cases

- Source 0 fails while source 1 is loading: `loading_count()` returns 1, status
  bar shows spinner, file list shows `[!]` for source 0 and `[~]` for source 1.
- Force-reload of a `Failed` source: `trigger_load(idx, true, ...)` replaces
  `Failed` with `Loading`; `loading_count()` increases by 1.
- `compare_diff_cache` invalidation on failure: when a failed load previously
  had `LoadState::Loaded` data, the diff cache might reference stale data.
  `handle_load_result` already invalidates the cache on every result (including
  errors); verify this still holds after the refactor.
- `with_loaded_for_tests` helper (added in spec-11): update its signature or
  an overload to accept `LoadState::Failed` so test helpers can seed failed
  state.

---

## Dependencies

No new crate dependencies.

---

## Done criteria

```bash
cargo test -p c2pa-tui -- app::tests manifest::loader::tests ui::file_list::tests
cargo clippy -p c2pa-tui -- -D warnings
cargo fmt -p c2pa-tui -- --check
```

All new tests pass.  No existing tests regress.  `grep -r "loading_count:" src/`
must return no field declarations (only the method call site and the method
definition).
