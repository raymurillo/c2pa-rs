# Spec 19 — Observability Core

**Phase:** 7 (sequential — requires Phase 6 merged: spec-15 and spec-17)
**Depends on:** spec-15, spec-17
**Produces:** `src/obs/` module with layered `tracing` subscriber (file · ring · stderr); credential redaction layer; single initialisation path

---

## Goal

Today the TUI initialises `tracing_subscriber::fmt()` twice — once in
[src/main.rs](../c2pa-tui/src/main.rs) and again in
[src/app.rs](../c2pa-tui/src/app.rs) (`App::run`) — both writing plain text to
stderr. During normal operation that stderr stream is mixed with the ratatui
alternate screen, and a single background `tracing::error!` from `reqwest` or
`c2pa` paints over the UI and corrupts the display until the next full redraw.

There is also no persistent log file. When a release user hits a bug there is
no artifact to attach to a report, and no `RUST_LOG` value they can set without
recompiling.

This spec replaces both `try_init` calls with a single `obs::init(&Config)`
that installs a layered subscriber:

1. **File layer** — rolling JSON to `$XDG_STATE_HOME/c2pa-tui/logs/` (or
   `~/.local/state/c2pa-tui/logs/` if `XDG_STATE_HOME` is unset; on Windows,
   `%LOCALAPPDATA%\c2pa-tui\logs\`). Rotated daily with a 7-file / 100 MB cap.
2. **Ring layer** — bounded in-memory `VecDeque<OwnedEvent>` (2 000 entries),
   readable via a new `App::log_ring` field. Drives the `F2` log overlay added
   in spec-20.
3. **Stderr layer** — **disabled** while the TUI owns stdout. Re-enabled by
   the panic hook and the normal exit path so final errors still surface to
   the user's shell.
4. **Redaction layer** — wraps all three, stripping credential-bearing fields
   and URL userinfo before the event reaches any downstream layer.

All four are part of one subscriber so an event is redacted exactly once no
matter how many layers log it.

---

## Files to modify

- `src/obs/mod.rs` — new module; `init`, `TelemetryGuard`, log directory helper
- `src/obs/redact.rs` — new; `Redactor` layer + field/URL scrubbing
- `src/obs/ring.rs` — new; `RingLayer`, `LogRing`, `OwnedEvent`
- `src/lib.rs` — add `pub mod obs;`
- `src/main.rs` — remove the direct `tracing_subscriber::fmt()` call; call
  `obs::init(&config)` immediately after CLI parsing, keep returned
  `TelemetryGuard` alive until process exit
- `src/app.rs` — delete the second `try_init` in `App::run` and the
  `PANIC_HOOK` block that ignores logging; add `log_ring: LogRing` field to
  `App` (wired up for spec-20)
- `src/config.rs` — add `log_level: tracing::Level`, `log_dir: Option<PathBuf>`
- `Cargo.toml` — add `tracing-appender = "0.2"`, `dirs = "5"`

---

## O1 — `obs::init` single entry point

```rust
/// Initialise the process-wide tracing subscriber.
///
/// Must be called exactly once, before any other thread emits a `tracing`
/// event. Returns a guard that must be kept alive for the lifetime of the
/// process — dropping it flushes the non-blocking file writer.
///
/// Idempotent: repeated calls return a no-op guard and log a `warn!`.
#[must_use = "dropping the guard flushes pending log writes"]
pub fn init(config: &Config) -> TelemetryGuard {
    // 1. Resolve log directory (XDG_STATE_HOME → ~/.local/state → temp_dir
    //    fallback). Create with mode 0o700 on Unix.
    // 2. Build tracing_appender::rolling::Builder (daily rotation, 7 files).
    // 3. Wrap in tracing_appender::non_blocking for async writes.
    // 4. Build RingLayer with capacity 2000.
    // 5. Build Redactor layer that wraps all three.
    // 6. Compose via tracing_subscriber::registry().with(…).init().
    // 7. Return TelemetryGuard { _writer_guard, log_ring }.
}

pub struct TelemetryGuard {
    _writer_guard: tracing_appender::non_blocking::WorkerGuard,
    /// Cheap clone; `App` clones this out and stores it.
    pub log_ring: LogRing,
}
```

### Requirements

- `obs::init` never panics. Failure to create the log directory downgrades to
  stderr-only and emits one `warn!` once the stderr layer is active.
- A process-wide `OnceLock<()>` guards against double-init (second call
  returns a no-op guard).
- Log level resolution precedence:
  1. `RUST_LOG` environment variable (via `EnvFilter::from_default_env`)
  2. `config.log_level` (default: `Level::INFO`)
- The stderr layer is installed with a `with_filter(LevelFilter::OFF)` when
  the TUI is about to take over the terminal, and swapped to `INFO` by the
  panic hook and the `Drop` on `TelemetryGuard`.

---

## O2 — `RingLayer` in-memory buffer

```rust
/// Fixed-capacity ring buffer of recent log events, shared via `Arc<Mutex<…>>`
/// so the TUI draw path can snapshot it without blocking log producers.
#[derive(Clone)]
pub struct LogRing {
    inner: Arc<Mutex<VecDeque<OwnedEvent>>>,
    capacity: usize,
}

#[derive(Clone, Debug)]
pub struct OwnedEvent {
    pub timestamp: std::time::SystemTime,
    pub level: tracing::Level,
    pub target: String,
    pub message: String,
    pub fields: indexmap::IndexMap<String, String>,
}

impl LogRing {
    pub fn snapshot(&self) -> Vec<OwnedEvent> { /* lock, clone, unlock */ }
    pub fn clear(&self) { /* lock, clear */ }
}
```

`RingLayer: tracing_subscriber::Layer<S>` visits the event's fields into an
`IndexMap<String, String>` via a `Visit` impl, pushes `OwnedEvent` to the
front, and pops from the back when over capacity.

### Requirements

- Lock hold time: a snapshot of a full 2 000-entry ring must complete in
  < 5 ms on a 2020-vintage laptop (benchmark in `benches/`).
- No event is dropped silently. If capacity is reached the oldest entry is
  evicted; an internal counter `dropped_count` is exposed via
  `LogRing::dropped_count()`.
- Thread-safe: `LogRing: Send + Sync + Clone`.

---

## O3 — `Redactor` layer

```rust
/// Redacts credential-bearing fields before they reach any downstream layer.
///
/// Matching rules (case-insensitive):
/// - Field names containing: "authorization", "auth", "token", "password",
///   "secret", "api_key", "apikey", "cookie", "set-cookie"
/// - Any field whose value parses as a URL with non-empty userinfo — the
///   userinfo is replaced with `***`
/// - Any field whose value starts with `Bearer ` or `Basic `
#[derive(Default)]
pub struct Redactor;
```

Replacements always use the literal string `"<redacted>"` so tests can assert
presence/absence without matching on variable content.

### Requirements

- URL userinfo redaction uses `url::Url::set_username("")` and
  `set_password(None)`; never substring manipulation.
- A new field name matching the denylist added in the future does **not**
  require code changes — the list lives in a `&'static [&'static str]`
  constant in `obs::redact` and is the single source of truth.
- Unit tests cover every rule and at least one negative case per rule.

---

## O4 — Kill duplicate initialisation

### Remove from `App::run`

Delete lines 168–171 of [app.rs](../c2pa-tui/src/app.rs):

```rust
// DELETE:
let _ = tracing_subscriber::fmt()
    .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
    .with_writer(std::io::stderr)
    .try_init();
```

### Keep the panic hook but have it re-enable stderr

The existing `PANIC_HOOK` at `app.rs:177` leaves the alternate screen and
disables raw mode. Extend it so the last thing before calling the default
panic handler is to toggle the stderr layer back on. Because we took the
`unwind` path in spec-21, the hook runs, the default handler prints the
panic message, and the process then unwinds — `TelemetryGuard::drop` flushes
pending writes.

```rust
PANIC_HOOK.get_or_init(|| {
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stderr(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture,
        );
        // NEW: re-enable stderr logging so the panic message reaches the user.
        crate::obs::enable_stderr();
        // Emit a structured record so the file log captures the panic too.
        tracing::error!(panic = %info, "process panicked");
        default_panic(info);
    }));
});
```

### Requirements

- Running `RUST_LOG=debug cargo run -p c2pa-tui -- fixtures/x.jpg` does not
  corrupt the TUI — no text bleeds onto the alternate screen.
- `grep -R "tracing_subscriber::fmt()" src/` returns zero hits outside
  `src/obs/`.
- The panic hook is installed after `obs::init` but before
  `EnterAlternateScreen` (order matters — a panic during terminal setup must
  still restore the screen).

---

## Testing strategy

### `obs/redact.rs`

```rust
#[test]
fn redacts_authorization_field() {
    // emit a tracing::info!(authorization = "Bearer abc123", "request sent");
    // snapshot the ring; assert fields["authorization"] == "<redacted>"
}

#[test]
fn redacts_url_userinfo() {
    // emit info!(url = "https://user:pass@example.com/foo");
    // assert ring event's url field == "https://***@example.com/foo"
}

#[test]
fn passes_through_benign_fields() {
    // emit info!(path = "/etc/hosts"); assert path unchanged
}
```

### `obs/ring.rs`

```rust
#[test]
fn ring_respects_capacity() {
    let ring = LogRing::with_capacity(3);
    for i in 0..5 { /* push OwnedEvent */ }
    assert_eq!(ring.snapshot().len(), 3);
    assert_eq!(ring.dropped_count(), 2);
}
```

### `obs/mod.rs`

```rust
#[test]
fn init_is_idempotent() {
    let _g1 = obs::init(&Config::default());
    let _g2 = obs::init(&Config::default());  // must not panic or double-subscribe
}
```

### Integration

- `tests/integration_obs.rs` — boot a fake `App`, emit events from multiple
  modules, assert they appear in the ring and in the rotating file.
- `benches/log_ring.rs` — criterion benchmark for snapshot latency.

---

## Edge cases

- **Home dir not writable** (e.g. sandboxed CI): fall back to `std::env::temp_dir()`
  and emit one `warn!` via stderr once the subscriber is ready.
- **Disk full mid-session**: `non_blocking` writer drops events; the counter
  in `LogRing::dropped_count()` catches the ring-side analogue. Status-bar
  rendering (spec-20) surfaces this.
- **Windows `XDG_STATE_HOME` absence**: use `dirs::data_local_dir()` — the
  `dirs` crate handles platform mapping.
- **Tests running in parallel**: the `OnceLock` means only the first test
  fully initialises; subsequent tests reuse the subscriber. Any test that
  asserts ring contents must clear the ring first.

---

## Dependencies

- `tracing-appender = "0.2"` — rolling file writer with non-blocking guard
- `dirs = "5"` — cross-platform state-dir resolution
- No new dev deps (criterion is already present)

---

## Done criteria

```bash
cargo test -p c2pa-tui -- obs::
cargo test -p c2pa-tui --test integration_obs
cargo clippy -p c2pa-tui -- -D warnings
cargo fmt -p c2pa-tui -- --check
```

- `grep -R "tracing_subscriber::fmt()" c2pa-tui/src/` returns no hits.
- Running with `RUST_LOG=debug` produces a file under
  `$XDG_STATE_HOME/c2pa-tui/logs/` and never writes to stderr during the TUI
  session.
- Log file contains no `Authorization:` header value, no `Bearer <token>`, no
  `user:pass@` URL userinfo (verified by a grep step in the test suite).
