# Spec 13 — Polish & Idiomatic Corrections

**Phase:** 4 (sequential — requires spec-10, spec-11, and spec-12 merged and `cargo build` clean)  
**Depends on:** spec-10, spec-11, spec-12  
**Produces:** `FromStr` impl for `Auth`; `pub(crate)` field visibility on `App`;
rustdoc on `RemoteClient::default`.

---

## Goal

Apply three low-severity idiomatic improvements that make the codebase more
maintainable without changing observable behaviour:

1. Implement `std::str::FromStr` for `Auth` so `clap` can call it automatically.
2. Restrict `App` field visibility to `pub(crate)` so the UI layer cannot
   accidentally mutate internal state.
3. Document `RemoteClient::default()` to clarify its intended scope.

**Sequencing note:** this spec must run after spec-12 because D2 (narrowing field
visibility) breaks any external test that writes to `App` fields directly.
Spec-12 migrates all snapshot tests to `App::with_loaded_for_tests` (added in
spec-11) before this change lands.

---

## Files to modify

- `src/remote/auth.rs` — add `FromStr` impl
- `src/main.rs` — switch `--auth` arg type from `String` to `Auth`
- `src/app.rs` — change `pub` fields to `pub(crate)`; add public read accessors
- `src/remote/client.rs` — add doc comment to `Default` impl
- `tests/snapshot_ui.rs` — migrate any remaining direct field writes to accessor calls
  (should be zero if spec-12 was implemented correctly, but verify)

---

## D1 — `std::str::FromStr` for `Auth`

Add to `src/remote/auth.rs`:

```rust
impl std::str::FromStr for Auth {
    type Err = crate::error::AppError;

    /// Parse an auth specification string.  Delegates to [`Auth::from_spec`].
    fn from_str(s: &str) -> crate::error::Result<Self> {
        Auth::from_spec(s)
    }
}
```

Keep `from_spec` as a named alias — it reads better in test code and allows
callers to be explicit about the custom parsing rules.

### Update `main.rs`

Change the `auth` field in the `Cli` struct from `String` to `Auth`:

```rust
// Before
/// Authentication spec: none | basic:user:pass | bearer:token | digest:user:pass
#[arg(long, default_value = "none")]
auth: String,

// After — clap calls Auth::from_str automatically via the ValueParser blanket impl
/// Auth spec: none | basic:user:pass | bearer:token | digest:user:pass
/// Secrets can be indirected: bearer:env:MY_VAR or bearer:file:/path/to/token
#[arg(long, default_value = "none")]
auth: Auth,
```

Remove the manual `Auth::from_spec(&cli.auth)` call and use `cli.auth` directly
everywhere in `main`.

**Note:** `clap` requires `FromStr` + `Clone` for `#[arg]` fields parsed this way.
`Auth` already derives `Clone`, so no additional change is needed.

### Tests

```rust
#[test]
fn from_str_delegates_to_from_spec() {
    use std::str::FromStr;
    let a = Auth::from_str("bearer:tok").unwrap();
    let b = Auth::from_spec("bearer:tok").unwrap();
    match (a, b) {
        (Auth::Bearer { token: t1 }, Auth::Bearer { token: t2 }) => assert_eq!(t1, t2),
        _ => panic!("expected Bearer from both paths"),
    }
}

#[test]
fn from_str_propagates_errors() {
    use std::str::FromStr;
    assert!(Auth::from_str("oauth:something").is_err());
}
```

---

## D2 — Restrict `App` field visibility to `pub(crate)`

All `App` fields are currently `pub`, meaning external crates and integration
tests can write to internal state directly, bypassing invariants.

### Step 1 — audit `tests/` for direct field writes

Before changing visibility, search for any test code in `tests/` that writes to
`App` fields:

```sh
rg -n "app\.(loaded|sources|state|selected_left|compare_selection|show_help|show_all_diffs)" \
   c2pa-tui/tests/
```

Any hits are writes that must be migrated to:
- `App::with_loaded_for_tests` for injecting loaded nodes (spec-11)
- `App::new(config)` plus the public event-handler methods for state transitions

If spec-12 was implemented correctly there should be zero hits, but verify before
proceeding.

### Step 2 — change field visibility

```rust
pub struct App {
    pub(crate) sources: Vec<(SourceId, Arc<dyn ManifestSource>)>,
    pub(crate) loaded: HashMap<SourceId, LoadState>,
    pub(crate) selected_left: Option<SourceId>,
    pub(crate) compare_selection: Option<SourceId>,
    pub(crate) filter: FieldFilter,
    pub(crate) matcher: Matcher,
    pub(crate) state: AppState,
    pub(crate) config: Config,
    pub(crate) client: RemoteClient,
    pub(crate) focused_pane: Pane,
    pub(crate) show_help: bool,
    pub(crate) show_all_diffs: bool,
    pub(crate) compare_diff_cache: Option<ManifestDiff>,
    pub(crate) layout_cache: Option<(ratatui::layout::Rect, CachedLayout)>,
    pub(crate) detail_tree_state: TreeState<String>,
    pub(crate) loading_count: usize,
    pub(crate) search_results: Vec<MatchResult>,
    pub(crate) search_cursor: usize,
    pub(crate) search_result_indices: HashSet<usize>,
}
```

### Step 3 — add public read accessors for external tests

```rust
impl App {
    /// Returns the number of registered sources.
    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    /// Returns the current `AppState`.
    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Returns `true` if the help overlay is visible.
    pub fn is_help_visible(&self) -> bool {
        self.show_help
    }

    /// Returns the load state for the currently selected source, if any.
    pub fn selected_load_state(&self) -> Option<&LoadState> {
        self.selected_left.and_then(|id| self.loaded.get(&id))
    }
}
```

Add additional accessors as needed to keep integration tests and the public API
surface compiling.  **Do not add accessors that return `&mut` references** —
all mutations must go through the event handlers in `app.rs`.

### Step 4 — fix compile errors

`cargo build` will emit errors for every out-of-crate field access.  Fix by
replacing direct field reads with the accessors above.  The `ui/` module reads
`app` fields for rendering; those accesses are `pub(crate)` and remain valid
because `ui` is part of the same crate.

---

## D3 — Document `RemoteClient::default()`

Spec-10 fixed the `Default` impl to delegate to `new()`.  Add the doc comment:

```rust
impl Default for RemoteClient {
    /// Construct a `RemoteClient` with the same timeouts and user-agent as
    /// [`RemoteClient::new`].
    ///
    /// Provided primarily for test convenience and for contexts where
    /// `Result`-based construction is inconvenient.  Production code should
    /// prefer [`RemoteClient::new`] for explicit error handling.
    fn default() -> Self {
        Self::new().expect("RemoteClient::new is infallible with default settings")
    }
}
```

No test change is needed — spec-12 C5 already guards against regression.

---

## Done criteria

```
cargo build
cargo test
cargo fmt -- --check
cargo clippy -- -D warnings
```

Verify clap parsing end-to-end:

```sh
cargo run -- --auth bearer:mytoken --help
cargo run -- --auth basic:user:pass --help
cargo run -- --auth none --help
cargo run -- --auth badspec 2>&1 | grep -i "error"
```
