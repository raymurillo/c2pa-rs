# Spec 17 — Data Model Cleanup

**Phase:** 6 (sequential — requires spec-16 merged)  
**Depends on:** spec-13, spec-16  
**Produces:** unified filter API; refactored `FieldDiff`; `AppError::InvalidInput`; decoupled error UI text

---

## Goal

Four idiomatic Rust and code quality findings that touch the data model and error
types, grouped because they are low-risk refactors that can be implemented in one
session:

- **Finding 9** — `apply` and `apply_ref` in `filter.rs` are near-identical
  functions that duplicate ~35 lines of logic.  One should delegate to the other.
- **Finding 12** — All four `FieldDiff` enum variants carry `path: String`,
  making every match arm repeat the same binding.  Extracting `path` to a
  wrapper struct reduces boilerplate across the diff and compare-UI code.
- **Finding 13** — `FieldFilter::from_query` constructs fake `glob::PatternError`
  values with hard-coded `msg` strings to return validation errors.  These are
  not glob-pattern errors; they need a dedicated `AppError::InvalidInput`
  variant.
- **Finding 18** — `AppState::Error { message }` pre-bakes the UI dismiss
  prompt (`"Press any key to dismiss."`) into the stored message string, making
  the raw error inaccessible for logging and tests.
- **Finding 19** — `comparison_value` in `diff.rs` re-parses the `"key: value"`
  format string on every field comparison.  The value can be extracted once when
  building the `IndexMap`.

---

## Files to modify

- `src/manifest/filter.rs` — unify `apply`/`apply_ref`
- `src/compare/diff.rs` — `FieldDiff` refactor; pre-compute comparison values
- `src/error.rs` — add `AppError::InvalidInput`
- `src/app.rs` — store raw error message in `AppState::Error`; update render

---

## D1 — Unify `apply` and `apply_ref`

### Current

Two functions with identical logic, differing only in whether the input nodes
are owned or borrowed:

```rust
pub fn apply(&self, nodes: Vec<DisplayNode>) -> Vec<DisplayNode>   { apply_inner(...) }
pub fn apply_ref(&self, nodes: &[DisplayNode]) -> Vec<DisplayNode> { apply_inner_ref(...) }
```

### Fix

Remove `apply_inner` (the owned variant).  Make `apply` delegate to `apply_ref`:

```rust
/// Apply this filter, consuming the input node list.
///
/// Equivalent to `apply_ref(&nodes)` but accepts an owned `Vec`.
pub fn apply(&self, nodes: Vec<DisplayNode>) -> Vec<DisplayNode> {
    self.apply_ref(&nodes)
}
```

`apply_ref` (with the path-buffer optimisation from spec-16 H2) is the single
implementation.  The owned `apply` no longer has a performance advantage because
spec-16 already makes `apply_ref` avoid unnecessary clones of pruned nodes.

### Requirements

- `apply` and `apply_ref` produce identical results (already enforced by
  existing proptest `apply_ref_matches_apply`).
- The `apply_inner` function is deleted.
- All existing callers of `apply` and `apply_ref` continue to compile.

---

## D2 — `FieldDiff` refactor

### Current

```rust
pub enum FieldDiff {
    Equal   { path: String, value: String },
    Changed { path: String, left: String, right: String },
    OnlyLeft  { path: String, value: String },
    OnlyRight { path: String, value: String },
}
```

Every match arm across `diff.rs`, `ui/compare.rs`, and tests repeats
`path` as an explicit binding.

### New types

```rust
/// The kind of difference for a single manifest field.
#[derive(Debug, Clone, PartialEq)]
pub enum FieldDiffKind {
    Equal   { value: String },
    Changed { left: String, right: String },
    OnlyLeft  { value: String },
    OnlyRight { value: String },
}

/// A single field's comparison result, combining its path with the diff kind.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldDiff {
    /// Dot-joined path of the field (e.g. `"Claim.title"`).
    pub path: String,
    /// The kind of difference observed.
    pub kind: FieldDiffKind,
}
```

### Constructor helpers

Add associated functions to reduce boilerplate in `diff()`:

```rust
impl FieldDiff {
    fn equal(path: String, value: String) -> Self {
        Self { path, kind: FieldDiffKind::Equal { value } }
    }
    fn changed(path: String, left: String, right: String) -> Self {
        Self { path, kind: FieldDiffKind::Changed { left, right } }
    }
    fn only_left(path: String, value: String) -> Self {
        Self { path, kind: FieldDiffKind::OnlyLeft { value } }
    }
    fn only_right(path: String, value: String) -> Self {
        Self { path, kind: FieldDiffKind::OnlyRight { value } }
    }
}
```

### `ManifestDiff` helper updates

```rust
impl ManifestDiff {
    pub fn diff_count(&self) -> usize {
        self.fields.iter()
            .filter(|f| !matches!(f.kind, FieldDiffKind::Equal { .. }))
            .count()
    }

    pub fn differences(&self) -> impl Iterator<Item = &FieldDiff> {
        self.fields.iter()
            .filter(|f| !matches!(f.kind, FieldDiffKind::Equal { .. }))
    }
}
```

### Call-site changes

All `match f { FieldDiff::Equal { path, value } => ... }` patterns become
`match f.kind { FieldDiffKind::Equal { value } => ... }` with `f.path`
accessible directly.

Update `ui/compare.rs` render logic and all tests accordingly.

---

## D3 — `AppError::InvalidInput` for filter validation

### Current

`FieldFilter::from_query` constructs fake `glob::PatternError` structs:

```rust
return Err(AppError::Glob(glob::PatternError {
    pos: 0,
    msg: "query exceeds maximum length of 256 characters",
}));
```

These are validation errors, not pattern-compilation errors.

### Fix

Add a new variant to `AppError`:

```rust
#[error("invalid input: {0}")]
InvalidInput(String),
```

Replace all three fake `glob::PatternError` constructions in `from_query`:

```rust
if q.len() > 256 {
    return Err(AppError::InvalidInput(
        "filter query exceeds maximum length of 256 characters".into()
    ));
}
// ...
if token.len() > 128 {
    return Err(AppError::InvalidInput(
        "filter token exceeds maximum length of 128 characters".into()
    ));
}
// ...
if brace_count > 4 {
    return Err(AppError::InvalidInput(
        "filter token contains too many alternation groups (max 4)".into()
    ));
}
```

### Requirements

- The new variant is `AppError::InvalidInput(String)`.
- Tests that previously asserted `Err(AppError::Glob(_))` for these validation
  paths must be updated to `Err(AppError::InvalidInput(_))`.
- Actual glob pattern errors (from `Pattern::new(...)`) continue to produce
  `AppError::Glob(glob::PatternError)` via `#[from]`.

---

## D4 — Decouple error UI text from `AppState::Error`

### Current

```rust
// handle_load_result / handle_filter_key
self.state = AppState::Error {
    message: format!("Error: {e}\n\nPress any key to dismiss."),
};
```

The dismiss prompt is baked in, making `message` unsuitable for logging or
programmatic inspection.

### Fix

Store only the raw error text:

```rust
AppState::Error { message: e.to_string() }
```

Move the UI formatting to the render layer in `ui/mod.rs` (or whichever draw
function renders the error overlay):

```rust
// In the error overlay draw function:
let display = format!("Error: {}\n\nPress any key to dismiss.", app.error_message());
```

Add a helper on `AppState`:

```rust
impl AppState {
    /// Returns the raw error message if the state is `Error`, else `None`.
    pub fn error_message(&self) -> Option<&str> {
        match self {
            AppState::Error { message } => Some(message),
            _ => None,
        }
    }
}
```

### Requirements

- `AppState::Error { message }` stores only the error text (no UI prompt).
- All existing tests that assert on `AppState::Error { message }` must be
  updated to not include the prompt in their expected string.
- The visible UI output is unchanged — the prompt still appears in the overlay.

---

## D5 — Pre-compute comparison values in `diff()`

### Current

```rust
fn comparison_value(display: &str) -> &str {
    display.split_once(": ").map(|(_, val)| val.trim()).unwrap_or_else(|| display.trim())
}

// Called inside the comparison loop:
Some(right_display) if comparison_value(left_display) == comparison_value(right_display) => ...
```

`comparison_value` re-parses the `"key: value"` string on every pair comparison.

### Fix

Pre-split when building the `IndexMap`s:

```rust
let left_map: IndexMap<String, (&str, &str)> = left_flat
    .iter()
    .map(|n| {
        let val = comparison_value(&n.display);
        (n.path.clone(), (n.display.as_str(), val))
    })
    .collect();
```

Then in the comparison loop, access the pre-split value directly:

```rust
for (path, (left_display, left_cmp)) in &left_map {
    match right_map.get(path) {
        Some((right_display, right_cmp)) if left_cmp == right_cmp => {
            fields.push(FieldDiff::equal(path.clone(), left_display.to_string()));
        }
        Some((right_display, _)) => {
            fields.push(FieldDiff::changed(
                path.clone(), left_display.to_string(), right_display.to_string()
            ));
        }
        None => {
            fields.push(FieldDiff::only_left(path.clone(), left_display.to_string()));
        }
    }
}
```

> **Note:** the lifetime constraints may require storing the pre-split value as
> an owned `String` rather than a `&str` slice, depending on how the `IndexMap`
> is structured.  Use `String` for simplicity if lifetime annotations become
> unwieldy.

---

## Testing Strategy

### `filter.rs`

- Existing tests pass unchanged.
- Delete `apply_inner`; update any test helpers that called it directly.
- Add a test confirming `apply` and `apply_ref` return the same result for the
  same input (already covered by proptest, but add an explicit doc-test).

### `diff.rs`

```rust
#[test]
fn field_diff_path_accessible_without_matching_kind() {
    let d = diff("l", &[node("title", "x")], "r", &[node("title", "y")]);
    assert_eq!(d.fields[0].path, "title");
    assert!(matches!(d.fields[0].kind, FieldDiffKind::Changed { .. }));
}

#[test]
fn diff_count_uses_kind() {
    // Verify diff_count works after the refactor.
    let d = diff("l", &[node("a", "1")], "r", &[node("a", "2")]);
    assert_eq!(d.diff_count(), 1);
}
```

### `error.rs`

```rust
#[test]
fn from_query_long_query_returns_invalid_input() {
    let long = "a".repeat(257);
    assert!(matches!(
        FieldFilter::from_query(&long),
        Err(AppError::InvalidInput(_))
    ));
}

#[test]
fn from_query_long_token_returns_invalid_input() {
    let long_token = "a".repeat(129);
    assert!(matches!(
        FieldFilter::from_query(&long_token),
        Err(AppError::InvalidInput(_))
    ));
}
```

### `app.rs`

```rust
#[test]
fn error_state_stores_raw_message_only() {
    let config = Config::default();
    let mut app = App::new(config).unwrap();
    app.handle_load_result(0, Err(AppError::Terminal("disk full".into())));
    match &app.state {
        AppState::Error { message } => {
            assert!(!message.contains("Press any key"),
                "raw message must not include UI prompt");
            assert!(message.contains("disk full"));
        }
        other => panic!("expected Error state, got {other:?}"),
    }
}
```

---

## Edge Cases

- `FieldDiff` is part of the public API (used in `ui/compare.rs`).  The struct
  refactor is a breaking change within the crate.  Because this is a `pub(crate)`
  type (there are no external consumers), the change is safe.
- `AppError::InvalidInput` — ensure `thiserror` derives `Display` properly;
  add a `#[from]` only if there is a suitable source error type (there is not,
  so use the plain tuple form).

---

## Dependencies

No new crate dependencies.

---

## Done criteria

```bash
cargo test -p c2pa-tui -- manifest::filter::tests compare::diff::tests app::tests
cargo clippy -p c2pa-tui -- -D warnings
cargo fmt -p c2pa-tui -- --check
```

All new and updated tests pass.  No existing tests regress.  The `AppError`
variant change must not cause any `non_exhaustive` match warnings.
