# Spec 12 — Test Coverage Gaps

**Phase:** 4 (sequential — requires spec-10 and spec-11 merged and `cargo build` clean)  
**Depends on:** spec-10, spec-11  
**Produces:** state-machine transition tests; async `DirSource` behaviour tests;
HTTP status mapping tests; C2PA error-variant mapping tests; `RemoteClient::default`
regression guard; comprehensive snapshot suite.

---

## Goal

Fill the six test coverage gaps identified in the architecture review.  The only
production code change in this spec is in `src/remote/client.rs` where a test for
the `is_timeout()` retry path validates the fix added in spec-10 A4.  Everything
else is pure test additions.

---

## Files to modify

- `src/app.rs` — add `#[cfg(test)]` state-transition tests
- `src/manifest/loader.rs` — add async `DirSource` behaviour tests
- `src/remote/client.rs` — add HTTP status and retry tests
- `tests/snapshot_ui.rs` — extend to cover all major UI states

No changes to `src/ui/`, `src/compare/`, or `src/search/`.

---

## C1 — `App` state machine transition tests

Add a `#[cfg(test)]` block inside `src/app.rs` that drives the key-event handlers
directly and asserts `App::state` after each transition.  Because these tests live
inside `src/app.rs`, they can access `pub(crate)` fields directly (spec-13 will
narrow visibility, but the `#[cfg(test)]` context remains inside the crate).

| Test name | Setup | Key event | Expected outcome |
|-----------|-------|-----------|-----------------|
| `browse_to_searching_on_slash` | Browse | `/` | `state == Searching { query: "" }` |
| `searching_to_browse_on_esc` | Searching | `Esc` | `state == Browse` |
| `browse_to_filtering_on_f` | Browse | `f` | `state == Filtering { query: "" }` |
| `filtering_to_browse_on_esc` | Filtering | `Esc` | `state == Browse` |
| `browse_to_comparing_second_c` | Browse, two sources loaded | `c` twice | `state == Comparing` |
| `comparing_to_browse_on_esc` | Comparing | `Esc` | `state == Browse`, `compare_selection == None` |
| `error_to_browse_on_any_key` | `Error { message }` | any key | `state == Browse` |
| `help_toggle_on_question_mark` | Browse | `?` | `show_help == true`; second `?` → `show_help == false` |

```rust
#[cfg(test)]
mod state_tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use crate::config::Config;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn make_app() -> App {
        App::new(Config::default()).unwrap()
    }

    #[test]
    fn browse_to_searching_on_slash() {
        let mut app = make_app();
        assert_eq!(app.state, AppState::Browse);
        app.handle_browse_key(key(KeyCode::Char('/')));
        assert_eq!(app.state, AppState::Searching { query: String::new() });
    }

    #[test]
    fn searching_to_browse_on_esc() {
        let mut app = make_app();
        app.state = AppState::Searching { query: "foo".into() };
        app.handle_search_key(key(KeyCode::Esc));
        assert_eq!(app.state, AppState::Browse);
    }

    #[test]
    fn help_toggle_on_question_mark() {
        let mut app = make_app();
        assert!(!app.show_help);
        app.handle_browse_key(key(KeyCode::Char('?')));
        assert!(app.show_help);
        app.handle_browse_key(key(KeyCode::Char('?')));
        assert!(!app.show_help);
    }

    // … (implement remaining rows from the table above)
}
```

---

## C2 — `DirSource::entries_async()` behaviour tests

Add `#[tokio::test]` cases inside `src/manifest/loader.rs`.  These tests require
spec-11's `entries_async()` to exist.

```rust
#[tokio::test]
async fn entries_async_empty_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = DirSource::new(tmp.path().into());
    let entries = dir.entries_async().await.unwrap();
    assert!(entries.is_empty());
}

#[tokio::test]
async fn entries_async_skips_unsupported_extensions() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("readme.txt"), b"hello").unwrap();
    std::fs::write(tmp.path().join("photo.jpg"), b"\xff\xd8\xff").unwrap();
    let dir = DirSource::new(tmp.path().into());
    let entries = dir.entries_async().await.unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path.extension().unwrap(), "jpg");
}

#[tokio::test]
async fn entries_async_returns_sorted_paths() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("b.jpg"), b"\xff\xd8\xff").unwrap();
    std::fs::write(tmp.path().join("a.jpg"), b"\xff\xd8\xff").unwrap();
    let dir = DirSource::new(tmp.path().into());
    let entries = dir.entries_async().await.unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries[0].path < entries[1].path);
}
```

---

## C3 — `RemoteClient::fetch()` HTTP status mapping and retry

### What wiremock can and cannot test

`wiremock` always accepts the TCP connection and returns HTTP responses — it cannot
simulate transport-level failures such as a reset connection mid-handshake.  The
retry logic in `fetch` fires on `e.is_connect()` and `e.is_timeout()` (the latter
added in spec-10 A4), which are `reqwest::Error` predicates on network-layer
errors, not HTTP status codes.

**Consequence:** a wiremock-based test cannot exercise the retry branch.  The
retry logic is best covered by either:
- An integration test using a real server that closes the connection (out of scope
  for this spec), or
- A unit test that swaps in a fake HTTP sender via a trait seam (a larger
  refactor also out of scope).

**What wiremock tests cover here:** that `fetch` correctly maps HTTP status codes
to `AppError` variants.  These are the tests that matter for user-visible behaviour.

### Tests to add in `src/remote/client.rs`

```rust
#[tokio::test]
async fn fetch_returns_bytes_on_200() {
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::method;

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"\xff\xd8\xff"))
        .mount(&server)
        .await;

    let client = RemoteClient::new().unwrap();
    let url = url::Url::parse(&format!("{}/asset.jpg", server.uri())).unwrap();
    let bytes = client.fetch(&url, &Auth::None).await.unwrap();
    assert!(!bytes.is_empty());
}

#[tokio::test]
async fn fetch_returns_no_manifest_on_404() {
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::method;

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let client = RemoteClient::new().unwrap();
    let url = url::Url::parse(&format!("{}/missing", server.uri())).unwrap();
    let err = client.fetch(&url, &Auth::None).await.unwrap_err();
    assert!(matches!(err, AppError::NoManifest(_)));
}

#[tokio::test]
async fn fetch_returns_auth_error_on_401() {
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::method;

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let client = RemoteClient::new().unwrap();
    let url = url::Url::parse(&format!("{}/protected", server.uri())).unwrap();
    let err = client.fetch(&url, &Auth::None).await.unwrap_err();
    assert!(matches!(err, AppError::Auth(_)));
}

#[tokio::test]
async fn fetch_returns_auth_error_on_403() {
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::method;

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let client = RemoteClient::new().unwrap();
    let url = url::Url::parse(&format!("{}/forbidden", server.uri())).unwrap();
    let err = client.fetch(&url, &Auth::None).await.unwrap_err();
    assert!(matches!(err, AppError::Auth(_)));
}

#[tokio::test]
async fn fetch_returns_http_error_on_500() {
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::method;

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let client = RemoteClient::new().unwrap();
    let url = url::Url::parse(&format!("{}/broken", server.uri())).unwrap();
    let err = client.fetch(&url, &Auth::None).await.unwrap_err();
    assert!(matches!(err, AppError::Http(_)));
}
```

---

## C4 — `FileSource::load()` C2PA error-variant mapping

`FileSource::load()` converts `c2pa::Error::JumbfNotFound` and
`c2pa::Error::ProvenanceMissing` into informational `DisplayNode`s rather than
propagating errors.  Add tests inside `src/manifest/loader.rs`:

```rust
#[tokio::test]
async fn load_unsigned_file_returns_informational_node() {
    let client = RemoteClient::default();
    let src = FileSource::new("tests/fixtures/unsigned.jpg".into());
    let nodes = src.load(&client).await.expect("should not error on unsigned file");
    assert_eq!(nodes.len(), 1);
    let node = &nodes[0];
    assert_eq!(node.key, "status");
    let msg = match &node.value {
        crate::manifest::tree::NodeValue::Str(s) => s.clone(),
        other => panic!("expected Str, got {other:?}"),
    };
    assert!(
        msg.to_lowercase().contains("no c2pa manifest"),
        "message was: {msg}"
    );
}

#[tokio::test]
async fn load_unsupported_extension_returns_error() {
    let client = RemoteClient::default();
    let src = FileSource::new("tests/fixtures/document.txt".into());
    let err = src.load(&client).await.unwrap_err();
    assert!(matches!(err, AppError::UnsupportedFormat(_)));
}
```

---

## C5 — `RemoteClient::default()` regression guard

```rust
#[test]
fn default_constructs_without_panic() {
    // Guards against Default reverting to raw reqwest::Client::new(),
    // which would lose the 30s timeout, connect_timeout, and user-agent.
    let client = RemoteClient::default();
    let _ = client.client();
}
```

---

## C6 — Comprehensive snapshot tests (`tests/snapshot_ui.rs`)

Use `ratatui::backend::TestBackend` and `insta::assert_snapshot!` for
golden-file comparison against a fixed 80×24 terminal.

### Critical: use `App::with_loaded_for_tests` for state injection

**Do not write directly to `app.loaded` or other fields.** Spec-13 (D2) will
narrow all `App` fields to `pub(crate)`, which would break any integration test
that directly accesses fields.  `App::with_loaded_for_tests` (added in spec-11)
is the supported injection path for external tests.

```rust
// BAD — will break when spec-13 narrows visibility:
app.loaded.insert(some_id, LoadState::Loaded(nodes));

// GOOD — works after spec-13:
let app = App::with_loaded_for_tests(Config::default(), "test.jpg", sample_nodes());
```

For tests that need to set state fields (`app.state`, `app.show_help`, etc.),
either:
- Move the test into `src/app.rs` as a `#[cfg(test)]` module (full `pub(crate)`
  access), or
- Add a purpose-built constructor variant (`App::with_state_for_tests`) using
  the same `#[cfg(test)]` impl block added in spec-11.

### Shared helpers

```rust
use ratatui::{Terminal, backend::TestBackend};
use c2pa_tui::{
    app::{App, AppState},
    config::{Config, Theme},
    manifest::tree::{DisplayNode, NodeValue},
};

fn make_terminal(w: u16, h: u16) -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(w, h)).unwrap()
}

fn buffer_to_string(buf: &ratatui::buffer::Buffer) -> String {
    let area = buf.area();
    (0..area.height)
        .map(|y| {
            let line: String = (0..area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect();
            line.trim_end().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn sample_nodes() -> Vec<DisplayNode> {
    vec![DisplayNode {
        key: "Manifest (active)".into(),
        value: NodeValue::Missing,
        children: vec![
            DisplayNode {
                key: "Claim".into(),
                value: NodeValue::Missing,
                children: vec![
                    DisplayNode {
                        key: "title".into(),
                        value: NodeValue::Str("My Photo".into()),
                        children: vec![],
                    },
                    DisplayNode {
                        key: "format".into(),
                        value: NodeValue::Str("image/jpeg".into()),
                        children: vec![],
                    },
                ],
            },
            DisplayNode {
                key: "Assertions (1)".into(),
                value: NodeValue::Missing,
                children: vec![DisplayNode {
                    key: "c2pa.actions".into(),
                    value: NodeValue::Json(serde_json::json!({"action": "c2pa.created"})),
                    children: vec![],
                }],
            },
            DisplayNode {
                key: "Validation".into(),
                value: NodeValue::Missing,
                children: vec![DisplayNode {
                    key: "status".into(),
                    value: NodeValue::Str("valid".into()),
                    children: vec![],
                }],
            },
        ],
    }]
}
```

### Test cases required

| Test name | State/config | What to verify |
|-----------|-------------|----------------|
| `snapshot_empty_app` | No sources, Browse | empty file list, empty detail pane |
| `snapshot_source_loading` | One source in `LoadState::Loading` | `[~]` icon in file list |
| `snapshot_source_loaded` | One source loaded, `with_loaded_for_tests` | `[✓]` icon, detail tree visible |
| `snapshot_source_error` | `AppState::Error { message }` | error overlay centred |
| `snapshot_detail_expanded` | Loaded, all tree nodes expanded | full tree visible |
| `snapshot_detail_with_filter` | Filter `assertions.*` applied | only assertion nodes shown |
| `snapshot_search_bar_empty` | `Searching { query: "" }` | overlay, empty results |
| `snapshot_search_bar_active` | `Searching { query: "title" }` | matching nodes highlighted |
| `snapshot_filter_bar_valid_glob` | `Filtering { query: "assertions.*" }` | preview shows surviving sections |
| `snapshot_compare_with_diffs` | `Comparing`, diff with `Changed` and `OnlyLeft` | colour-coded rows |
| `snapshot_compare_show_all` | `Comparing`, `show_all_diffs: true` | equal rows rendered |
| `snapshot_compare_no_diff` | `Comparing`, no second source | placeholder text |
| `snapshot_help_overlay` | `show_help: true` | key binding list visible |
| `snapshot_light_theme` | `Theme::Light`, one source loaded | light-palette border/highlight colours |
| `snapshot_mono_theme` | `Theme::Mono`, one source loaded | no colour, bold/reverse/underline only |
| `snapshot_remote_source_label` | Remote source label | `(remote)` suffix in file list |

---

## Done criteria

```
cargo build
cargo test
cargo test --test snapshot_ui
cargo fmt -- --check
cargo clippy -- -D warnings
cargo tarpaulin --out Html   # target: ≥70% line coverage
```

Run `cargo insta review` to accept new snapshots, then commit the
`tests/snapshots/` directory.
