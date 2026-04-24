# Spec 20 — Error Reporter & Log Overlay

**Phase:** 7 (sequential — requires spec-19 merged)
**Depends on:** spec-17, spec-18, spec-19
**Produces:** `error::Reporter` with correlation IDs; F2 log overlay widget; structured logging of every user-visible error

---

## Goal

Errors in the TUI are currently visible for exactly one frame-cycle: the
`AppState::Error { message }` overlay is dismissed by any keypress and the raw
error is never written anywhere the user can retrieve it. There is no way to
correlate a UI error with a structured log event for bug triage.

Spec-17 moves `AppState::Error` to carry the raw `AppError` instead of a
pre-formatted display string. Spec-18 adds `LoadState::Failed`. This spec
builds on both to introduce a small reporter that:

1. Assigns each reported error a short **correlation ID** (8 hex chars).
2. Emits a structured `tracing::error!` with the ID, the error chain, and
   the classification.
3. Returns a `UserMessage` that the UI renders — including the ID so the user
   can reference it in a bug report.
4. Backs a new `F2` log overlay that shows the ring buffer from spec-19 and
   a "recent failures" list sourced from per-source `LoadState::Failed`.

After this spec, every `AppState::Error` and every `LoadState::Failed` has a
matching log entry, and the user can copy the correlation ID out of the
overlay or the F2 view.

---

## Files to modify

- `src/error.rs` — add `Reporter`, `CorrelationId`, `UserMessage`, `ErrorClass`
- `src/app.rs` — wire `Reporter` into `handle_load_result`, `handle_filter_key`;
  add `log_ring: LogRing`; extend `AppState::Error` shape (coordinate with spec-17)
- `src/ui/mod.rs` — render correlation ID in error overlay; dispatch F2 key
- `src/ui/log_overlay.rs` — new; two-section log overlay widget
- `Cargo.toml` — no new deps (use `std::time::SystemTime` + `Display` hex)

---

## R1 — `error::Reporter`

```rust
/// A short, non-cryptographic identifier stamped onto every user-visible
/// error so the user can reference it in bug reports.
///
/// 32 bits rendered as 8 lowercase hex chars. Generated from
/// `SystemTime::now().duration_since(UNIX_EPOCH).subsec_nanos()` XORed
/// with a per-process counter — collisions are acceptable, uniqueness
/// within a session is not guaranteed but is overwhelmingly likely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorrelationId(u32);

impl std::fmt::Display for CorrelationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:08x}", self.0)
    }
}

/// Classification used by the reporter to decide log level and whether to
/// auto-offer the diagnostic bundle (spec-22).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// User-correctable input (typo in filter glob, bad CLI arg).
    /// Logged at DEBUG. No bundle prompt.
    UserInput,
    /// External failure (network, disk full, HTTP 5xx). Logged at WARN.
    /// No bundle prompt unless repeated.
    Transient,
    /// Likely bug (c2pa SDK parse error on a valid file; panic recovery).
    /// Logged at ERROR. Auto-offer bundle.
    Internal,
}

/// UI-ready message + correlation ID.
#[derive(Debug, Clone)]
pub struct UserMessage {
    pub id: CorrelationId,
    pub class: ErrorClass,
    pub title: String,
    pub detail: String,
}

/// Stateless reporter. Constructed once, held by `App`.
pub struct Reporter;

impl Reporter {
    /// Report an error. Emits one structured tracing event and returns the
    /// `UserMessage` the UI should display.
    pub fn report(&self, err: &AppError) -> UserMessage { /* … */ }

    /// Classify an error without emitting anything. Used by spec-22 to
    /// decide whether to auto-offer the diag bundle.
    pub fn classify(err: &AppError) -> ErrorClass { /* … */ }
}
```

### Classification rules

| Variant | Class |
|---|---|
| `AppError::InvalidInput(_)` (spec-17) | `UserInput` |
| `AppError::Auth(_)` | `UserInput` |
| `AppError::Url(_)` | `UserInput` |
| `AppError::Glob(_)` | `UserInput` |
| `AppError::UnsupportedFormat(_)` | `UserInput` |
| `AppError::Http(e)` if `e.is_timeout()` or `e.is_connect()` | `Transient` |
| `AppError::Http(_)` otherwise | `Transient` |
| `AppError::Io(e)` if `e.kind() == NotFound` | `UserInput` |
| `AppError::Io(_)` otherwise | `Transient` |
| `AppError::Walk(_)` | `Transient` |
| `AppError::NoManifest(_)` | `UserInput` |
| `AppError::C2pa(_)` | `Internal` |
| `AppError::Terminal(_)` | `Internal` |

### Event shape

```rust
tracing::event!(
    level_for_class(class),
    correlation_id = %id,
    class = ?class,
    error = %err,            // Display
    error.chain = ?chain,    // iter of sources via std::error::Error::source()
    "{}", title,             // one-line summary
);
```

### Requirements

- `Reporter::report` is `&self` (no mutation). `Reporter` is `Send + Sync`.
- A single call emits exactly one tracing event.
- `UserMessage::title` never exceeds 80 chars. `detail` is unbounded (the
  overlay wraps it).
- No `unwrap`/`expect` on runtime paths. Time source failure falls back to a
  monotonic counter only.

---

## R2 — Integrate with `AppState::Error`

Spec-17 D4 changes `AppState::Error` to hold raw error data. This spec
further refines that to include the correlation ID:

```rust
pub enum AppState {
    // …
    Error {
        id: CorrelationId,
        class: ErrorClass,
        title: String,
        detail: String,
    },
}
```

### Call sites

`handle_load_result` (error branch):

```rust
Err(e) => {
    let msg = self.reporter.report(&e);
    self.loaded.insert(idx, LoadState::Failed {
        id: msg.id,
        detail: e.to_string(),  // spec-18 stores raw string
    });
    self.state = AppState::Error {
        id: msg.id,
        class: msg.class,
        title: msg.title,
        detail: msg.detail,
    };
}
```

`handle_filter_key` (invalid glob branch): identical pattern.

### Overlay rendering

The error overlay (`ui::draw_error_overlay`) appends `" [id: abcd1234]"` to the
title line. A footer row shows: `"Press any key to dismiss · F2 for logs · d for diag bundle"`
(the last part wired up in spec-22).

Spec-18 updates `LoadState::Failed` to a struct variant so both the file-list
icon tooltip and the F2 overlay can show the correlation ID.

### Requirements

- Every `AppState::Error` is preceded by exactly one `tracing::error!` (or
  `warn!` / `debug!` depending on class).
- The correlation ID shown in the UI matches the one in the log event.
- Dismissing the error overlay does not clear the `LoadState::Failed` entry.

---

## R3 — `F2` log overlay

New key binding: `F2` (any state except `Searching`/`Filtering` — typing
takes precedence in those states). Opens a full-screen popup.

### Layout

```
┌─ c2pa-tui log — 47 events, 0 dropped ─────────────────────┐
│ ▶ Failed sources (2)                                       │
│   [abcd1234] /path/to/a.jpg — No manifest present          │
│   [ef012345] https://example.com/b.jpg — connection reset  │
│                                                            │
│ ▼ Live log (tail)                                          │
│   14:22:03.123 INFO  c2pa_tui::app: starting event loop    │
│   14:22:04.891 WARN  reqwest: tls handshake slow (1.2s)    │
│   14:22:05.002 ERROR c2pa_tui: [abcd1234] NoManifest …     │
│   …                                                        │
│                                                            │
│ j/k scroll · g/G top/bottom · c clear ring · q close       │
└────────────────────────────────────────────────────────────┘
```

### Module: `src/ui/log_overlay.rs`

```rust
pub struct LogOverlayState {
    pub scroll: usize,
    pub failures_collapsed: bool,
}

pub fn draw(
    frame: &mut ratatui::Frame,
    area: ratatui::layout::Rect,
    ring: &LogRing,
    loaded: &HashMap<SourceId, LoadState>,
    state: &LogOverlayState,
    theme: &Theme,
) { /* … */ }
```

Events rendered from newest-first; ERROR/WARN coloured using the existing
theme. Correlation IDs are shown in mono style so they stand out.

### Key handling

New `AppState::LogOverlay` variant (or `show_log: bool` — pick `show_log` to
avoid a state-machine expansion; the overlay is purely additive). In
`handle_browse_key`:

```rust
KeyCode::F(2) => { self.show_log = !self.show_log; }
```

Inside the overlay: `j/k` scrolls, `g/G` jumps, `c` calls `self.log_ring.clear()`,
`q`/`F2` closes.

### Requirements

- The overlay never allocates more than the snapshot it is rendering from.
- Opening and closing the overlay is O(1) with respect to ring size.
- The "Failed sources" section is sorted by insertion order (stable).
- Correlation IDs in the overlay are selectable by mouse drag when the
  terminal supports it (ratatui raw mode passes drags through — no extra
  work needed; just don't consume `MouseEvent::Drag`).

---

## Testing strategy

### `error.rs`

```rust
#[test]
fn classify_maps_variants_correctly() {
    use AppError::*;
    assert_eq!(Reporter::classify(&InvalidInput("x".into())), ErrorClass::UserInput);
    // … one assertion per variant
}

#[test]
fn correlation_id_is_deterministic_hex() {
    let id = CorrelationId::from_raw(0xdeadbeef);
    assert_eq!(id.to_string(), "deadbeef");
}

#[test]
fn report_emits_exactly_one_event() {
    // use tracing_test::traced_test to capture events
    let r = Reporter;
    r.report(&AppError::NoManifest("foo".into()));
    // assert exactly one event with correlation_id field
}
```

### `ui/log_overlay.rs`

`insta` snapshot tests covering:
- Empty ring, no failures
- 3 live events, 2 failures
- Failures section collapsed
- Scrolled view

### Integration

`tests/integration_error_reporter.rs`:
- Feed an `AppError::Http` via `MockManifestSource`, assert a matching
  structured event appears in the ring and `LoadState::Failed` carries the
  same ID.

---

## Edge cases

- **Reporter called before `obs::init`**: log events silently discarded by the
  `tracing` no-op default. Reporter still returns a valid `UserMessage`.
- **Ring cleared while overlay is open**: overlay re-snapshots on every draw,
  so an empty ring renders the "0 events" header without panic.
- **Correlation ID collision within session**: acceptable. The class + title
  + timestamp are sufficient for disambiguation in logs.
- **F2 pressed while an error overlay is showing**: error overlay takes
  precedence; F2 is a no-op until the error is dismissed (so the user can
  actually see the correlation ID before diving into the log).

---

## Dependencies

No new crate dependencies.

---

## Done criteria

```bash
cargo test -p c2pa-tui -- error:: ui::log_overlay::
cargo test -p c2pa-tui --test integration_error_reporter
cargo clippy -p c2pa-tui -- -D warnings
cargo fmt -p c2pa-tui -- --check
```

- Every code path that sets `AppState::Error` goes through `Reporter::report`
  (grep audit: no direct `AppState::Error { … }` construction outside
  `error.rs` and one call site per handler).
- Snapshot tests pass for all log-overlay variants.
- Manual smoke: loading a non-existent file shows an overlay with a hex ID;
  pressing F2 after dismissal shows the same ID in the failures list and in
  the live log.
