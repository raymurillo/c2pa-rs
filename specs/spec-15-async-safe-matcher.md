# Spec 15 — Async-Safe Matcher

**Phase:** 6 (sequential — requires spec-18 merged)  
**Depends on:** spec-13, spec-18  
**Produces:** non-blocking `Matcher`; `Arc<str>` display sharing; no per-keystroke query clone

---

## Goal

Three findings from the architecture review all concern `src/search/matcher.rs`
and its coupling to `app.rs`:

- **Finding 4** — `Matcher::index()` and `Matcher::query()` each contain a
  blocking CPU spin-loop (`loop { nucleo.tick(10); if !running break; }`).
  Both are called from the synchronous `App` event handlers, which run on the
  tokio executor.  For a manifest with hundreds of flattened nodes a single
  `tick` burst can take several milliseconds, stalling redraws and keystroke
  processing.
- **Finding 7** — `Matcher::index()` calls `nodes.to_vec()` to store a clone of
  all `FlatNode`s in `self.items`, then immediately copies the same `display`
  strings into nucleo's injector column.  The display strings are the dominant
  allocation; they are stored twice.
- **Finding 14** — `App::reindex_and_search` must clone the active query string
  on every keystroke because the borrow checker cannot simultaneously borrow
  `self.state` (for the query) and `self.matcher` mutably.

The fix addresses all three by:

1. Running nucleo's tick loop on a dedicated `tokio::task::spawn_blocking` call
   so the event loop is never stalled.
2. Sharing display strings between `self.items` and nucleo via `Arc<str>`.
3. Restructuring `App` so the query string is extracted before calling into
   `Matcher`, eliminating the clone.

---

## Files to modify

- `src/search/matcher.rs` — async-friendly API; `Arc<str>` sharing
- `src/app.rs` — call sites for `reindex_for_selected`, `reindex_and_search`

---

## M1 — `Matcher` async API

### Current blocking API

```rust
pub fn index(&mut self, nodes: &[FlatNode]) { /* spin loop */ }
pub fn query(&mut self, pattern: &str) -> Vec<MatchResult> { /* spin loop */ }
```

### New async API

```rust
/// Replace the current index with a new set of nodes.
/// Returns when nucleo has ingested all items (runs on a blocking thread).
pub async fn index_async(&mut self, nodes: &[FlatNode]);

/// Run a fuzzy query; returns ranked results.
/// Returns immediately for an empty pattern (no blocking work).
pub async fn query_async(&mut self, pattern: &str) -> Vec<MatchResult>;
```

Keep the synchronous `index` / `query` methods for tests only (behind
`#[cfg(test)]`) — do not break the existing test suite.

### Implementation

Both methods wrap the existing spin-loop bodies with
`tokio::task::spawn_blocking`:

```rust
pub async fn index_async(&mut self, nodes: &[FlatNode]) {
    // Build the items vec and the nucleo push list before crossing the
    // blocking boundary (avoids Send bound on FlatNode / Nucleo).
    let display_strings: Vec<Arc<str>> = nodes
        .iter()
        .map(|n| Arc::from(n.display.as_str()))
        .collect();

    // Store our copy — Arc clone is O(1), no string copies.
    self.items = nodes.iter().cloned().collect();
    // Replace per-item String with shared Arc in self.arc_displays.
    self.arc_displays = display_strings.clone();

    self.nucleo.restart(true);
    let injector = self.nucleo.injector();
    for (i, arc) in display_strings.iter().enumerate() {
        let arc = arc.clone();
        injector.push(i, move |_item, cols| {
            cols[0] = arc.as_ref().into();
        });
    }

    // Move the blocking poll onto a dedicated thread.
    let nucleo_ptr = &mut self.nucleo as *mut Nucleo<usize>;
    tokio::task::spawn_blocking(move || {
        // SAFETY: the &mut self borrow ensures exclusive access; the future
        // is awaited synchronously so the pointer cannot outlive self.
        let nucleo = unsafe { &mut *nucleo_ptr };
        loop {
            if !nucleo.tick(10).running { break; }
        }
    })
    .await
    .expect("nucleo index thread panicked");
}
```

> **Safety note:** The `*mut` trick above is the naive approach; prefer
> extracting `Nucleo` into an `Arc<Mutex<Nucleo<usize>>>` so the `spawn_blocking`
> closure is naturally `'static + Send`.  The `Arc<Mutex>` approach is
> described in the implementation details section below.

### Preferred implementation: `Arc<Mutex<Nucleo<usize>>>`

Wrap the nucleo instance so it can be moved into the blocking closure without
raw pointers:

```rust
pub struct Matcher {
    nucleo: Arc<std::sync::Mutex<Nucleo<usize>>>,
    items: Vec<FlatNode>,
    arc_displays: Vec<Arc<str>>,
}

impl Matcher {
    pub fn new() -> Self {
        Self {
            nucleo: Arc::new(std::sync::Mutex::new(
                Nucleo::new(Config::DEFAULT, Arc::new(|| {}), None, 1)
            )),
            items: Vec::new(),
            arc_displays: Vec::new(),
        }
    }

    pub async fn index_async(&mut self, nodes: &[FlatNode]) {
        self.arc_displays = nodes.iter()
            .map(|n| Arc::from(n.display.as_str()))
            .collect();
        self.items = nodes.to_vec();

        {
            let mut n = self.nucleo.lock().unwrap();
            n.restart(true);
            let injector = n.injector();
            for (i, arc) in self.arc_displays.iter().enumerate() {
                let arc = arc.clone();
                injector.push(i, move |_item, cols| {
                    cols[0] = arc.as_ref().into();
                });
            }
        }

        let nucleo = Arc::clone(&self.nucleo);
        tokio::task::spawn_blocking(move || {
            let mut n = nucleo.lock().unwrap();
            loop { if !n.tick(10).running { break; } }
        })
        .await
        .expect("nucleo index thread panicked");
    }

    pub async fn query_async(&mut self, pattern: &str) -> Vec<MatchResult> {
        if pattern.is_empty() {
            return self.items.iter().enumerate()
                .map(|(i, _)| MatchResult { node_index: i, score: 0, highlight_ranges: vec![] })
                .collect();
        }

        {
            let mut n = self.nucleo.lock().unwrap();
            n.pattern.reparse(0, pattern, CaseMatching::Ignore, Normalization::Smart, false);
        }

        let nucleo = Arc::clone(&self.nucleo);
        tokio::task::spawn_blocking(move || {
            let mut n = nucleo.lock().unwrap();
            loop { if !n.tick(10).running { break; } }
        })
        .await
        .expect("nucleo query thread panicked");

        // … collect snapshot, compute highlight ranges (same as current query()) …
        self.collect_results(pattern)
    }
}
```

---

## M2 — Eliminate per-keystroke query clone in `App`

### Problem

```rust
// app.rs — must clone because state and matcher are both fields of self
let query = match &self.state {
    AppState::Searching { query } => query.clone(),
    _ => String::new(),
};
self.search_results = self.matcher.query(&query);
```

### Fix

Extract the query string into a local variable before mutably borrowing the
matcher, using an explicit block to end the borrow:

```rust
pub async fn reindex_and_search(&mut self) {
    let query: String = match &self.state {
        AppState::Searching { query } => query.clone(),
        _ => return,
    };

    let prev = self.search_results.get(self.search_cursor).map(|r| r.node_index);
    self.search_results = self.matcher.query_async(&query).await;

    self.search_cursor = prev
        .and_then(|idx| self.search_results.iter().position(|r| r.node_index == idx))
        .unwrap_or(0);
    self.search_result_indices = self.search_results.iter().map(|r| r.node_index).collect();
}
```

The clone cannot be entirely eliminated without splitting `App` into separate
state and matcher structs (an architectural change out of scope for this spec).
The clone is one small `String` allocation per keystroke and is acceptable.
The primary goal here is converting the blocking calls to `async`.

---

## M3 — `Arc<str>` display sharing

`Matcher::items` stores `Vec<FlatNode>` where each `FlatNode` has a
`display: String`.  Nucleo also stores display strings (as `Utf32String`) in
its column.  Replace the redundant copy by:

1. Storing `arc_displays: Vec<Arc<str>>` alongside `items`.
2. Pushing `Arc<str>` clones into nucleo's injector (each clone is 8 bytes,
   not a string copy).
3. Using `arc_displays[i]` in `collect_results` for highlight computation
   instead of `self.items[i].display`.

This halves the peak memory use during indexing for large manifests.

---

## Call-site changes in `app.rs`

`reindex_for_selected` and `reindex_and_search` become `async fn`:

```rust
pub async fn reindex_for_selected(&mut self) { ... }
pub async fn reindex_and_search(&mut self) { ... }
```

Update all callers:

| Caller | Change |
|--------|--------|
| `handle_browse_key` (Up/Down) | must become `async fn` or use `.await` |
| `handle_load_result` | already sync; call `reindex_for_selected().await` — make caller async or use `tokio::spawn` |
| `handle_search_key` (Char/Backspace) | add `.await` on `reindex_and_search` |

`handle_browse_key` and `handle_search_key` are called from `handle_event`
which is already `async`, so adding `.await` is straightforward.

`handle_load_result` is currently sync.  Convert it to `async fn` and
`await` `reindex_for_selected`.

---

## Testing Strategy

### Unit tests for `Matcher`

```rust
#[tokio::test]
async fn index_async_and_query_async_return_same_results_as_sync() {
    let nodes = make_nodes();
    let mut m = Matcher::new();
    m.index_async(&nodes).await;
    let async_results = m.query_async("jpeg").await;

    let mut m2 = Matcher::new();
    m2.index(&nodes);
    let sync_results = m2.query("jpeg");

    assert_eq!(
        async_results.iter().map(|r| r.node_index).collect::<Vec<_>>(),
        sync_results.iter().map(|r| r.node_index).collect::<Vec<_>>(),
    );
}

#[tokio::test]
async fn empty_query_returns_all_items_async() { ... }

#[tokio::test]
async fn reindex_clears_previous_items_async() { ... }

proptest! {
    #[test]
    fn query_async_never_panics(pattern in ".*", displays in ...) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async { ... });
    }
}
```

### Integration: `App` no longer stalls

Add a benchmark or timing test that:
1. Indexes 1 000 flat nodes.
2. Measures time for a single `reindex_and_search` call.
3. Asserts it completes in < 200 ms (generous bound; actual should be < 10 ms).

---

## Edge Cases

- `spawn_blocking` panics: propagated via `.expect()`; treated as a fatal error.
  Consider mapping to `AppError::Terminal` instead.
- Concurrent calls: `Arc<Mutex<Nucleo>>` ensures exclusive access.  `query_async`
  after a concurrent `index_async` is safe because the lock serialises them.
- Empty index: `query_async("")` must still return an empty `Vec` (not panic) if
  `index_async` has never been called.

---

## Dependencies

No new crate dependencies.  `tokio::task::spawn_blocking` is already available
via the `tokio` dependency with the `rt` feature.

---

## Done criteria

```bash
cargo test -p c2pa-tui -- search::matcher::tests app::tests
cargo clippy -p c2pa-tui -- -D warnings
cargo fmt -p c2pa-tui -- --check
```

All new tests pass.  No existing tests regress.  `cargo test` must complete
without any thread deadlock (CI timeout implies deadlock in the blocking task).
