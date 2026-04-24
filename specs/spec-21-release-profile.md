# Spec 21 — Release Profile & Panic Safety

**Phase:** 8 (parallel with spec-22 — requires spec-19 merged)
**Depends on:** spec-19
**Produces:** crate-local `[profile.release]`; audited panic hook; `cargo bloat` CI gate; split debug-info artifact

---

## Goal

The workspace `[profile.release]` at `Cargo.toml:25` already sets `strip = true`,
`opt-level = 3`, and `lto = "thin"`. For `c2pa-tui` specifically there are two
extra concerns:

1. **Binary size** (NF1 ≤ 15 MB Linux x64). Workspace defaults produce a
   binary around 18–19 MB because `codegen-units = 16` (default) leaves
   inlining opportunities on the table and because `panic = "unwind"` carries
   the full unwinding machinery from every dependency.
2. **Useful backtraces**. Stripping symbols eliminates file:line resolution
   for panics. We must ship separate debug-info artifacts so incidents can be
   root-caused from a bug report + correlation ID alone.
3. **Terminal hygiene on panic**. The existing `PANIC_HOOK` at
   [app.rs:177](../c2pa-tui/src/app.rs:177) restores the terminal before the
   default handler runs. This spec verifies that invariant end-to-end and
   closes two known gaps.

> **Panic strategy:** `panic = "unwind"` is kept. Abort would shave ~1.2 MB
> but prevents the existing `Drop` in `TelemetryGuard` (spec-19) from
> flushing the log file — and we prioritise diagnosability over binary size.

---

## Files to modify

- `c2pa-tui/Cargo.toml` — new `[profile.release]` overrides
- `src/app.rs` — panic-hook audit; emit a `tracing::error!(panic = …)`
  before calling the default handler (coordinates with spec-19 O4)
- `.github/workflows/release.yml` — new; size gate and debug-info split
- `c2pa-tui/docs/RELEASE.md` — new; documents symbol-lookup procedure

---

## P1 — Crate-local release profile

```toml
# c2pa-tui/Cargo.toml

[profile.release]
# Inherits workspace: strip = true, opt-level = 3, lto = "thin"
codegen-units = 1        # maximum inlining; ~5% smaller, ~10% slower compile
incremental   = false    # release builds never benefit from incremental
overflow-checks = false  # already the default; make intent explicit

[profile.release-debug]
inherits = "release"
strip    = false         # keep symbols for the sidecar debug artifact
debug    = "full"        # DWARF file:line tables
split-debuginfo = "packed"  # emit a .dwp (Linux) / .dSYM (macOS)
```

### Why `release-debug`, not `release`?

The stripped release binary is what ships to users. The `release-debug`
profile produces a bit-identical binary *plus* a sidecar debug archive. CI
uploads both artifacts on a tag; support staff use the sidecar to resolve a
backtrace from a user-submitted correlation ID.

### Requirements

- `cargo build -p c2pa-tui --release` produces a binary ≤ 15 MB on
  `x86_64-unknown-linux-gnu` (measured after `strip`).
- `cargo build -p c2pa-tui --profile release-debug` produces the binary plus
  a `.dwp` file on Linux, `.dSYM` bundle on macOS, `.pdb` on Windows.
- Both profiles produce byte-identical stripped binaries when compared via
  `diff <(objcopy --strip-all a) <(objcopy --strip-all b)`. (This is the
  guarantee that makes the sidecar useful — if the binaries diverge, the
  symbols don't match.)

---

## P2 — Panic hook audit

Spec-19 O4 extends the existing hook to re-enable stderr and emit a log
event. This spec adds two further guarantees:

### G1 — Hook installed before any terminal setup

A panic *during* `EnterAlternateScreen` would leave the terminal in an
unrecoverable state because the hook has no `LeaveAlternateScreen` to pair
with. Move the `PANIC_HOOK.get_or_init` call out of `App::run` and into
`obs::init` so it is guaranteed to be active before the TUI setup block at
`app.rs:190-197` runs.

### G2 — Hook is composable, not swallowing

The current hook ignores errors from `disable_raw_mode` and `execute!`.
That's correct — the panic is already in progress, nothing to do — but it
must not also ignore the default handler. Verify the `default_panic(info)`
call at the end of the closure is unconditional.

### Updated hook (in `obs::init`, from spec-19)

```rust
static PANIC_HOOK: OnceLock<()> = OnceLock::new();
PANIC_HOOK.get_or_init(|| {
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // 1. Restore terminal — best-effort, ignore errors.
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stderr(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture,
        );
        // 2. Re-enable stderr logging so the user sees the panic message.
        crate::obs::enable_stderr();
        // 3. Structured record — survives in the log file via unwind + Drop.
        let location = info.location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "<unknown>".into());
        let payload = info.payload().downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(|s| s.as_str()))
            .unwrap_or("<non-string payload>");
        tracing::error!(
            panic.location = %location,
            panic.payload = %payload,
            "process panicking — terminal restored, unwinding"
        );
        // 4. Delegate — default handler prints panic to stderr and, if the
        //    unwind reaches main, the process exits with code 101.
        default_panic(info);
    }));
});
```

### Requirements

- Panic during `App::run` before any draw call: terminal is restored,
  log file contains the `process panicking` event, process exits 101.
- Panic during the ratatui draw closure: same behaviour (backend is
  dropped, `impl Drop for Terminal` runs via unwind).
- Double panic (panic in `Drop` during unwind): documented as aborting —
  `std::process::abort()` fires from std, the stderr layer is already back
  on so the user sees it, log file may be truncated. This is acceptable.

### Tests

Two tests in `tests/integration_panic.rs` using `std::panic::catch_unwind`:

```rust
#[test]
fn panic_in_event_loop_restores_terminal() {
    // Use a mock `ManifestSource` whose `load` panics.
    // Run App::run in a thread that uses catch_unwind.
    // After join, assert stdout is not in raw mode (isatty probe) and the
    // ring contains a panic event.
}

#[test]
fn panic_hook_is_idempotent_across_app_runs() {
    // Call obs::init twice. Panic once. Assert exactly one "process panicking"
    // event in the ring (not two — would indicate a stacked hook).
}
```

---

## P3 — CI size gate & debug artifact

### `.github/workflows/release.yml` (new)

```yaml
name: release
on:
  push:
    tags: ['c2pa-tui-v*']

jobs:
  build:
    strategy:
      matrix:
        include:
          - { os: ubuntu-latest,  target: x86_64-unknown-linux-gnu,   size_limit: 15728640 }  # 15 MB
          - { os: ubuntu-latest,  target: x86_64-unknown-linux-musl,  size_limit: 16777216 }  # 16 MB (musl slightly bigger)
          - { os: macos-14,       target: aarch64-apple-darwin,       size_limit: 17825792 }  # 17 MB
          - { os: macos-13,       target: x86_64-apple-darwin,        size_limit: 17825792 }
          - { os: windows-latest, target: x86_64-pc-windows-msvc,     size_limit: 19922944 }  # 19 MB best-effort
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - run: rustup target add ${{ matrix.target }}
      - run: cargo build -p c2pa-tui --profile release-debug --target ${{ matrix.target }}
      - name: Enforce size budget
        run: |
          bin=target/${{ matrix.target }}/release-debug/c2pa-tui
          size=$(stat -c%s "$bin" 2>/dev/null || stat -f%z "$bin")
          [ "$size" -le ${{ matrix.size_limit }} ] || { echo "binary too large: $size > ${{ matrix.size_limit }}"; exit 1; }
      - name: cargo bloat report
        run: cargo bloat --release -p c2pa-tui -n 20 > bloat-${{ matrix.target }}.txt
      - uses: actions/upload-artifact@v4
        with:
          name: c2pa-tui-${{ matrix.target }}
          path: |
            target/${{ matrix.target }}/release-debug/c2pa-tui*
            bloat-${{ matrix.target }}.txt
```

### Requirements

- Workflow triggers only on `c2pa-tui-v*` tags — does not run on every PR.
- Windows is **best-effort**: size limit is generous (19 MB) and a build
  failure on Windows does not block a Linux/macOS release. Use
  `continue-on-error: true` for the Windows matrix row.
- Every target produces a stripped binary **and** a debug-info sidecar
  (`.dwp` / `.dSYM` / `.pdb`) in the artifact.
- A `cargo-bloat` text report is attached so regressions in size are
  attributable to a specific crate.

---

## P4 — Symbol lookup procedure (`docs/RELEASE.md`)

Short new doc — less than one page. Covers:

- How to match a binary checksum to a debug-info sidecar
  (`sha256sum target/release/c2pa-tui`).
- How to resolve a panic location: `addr2line -e c2pa-tui.dwp <addr>` on
  Linux, `atos -o c2pa-tui.dSYM <addr>` on macOS.
- How to map a user's correlation ID to a log entry in their diagnostic
  bundle (spec-22).

This file is not user-facing — it lives alongside the other `docs/` entries
and is intended for maintainers handling bug reports.

---

## Edge cases

- **`cargo bloat` not installed** in CI: add a cached install step
  (`cargo install cargo-bloat --locked`) gated on a tool-cache key.
- **Workspace profile changes upstream**: the crate-local `[profile.release]`
  inherits from the workspace profile, so adding `codegen-units = 1` does not
  re-specify other settings. If the workspace later changes
  `opt-level` we want that to propagate — verified in a test script.
- **MSRV enforcement**: `split-debuginfo = "packed"` requires Rust 1.65+.
  MSRV is 1.88 per `Cargo.toml:5`, no issue.
- **Windows PDB path in stripped binary**: MSVC embeds a PDB path in the
  binary by default. Since the binary is stripped this doesn't leak user
  paths, but verify once in CI that the stripped binary contains no
  absolute paths matching `C:\\` or `/Users/`.

---

## Dependencies

- `cargo-bloat` in CI (not a crate dep).
- No new runtime deps.

---

## Done criteria

```bash
# Local
cargo build -p c2pa-tui --release
size=$(stat -c%s target/release/c2pa-tui 2>/dev/null || stat -f%z target/release/c2pa-tui)
[ "$size" -le 15728640 ]

cargo build -p c2pa-tui --profile release-debug
ls target/release-debug/c2pa-tui*  # binary + debug sidecar

cargo test -p c2pa-tui --test integration_panic
cargo clippy -p c2pa-tui -- -D warnings
cargo fmt -p c2pa-tui -- --check
```

- CI workflow file validated by `act` or `gh workflow view`.
- `docs/RELEASE.md` exists and covers all three host platforms.
- Manual test: induce a panic via a mock source that always panics; assert
  terminal returns to a normal shell prompt and the log file contains the
  panic record with a file:line location.
