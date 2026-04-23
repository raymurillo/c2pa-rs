# Spec 09 — Integration, CLI Polish & Full Test Suite

**Phase:** 3 (sequential — requires all Phase 2 specs merged)  
**Depends on:** all previous specs (00–08)  
**Produces:** complete `main.rs` with clap CLI; full integration test suite;
full snapshot test suite; help overlay; `--theme` color switching.

---

## Goal

Wire everything together into a shippable binary: parse CLI arguments, construct
the `App` from them, populate sources, and hand off to the event loop. Write the
integration tests that exercise real file loading and the wiremock remote tests.
Write snapshot tests for every major UI state.

---

## Files to modify

- `src/main.rs` — full clap CLI
- `src/config.rs` — `Theme` color palette, `Config::from_cli`
- `src/app.rs` — help overlay toggle, `--theme` propagation to draw
- `src/ui/mod.rs` — help overlay rendering
- `tests/integration_loader.rs` — new file
- `tests/integration_remote.rs` — new file (or extend from spec-02)
- `tests/snapshot_ui.rs` — new file (or extend from spec-06/07/08)

---

## `src/main.rs` — full implementation

```rust
use clap::Parser;
use c2pa_tui::{
    app::App,
    config::{Config, Theme},
    manifest::loader::{FileSource, DirSource, RemoteSource},
    remote::Auth,
    error::AppError,
};

#[derive(Parser, Debug)]
#[command(name = "c2pa-tui", version, about = "Terminal UI for C2PA manifests")]
struct Cli {
    /// Files, directories, or HTTP URLs to load on startup.
    #[arg(name = "PATHS_OR_URLS")]
    inputs: Vec<String>,

    /// Authentication spec: none | basic:user:pass | bearer:token | digest:user:pass
    #[arg(long, default_value = "none")]
    auth: String,

    /// Initial field filter glob (e.g. "assertions.*")
    #[arg(long)]
    filter: Option<String>,

    /// Disable mouse support
    #[arg(long)]
    no_mouse: bool,

    /// Color theme: dark | light | mono
    #[arg(long, default_value = "dark")]
    theme: String,
}

fn main() {
    let cli = Cli::parse();

    let auth = Auth::from_spec(&cli.auth).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });

    let theme = match cli.theme.as_str() {
        "light" => Theme::Light,
        "mono"  => Theme::Mono,
        _       => Theme::Dark,
    };

    let config = Config {
        theme,
        mouse_enabled: !cli.no_mouse,
        auth: auth.clone(),
        initial_filter: cli.filter.clone(),
        ..Config::default()
    };

    let mut app = App::new(config).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });

    // Apply initial filter if provided
    if let Some(f) = &cli.filter {
        match c2pa_tui::manifest::filter::FieldFilter::from_query(f) {
            Ok(filter) => app.filter = filter,
            Err(e) => {
                eprintln!("error: invalid filter: {e}");
                std::process::exit(1);
            }
        }
    }

    // Populate sources from CLI inputs
    for input in &cli.inputs {
        if input.starts_with("http://") || input.starts_with("https://") {
            match url::Url::parse(input) {
                Ok(url) => app.add_source(Box::new(RemoteSource::new(url, auth.clone()))),
                Err(e) => {
                    eprintln!("warning: invalid URL {input:?}: {e}");
                }
            }
        } else {
            let path = std::path::PathBuf::from(input);
            if path.is_dir() {
                app.add_source(Box::new(DirSource::new(path)));
            } else {
                app.add_source(Box::new(FileSource::new(path)));
            }
        }
    }

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    if let Err(e) = rt.block_on(app.run()) {
        // Terminal has already been restored inside App::run
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
```

---

## `--theme` propagation

In `src/config.rs`, add a method that returns ratatui `Style`s for each theme:

```rust
use ratatui::style::{Color, Style, Modifier};

impl Theme {
    pub fn border_focused(&self) -> Style {
        match self {
            Theme::Dark  => Style::default().fg(Color::Yellow),
            Theme::Light => Style::default().fg(Color::Blue),
            Theme::Mono  => Style::default().add_modifier(Modifier::BOLD),
        }
    }
    pub fn border_normal(&self) -> Style {
        Style::default()
    }
    pub fn highlight(&self) -> Style {
        match self {
            Theme::Dark  => Style::default().bg(Color::DarkGray),
            Theme::Light => Style::default().bg(Color::Gray),
            Theme::Mono  => Style::default().add_modifier(Modifier::REVERSED),
        }
    }
    pub fn match_highlight(&self) -> Style {
        match self {
            Theme::Dark  => Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
            Theme::Light => Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD),
            Theme::Mono  => Style::default().add_modifier(Modifier::UNDERLINED),
        }
    }
    pub fn diff_changed(&self) -> Style {
        match self {
            Theme::Mono  => Style::default().add_modifier(Modifier::BOLD),
            _            => Style::default().fg(Color::Yellow),
        }
    }
    pub fn diff_only_left(&self) -> Style {
        match self {
            Theme::Mono  => Style::default().add_modifier(Modifier::DIM),
            _            => Style::default().fg(Color::Red),
        }
    }
    pub fn diff_only_right(&self) -> Style {
        match self {
            Theme::Mono  => Style::default().add_modifier(Modifier::ITALIC),
            _            => Style::default().fg(Color::Green),
        }
    }
}
```

Update all widget draw functions in `ui/` to accept `app.config.theme` and call
these methods instead of hardcoding colours. Pass `&app.config.theme` or just
`app` (which already has `config`).

---

## Help overlay

Add to `src/ui/mod.rs` (called from `draw()` when `app.show_help` is true):

```rust
fn draw_help_overlay(frame: &mut Frame, area: Rect) {
    use crate::ui::layout::centered_popup;
    let popup = centered_popup(area, 60, 70);
    let text = vec![
        "Key bindings",
        "",
        "↑/↓ or j/k    Navigate file list",
        "Enter          Load selected file",
        "r              Reload (force re-fetch)",
        "Tab            Switch focus (list ↔ detail)",
        "Space          Expand/collapse tree node",
        "/              Open search bar",
        "f              Open filter bar",
        "c              Mark for compare (press twice on different files)",
        "Esc            Cancel / close overlay",
        "a              (Compare) toggle equal rows",
        "?              Toggle this help",
        "q / Ctrl+C     Quit",
    ];
    frame.render_widget(
        ratatui::widgets::Paragraph::new(text.join("\n"))
            .block(ratatui::widgets::Block::default()
                .borders(ratatui::widgets::Borders::ALL)
                .title("Help")),
        popup,
    );
}
```

Wire in `draw()`:

```rust
if app.show_help {
    draw_help_overlay(frame, frame.area());
}
```

Toggle `show_help` on `?` key press (already added to Browse key handler in spec-05).

---

## `tests/integration_loader.rs`

```rust
use c2pa_tui::manifest::loader::{FileSource, DirSource, ManifestSource};
use c2pa_tui::remote::RemoteClient;

#[tokio::test]
async fn load_signed_jpeg_returns_manifest_tree() {
    let client = RemoteClient::default();
    let src = FileSource::new("tests/fixtures/signed.jpg".into());
    let nodes = src.load(&client).await.expect("load should succeed");
    assert!(!nodes.is_empty(), "signed file should produce nodes");
    assert!(
        nodes.iter().any(|n| n.key.contains("Manifest")),
        "should have at least one Manifest node"
    );
}

#[tokio::test]
async fn load_unsigned_file_returns_informational_node() {
    let client = RemoteClient::default();
    // Use an unsigned fixture — copy any valid JPEG that has no C2PA manifest
    let src = FileSource::new("tests/fixtures/unsigned.jpg".into());
    let nodes = src.load(&client).await.expect("should not error on unsigned file");
    assert_eq!(nodes.len(), 1);
    assert!(
        matches!(&nodes[0].value, c2pa_tui::manifest::tree::NodeValue::Str(s) if s.contains("No C2PA manifest")),
    );
}

#[tokio::test]
async fn unsupported_extension_returns_error() {
    let client = RemoteClient::default();
    let src = FileSource::new("tests/fixtures/document.txt".into());
    let err = src.load(&client).await.unwrap_err();
    assert!(matches!(err, c2pa_tui::error::AppError::UnsupportedFormat(_)));
}

#[tokio::test]
async fn dir_source_discovers_all_supported_files() {
    // tests/fixtures/ should have signed.jpg, unsigned.jpg, signed.png
    let client = RemoteClient::default();
    let src = DirSource::new("tests/fixtures/".into());
    let entries = src.entries().unwrap();
    // At minimum we should find the jpg and png fixtures
    assert!(entries.len() >= 2);
    assert!(entries.iter().any(|e| e.path.extension().map(|x| x == "jpg").unwrap_or(false)));
}

#[tokio::test]
async fn manifest_tree_has_claim_and_assertions_sections() {
    let client = RemoteClient::default();
    let src = FileSource::new("tests/fixtures/signed.jpg".into());
    let nodes = src.load(&client).await.unwrap();
    let manifest = nodes.iter().find(|n| n.key.contains("Manifest")).unwrap();
    assert!(manifest.children.iter().any(|n| n.key == "Claim"), "should have Claim");
    assert!(manifest.children.iter().any(|n| n.key.starts_with("Assertions")), "should have Assertions");
}

#[tokio::test]
async fn manifest_tree_has_validation_section() {
    let client = RemoteClient::default();
    let src = FileSource::new("tests/fixtures/signed.jpg".into());
    let nodes = src.load(&client).await.unwrap();
    let manifest = nodes.iter().find(|n| n.key.contains("Manifest")).unwrap();
    assert!(manifest.children.iter().any(|n| n.key == "Validation"), "should have Validation");
}
```

### Fixtures needed for integration tests

- `tests/fixtures/signed.jpg` — a valid C2PA-signed JPEG
- `tests/fixtures/unsigned.jpg` — a JPEG with no C2PA manifest  
- `tests/fixtures/signed.png` — a valid C2PA-signed PNG
- `tests/fixtures/document.txt` — any text file (unsupported format)

Source signed fixtures from the `c2pa-rs` test suite:
```
sdk/tests/fixtures/
```
Copy relevant files. Create a minimal `unsigned.jpg` by taking any plain JPEG
(e.g. from the internet or using `convert -size 1x1 xc:white unsigned.jpg` with ImageMagick).

---

## `tests/snapshot_ui.rs` — comprehensive snapshots

Write a shared helper module at the top of the file:

```rust
use ratatui::{Terminal, backend::TestBackend};
use c2pa_tui::{app::{App, AppState, Pane}, config::Config, manifest::tree::{DisplayNode, NodeValue}};

fn make_test_terminal(w: u16, h: u16) -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(w, h)).unwrap()
}

fn buffer_to_string(buf: &ratatui::buffer::Buffer) -> String {
    // Convert buffer cells to a multi-line string for snapshotting
    let area = buf.area();
    let mut lines = Vec::new();
    for y in 0..area.height {
        let mut line = String::new();
        for x in 0..area.width {
            line.push_str(buf[(x, y)].symbol());
        }
        lines.push(line.trim_end().to_string());
    }
    lines.join("\n")
}

fn make_sample_nodes() -> Vec<DisplayNode> {
    vec![
        DisplayNode {
            key: "Manifest (active)".into(),
            value: NodeValue::Missing,
            children: vec![
                DisplayNode {
                    key: "Claim".into(),
                    value: NodeValue::Missing,
                    children: vec![
                        DisplayNode { key: "title".into(), value: NodeValue::Str("My Photo".into()), children: vec![] },
                        DisplayNode { key: "format".into(), value: NodeValue::Str("image/jpeg".into()), children: vec![] },
                    ],
                },
                DisplayNode {
                    key: "Assertions (1)".into(),
                    value: NodeValue::Missing,
                    children: vec![
                        DisplayNode { key: "c2pa.actions".into(), value: NodeValue::Json(serde_json::json!({"action":"c2pa.created"})), children: vec![] },
                    ],
                },
                DisplayNode {
                    key: "Validation".into(),
                    value: NodeValue::Missing,
                    children: vec![
                        DisplayNode { key: "status".into(), value: NodeValue::Str("valid".into()), children: vec![] },
                    ],
                },
            ],
        }
    ]
}

// Use the mockall-generated mock rather than a hand-rolled fake.
use c2pa_tui::manifest::loader::MockManifestSource;

fn make_app_with_loaded_manifest() -> App {
    let mut app = App::new(Config::default()).unwrap();
    let nodes = make_sample_nodes();
    let mut mock = MockManifestSource::new();
    mock.expect_label().return_const("test.jpg".to_string());
    mock.expect_is_remote().return_const(false);
    mock.expect_load().return_once({
        let n = nodes.clone();
        move |_| Ok(n)
    });
    app.add_source(Box::new(mock));
    app.loaded.insert(0, nodes);
    app
}
```

### Snapshot test cases

Cover every distinct UI state:

1. **Empty state** — no sources loaded, empty file list, empty detail pane
2. **File list, one item, unloaded** — source added but not yet loaded
3. **File list, one item, loaded** — checkmark shown, detail pane shows tree
4. **Detail pane expanded** — all top-level nodes expanded
5. **Detail pane with filter** — only assertions visible
6. **Search bar, empty query** — all nodes shown in results
7. **Search bar, active query** — matching nodes highlighted
8. **Filter bar, valid glob** — preview shows surviving sections
9. **Filter bar, invalid glob** — error message shown
10. **Compare view, differences** — changed/only-left/only-right rows visible
11. **Compare view, show all** — equal rows also visible (dimmed)
12. **Compare view, no diff loaded** — placeholder text
13. **Error overlay** — error message centered over TUI
14. **Help overlay** — full key binding list visible
15. **Light theme** — same as #3 but with light theme colours
16. **Mono theme** — same as #3 but with mono theme

---

## Done criteria

```
cargo build
cargo test
cargo test --test integration_loader
cargo test --test snapshot_ui
cargo fmt -- --check
cargo clippy -- -D warnings
cargo tarpaulin --out Html           # coverage report (target: ≥60% line coverage)
```

Verify all documented CLI flags work end-to-end:

```sh
cargo run -- --help
cargo run -- tests/fixtures/signed.jpg
cargo run -- --theme light tests/fixtures/signed.jpg
cargo run -- --filter "assertions.*" tests/fixtures/signed.jpg
cargo run -- --auth bearer:mytoken https://example.com/asset.jpg
cargo run -- --no-mouse tests/fixtures/
```

Verify the binary accepts all documented CLI flags:

```sh
cargo run -- --help
cargo run -- tests/fixtures/signed.jpg
cargo run -- --theme light tests/fixtures/signed.jpg
cargo run -- --filter "assertions.*" tests/fixtures/signed.jpg
cargo run -- --auth bearer:mytoken https://example.com/asset.jpg
cargo run -- --no-mouse tests/fixtures/
```
