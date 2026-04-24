# Spec 22 — Diagnostic Bundle

**Phase:** 8 (parallel with spec-21 — requires spec-20 merged)
**Depends on:** spec-19, spec-20
**Produces:** `--diag-bundle <path>` CLI flag; auto-offer prompt on `Internal`-class errors; redacted tar.gz writer

---

## Goal

When a user hits a bug in a release build, they should be able to produce a
single file that contains everything a maintainer needs to triage the report
— and nothing else. Today the only recourse is "attach the terminal scrollback,"
which is lossy and may contain credentials the user doesn't realise are
visible.

This spec adds a `diag::Bundle` writer that assembles:

- The last seven log files from spec-19 (already redacted)
- A `manifest.json` describing the session: version, OS/arch, feature flags,
  active theme, active `Auth` kind (scheme only, never values), CLI argv
  with the `--auth` value replaced by `<redacted>`
- Every current `LoadState::Failed` entry with its correlation ID
- The ring-buffer snapshot (spec-19) at the moment of capture

Two entry points:

1. `c2pa-tui --diag-bundle /tmp/report.tgz` — CLI one-shot, writes the bundle
   without starting the TUI (used when the UI itself is broken).
2. **Auto-offer after `ErrorClass::Internal`**: the error overlay footer
   changes from `"Press any key to dismiss"` to `"d: save diag bundle · any
   other key to dismiss"`. Pressing `d` opens a tiny path-entry overlay
   defaulting to `~/c2pa-tui-diag-<timestamp>.tgz`.

---

## Files to modify

- `src/diag/mod.rs` — new; `Bundle`, `BundleManifest`, `write_bundle`
- `src/main.rs` — parse `--diag-bundle`; dispatch before App startup
- `src/app.rs` — handle `d` key in error overlay when class is `Internal`;
  add `AppState::DiagBundlePath { path: String }` for the path-entry overlay
- `src/ui/diag_bundle_prompt.rs` — new; small path-entry widget
- `src/error.rs` — `AppError::Diag(String)` for bundle-writer failures
- `Cargo.toml` — add `tar = "0.4"`, `flate2 = "1"` (gzip)

---

## D1 — `diag::Bundle` writer

```rust
/// Session-level diagnostic information suitable for attaching to a bug report.
///
/// Every field is guaranteed to be free of credential material after construction.
pub struct Bundle {
    pub manifest: BundleManifest,
    pub log_files: Vec<PathBuf>,
    pub ring_snapshot: Vec<OwnedEvent>,
    pub failures: Vec<FailureEntry>,
}

#[derive(serde::Serialize)]
pub struct BundleManifest {
    pub version: &'static str,        // env!("CARGO_PKG_VERSION")
    pub build_profile: &'static str,  // "release" / "release-debug" / "debug"
    pub features: Vec<&'static str>,  // compile-time feature flags
    pub os: &'static str,             // std::env::consts::OS
    pub arch: &'static str,           // std::env::consts::ARCH
    pub timestamp: String,            // RFC 3339
    pub argv: Vec<String>,            // with --auth value replaced
    pub auth_scheme: &'static str,    // "none" / "basic" / "bearer" / "digest"
    pub theme: String,
    pub mouse_enabled: bool,
    pub sources: Vec<SourceRef>,      // labels only, no credentials
}

#[derive(serde::Serialize)]
pub struct SourceRef {
    pub id: String,       // SourceId::display
    pub kind: &'static str,  // "file" / "dir" / "remote"
    pub label: String,    // filename only, URLs stripped of userinfo + query
}

#[derive(serde::Serialize)]
pub struct FailureEntry {
    pub id: String,               // CorrelationId::display
    pub source_label: String,     // redacted same way as SourceRef
    pub error: String,            // raw error display, already redacted by Reporter
}

impl Bundle {
    /// Capture the current state. Pure — does no I/O, so it cannot fail.
    pub fn capture(app: &App, log_dir: &Path) -> Bundle { /* … */ }

    /// Serialize to a gzip-compressed tar archive.
    pub fn write<W: Write>(&self, w: W) -> Result<(), AppError> { /* … */ }
}
```

### Tar layout

```
c2pa-tui-diag-20260424T152233Z/
├── manifest.json
├── ring.json
├── failures.json
└── logs/
    ├── c2pa-tui.log.2026-04-22
    ├── c2pa-tui.log.2026-04-23
    └── c2pa-tui.log.2026-04-24
```

Timestamps in filenames use the `rolling` appender's format. The top-level
directory is named from the capture timestamp so extracting two bundles into
the same directory does not collide.

### Requirements

- `Bundle::write` never panics. All I/O failures surface as
  `AppError::Diag(…)`.
- The tar archive is deterministic modulo timestamps: given the same inputs
  and capture time, two invocations produce byte-identical output. (Enables
  a reproducibility test in CI.)
- Output is gzip-compressed with default compression level.
- The writer streams — it does not buffer the whole archive in memory.

---

## D2 — Redaction audit

Before writing, every string that enters the bundle passes through one of:

- Values already produced by spec-19's `Redactor` layer (log files, ring
  events) — no double-redaction, trust upstream.
- A new `diag::scrub_url` helper for URL fields that bypass the tracing
  pipeline (e.g. `SourceRef::label` for remote sources).
- A new `diag::scrub_argv(&[String])` that finds a `--auth` argument and
  replaces the next argument's value with `<redacted>` (and handles
  `--auth=<value>` form).

```rust
pub fn scrub_argv(argv: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(argv.len());
    let mut i = 0;
    while i < argv.len() {
        if argv[i] == "--auth" && i + 1 < argv.len() {
            out.push(argv[i].clone());
            out.push("<redacted>".into());
            i += 2;
            continue;
        }
        if let Some(rest) = argv[i].strip_prefix("--auth=") {
            let scheme = rest.splitn(2, ':').next().unwrap_or("");
            out.push(format!("--auth={}:<redacted>", scheme));
            i += 1;
            continue;
        }
        out.push(argv[i].clone());
        i += 1;
    }
    out
}
```

### Requirements

- Unit tests for `scrub_argv` cover: no `--auth`, `--auth none`,
  `--auth basic:user:pw`, `--auth=bearer:abc`, `--auth` at end of argv
  (malformed — no value follows).
- A bundle produced from a session that used `--auth bearer:supersecret`
  contains neither `supersecret` nor any of its substrings of length ≥ 6
  (verified by a property-based test that reads the tar back).
- `scrub_url` strips userinfo via `url::Url::set_username("")` +
  `set_password(None)` and drops the query string entirely.

---

## D3 — CLI flag

```rust
// main.rs — clap struct addition
/// Write a diagnostic bundle to the given path and exit without starting
/// the TUI. Output is gzip-compressed tar. See docs/GETTING_STARTED.md for
/// redaction guarantees.
#[arg(long, value_name = "PATH")]
diag_bundle: Option<PathBuf>,
```

### Behaviour

When `--diag-bundle` is present:

1. `obs::init` runs (log file is part of the bundle).
2. No TUI setup; no sources loaded.
3. `Bundle::capture(&App::new_empty(), &log_dir)` captures the last-seven
   log files (from prior sessions) plus an empty failures/ring.
4. Write the bundle to the path.
5. Print `"wrote diagnostic bundle: <path>\n  manifest: <size> bytes\n  logs: N files"`.
6. Exit 0.

### Requirements

- The CLI path is robust to a non-existent parent directory — creates it
  with mode 0o700 on Unix.
- Relative paths resolve against `$PWD`.
- Exit code is 1 if writing fails, 0 on success.

---

## D4 — Auto-offer on `Internal` errors

### Error overlay update

Spec-20's error overlay footer is:

```
Press any key to dismiss · F2 for logs
```

When `AppState::Error { class: Internal, .. }`, the footer becomes:

```
d: save diagnostic bundle · F2 for logs · any other key to dismiss
```

### Key handler

In `handle_error_key` (new; split out from the current "any key dismisses"
logic):

```rust
fn handle_error_key(&mut self, key: KeyEvent) {
    let is_internal = matches!(
        self.state,
        AppState::Error { class: ErrorClass::Internal, .. }
    );
    match key.code {
        KeyCode::Char('d') if is_internal => {
            let default_path = format!(
                "c2pa-tui-diag-{}.tgz",
                chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
            );
            self.state = AppState::DiagBundlePath { path: default_path };
        }
        KeyCode::F(2) => self.show_log = !self.show_log,
        _ => self.state = AppState::Browse,
    }
}
```

### Path-entry overlay

Small single-line input overlay (`src/ui/diag_bundle_prompt.rs`) — same
construction as the existing filter bar ([ui/filter_bar.rs](../c2pa-tui/src/ui/filter_bar.rs)).

- `Enter` — call `Bundle::capture(self, &log_dir).write(File::create(path)?)`;
  on success set `self.state = AppState::Error { class: UserInput, title:
  "Diagnostic bundle saved", detail: format!("{}"), .. }` (reuse the overlay
  for the confirmation — a cheap way to display the path without a new
  state).
- `Esc` — back to `Browse`.
- `Backspace`/char — edit `path`.

### Requirements

- The `d` key only works when `class == Internal`. For `UserInput` /
  `Transient` errors, `d` is a pass-through to "dismiss".
- The bundle includes the error that triggered the auto-offer (because the
  corresponding `LoadState::Failed` is already in `app.loaded`).
- If `Bundle::write` fails mid-stream, a partial file may be left at the
  target path — documented in `docs/GETTING_STARTED.md`; no cleanup attempted.

---

## Testing strategy

### `diag/mod.rs`

```rust
#[test]
fn bundle_writer_produces_valid_gzip_tar() {
    let app = App::test_fixture_with_failures();
    let bundle = Bundle::capture(&app, Path::new("/tmp/nonexistent"));
    let mut buf = Vec::new();
    bundle.write(&mut buf).unwrap();
    // Round-trip: decompress + untar, assert manifest.json parses.
}

#[test]
fn scrub_argv_handles_all_auth_forms() { /* table-driven */ }

#[test]
fn bundle_contains_no_credentials() {
    // Build a session that set --auth bearer:supersecretvalue123
    // Capture bundle, read every byte back
    // assert!(!bytes.windows(10).any(|w| w == b"supersecre"));
}
```

### Integration

`tests/integration_diag_bundle.rs`:
- Launch the CLI with `--diag-bundle /tmp/x.tgz` and assert exit 0 and tar
  validity via the `tar` crate.
- Run an in-process session that induces an `Internal` error, inject a
  `d` keypress + path, assert a bundle lands at the path.

### Snapshot

One `insta` snapshot for the diag-bundle prompt overlay.

---

## Edge cases

- **Log directory doesn't exist** (first run, no logs yet): the bundle
  contains a `logs/` directory with only the current session's partial log.
- **Disk full on write**: `AppError::Diag("no space left on device")`
  surfaces through the `Reporter`; classified `Transient`.
- **User types `~` in path**: no shell expansion. Path is taken verbatim.
  (The TUI is not a shell.) Documented in the prompt footer: "absolute
  path only".
- **Bundle > 100 MB**: the log rotation cap already bounds this. Documented
  maximum: ~110 MB in pathological cases.
- **Windows path separators in tar**: `tar` crate normalises to forward
  slashes. Tested.

---

## Dependencies

- `tar = "0.4"` — pure-Rust tar writer
- `flate2 = "1"` — gzip compression (feature: `rust_backend` for no C deps)
- `chrono = "0.4"` — RFC 3339 timestamps (pulls in the `clock` feature)

Total dep weight increase: ~60 KB in the stripped binary. Within the NF1
budget.

---

## Done criteria

```bash
cargo test -p c2pa-tui -- diag::
cargo test -p c2pa-tui --test integration_diag_bundle
cargo clippy -p c2pa-tui -- -D warnings
cargo fmt -p c2pa-tui -- --check
```

- Running `c2pa-tui --diag-bundle /tmp/x.tgz` produces a valid archive and
  exits 0.
- Property test asserts no credential material in any byte of any
  bundle across 1 000 random `--auth` values.
- Auto-offer appears exactly for `Internal`-class errors in a manual
  smoke test.
