use async_trait::async_trait;
use c2pa_tui::app::{App, AppState, LoadState};
use c2pa_tui::config::{Config, Theme};
use c2pa_tui::manifest::loader::ManifestSource;
use c2pa_tui::manifest::tree::{DisplayNode, NodeValue};
use c2pa_tui::remote::RemoteClient;
use ratatui::{backend::TestBackend, Terminal};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

struct TestSource {
    label: String,
    remote: bool,
}

impl TestSource {
    fn new(label: &str) -> Arc<Self> {
        Arc::new(Self {
            label: label.to_owned(),
            remote: false,
        })
    }

    fn remote(label: &str) -> Arc<Self> {
        Arc::new(Self {
            label: label.to_owned(),
            remote: true,
        })
    }
}

#[async_trait]
impl ManifestSource for TestSource {
    fn label(&self) -> &str {
        &self.label
    }

    fn is_remote(&self) -> bool {
        self.remote
    }

    async fn load(&self, _client: &RemoteClient) -> c2pa_tui::error::Result<Vec<DisplayNode>> {
        Ok(vec![])
    }
}

fn make_test_terminal(width: u16, height: u16) -> Terminal<TestBackend> {
    Terminal::new(TestBackend::new(width, height)).unwrap()
}

fn buffer_to_string(buffer: &ratatui::buffer::Buffer) -> String {
    let area = buffer.area();
    (0..area.height)
        .map(|y| {
            let row: String = (0..area.width)
                .map(|x| {
                    buffer
                        .cell((x, y))
                        .map(|c| c.symbol().to_string())
                        .unwrap_or_default()
                })
                .collect();
            row.trim_end().to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn make_leaf(key: &str, value: &str) -> DisplayNode {
    DisplayNode {
        key: key.to_owned(),
        value: NodeValue::Str(value.to_owned()),
        children: vec![],
    }
}

fn make_branch(key: &str, children: Vec<DisplayNode>) -> DisplayNode {
    DisplayNode {
        key: key.to_owned(),
        value: NodeValue::Missing,
        children,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn empty_app_renders_without_panic() {
    let mut app = App::new(Config::default()).unwrap();
    let mut terminal = make_test_terminal(80, 24);
    terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
    let content = buffer_to_string(terminal.backend().buffer());
    assert!(content.contains("Files"), "should render Files header");
}

#[test]
fn file_list_renders_single_item() {
    let mut app = App::new(Config::default()).unwrap();
    app.add_source(TestSource::new("test.jpg"));

    let mut terminal = make_test_terminal(80, 24);
    terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
    let content = buffer_to_string(terminal.backend().buffer());

    assert!(content.contains("test.jpg"), "file label should appear");
    assert!(
        content.contains("[ ]"),
        "unloaded file should show empty icon"
    );
}

#[test]
fn file_list_shows_loading_indicator() {
    let mut app = App::new(Config::default()).unwrap();
    app.add_source(TestSource::new("loading.jpg"));
    app.loaded.insert(0, LoadState::Loading);
    app.loading_count = 1;

    let mut terminal = make_test_terminal(80, 24);
    terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
    let content = buffer_to_string(terminal.backend().buffer());

    assert!(
        content.contains("[~]"),
        "loading file should show tilde icon"
    );
}

#[test]
fn file_list_shows_loaded_indicator() {
    let mut app = App::new(Config::default()).unwrap();
    app.add_source(TestSource::new("loaded.jpg"));
    app.loaded.insert(0, LoadState::Loaded(vec![]));

    let mut terminal = make_test_terminal(80, 24);
    terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
    let content = buffer_to_string(terminal.backend().buffer());

    assert!(
        content.contains("[✓]"),
        "loaded file should show check icon"
    );
}

#[test]
fn file_list_shows_remote_suffix() {
    let mut app = App::new(Config::default()).unwrap();
    // Use a short label so it fits in the left pane (25% of 120 = 30 cols)
    app.add_source(TestSource::remote("img.jpg"));

    let mut terminal = make_test_terminal(120, 24);
    terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
    let content = buffer_to_string(terminal.backend().buffer());

    assert!(
        content.contains("(remote)"),
        "remote source should show (remote) suffix"
    );
}

#[test]
fn detail_pane_renders_loaded_tree() {
    let mut app = App::new(Config::default()).unwrap();
    app.add_source(TestSource::new("asset.jpg"));

    let nodes = vec![make_branch("Claim", vec![make_leaf("title", "My Photo")])];
    app.loaded.insert(0, LoadState::Loaded(nodes));

    let mut terminal = make_test_terminal(80, 24);
    terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
    let content = buffer_to_string(terminal.backend().buffer());

    assert!(
        content.contains("Claim"),
        "detail pane should show Claim node"
    );
}

#[test]
fn status_bar_shows_browse_hints() {
    let mut app = App::new(Config::default()).unwrap();
    let mut terminal = make_test_terminal(120, 24);
    terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
    let content = buffer_to_string(terminal.backend().buffer());

    assert!(
        content.contains("q:quit"),
        "status bar should show quit hint"
    );
    assert!(
        content.contains("Enter:load"),
        "status bar should show load hint"
    );
}

#[test]
fn status_bar_shows_loading_message_when_loading() {
    let mut app = App::new(Config::default()).unwrap();
    app.add_source(TestSource::new("loading.jpg"));
    app.loaded.insert(0, LoadState::Loading);
    app.loading_count = 1;

    let mut terminal = make_test_terminal(80, 24);
    terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
    let content = buffer_to_string(terminal.backend().buffer());

    assert!(
        content.contains("Loading"),
        "status bar should show loading message"
    );
}

#[test]
fn detail_pane_shows_source_label_as_title() {
    let mut app = App::new(Config::default()).unwrap();
    app.add_source(TestSource::new("photo.jpg"));
    app.loaded.insert(0, LoadState::Loaded(vec![]));

    let mut terminal = make_test_terminal(80, 24);
    terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
    let content = buffer_to_string(terminal.backend().buffer());

    assert!(
        content.contains("photo.jpg"),
        "detail pane border should show the source label"
    );
}

// ---------------------------------------------------------------------------
// Spec-07 helpers and tests
// ---------------------------------------------------------------------------

fn make_app_with_loaded_manifest() -> App {
    let mut app = App::new(Config::default()).unwrap();
    app.add_source(TestSource::new("test.jpg"));
    let nodes = vec![
        make_branch(
            "Claim",
            vec![
                make_leaf("format", "image/jpeg"),
                make_leaf("title", "Test Photo"),
            ],
        ),
        make_branch(
            "Assertions",
            vec![make_branch(
                "c2pa.hash.data",
                vec![make_leaf("alg", "sha256")],
            )],
        ),
    ];
    app.loaded.insert(0, LoadState::Loaded(nodes));
    // Prime the search index so reindex_and_search() only needs to query.
    app.reindex_for_selected();
    app
}

#[test]
fn search_bar_renders_with_query() {
    let mut app = make_app_with_loaded_manifest();
    app.state = AppState::Searching {
        query: "jpeg".into(),
    };
    app.reindex_and_search();
    let mut terminal = make_test_terminal(100, 30);
    terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
    let content = buffer_to_string(terminal.backend().buffer());
    insta::assert_snapshot!(content);
}

#[test]
fn filter_bar_renders_preview() {
    let mut app = make_app_with_loaded_manifest();
    app.state = AppState::Filtering {
        query: "Assertions.*".into(),
    };
    let mut terminal = make_test_terminal(100, 30);
    terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
    let content = buffer_to_string(terminal.backend().buffer());
    insta::assert_snapshot!(content);
}

#[test]
fn filter_bar_shows_error_for_invalid_glob() {
    let mut app = make_app_with_loaded_manifest();
    app.state = AppState::Filtering {
        query: "[invalid".into(),
    };
    let mut terminal = make_test_terminal(100, 30);
    terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
    let content = buffer_to_string(terminal.backend().buffer());
    insta::assert_snapshot!(content);
}

// ---------------------------------------------------------------------------
// Spec-09 snapshot tests
// ---------------------------------------------------------------------------

fn make_compare_app() -> App {
    let mut app = make_app_with_loaded_manifest();

    // Add a second source with slightly different content.
    app.add_source(TestSource::new("other.jpg"));
    let nodes_right = vec![
        make_branch(
            "Claim",
            vec![
                make_leaf("format", "image/png"),
                make_leaf("title", "Other Photo"),
            ],
        ),
        make_branch(
            "Assertions",
            vec![make_branch(
                "c2pa.hash.data",
                vec![make_leaf("alg", "sha512")],
            )],
        ),
    ];
    app.loaded.insert(1, LoadState::Loaded(nodes_right));
    app.compare_selection = Some(1);
    app.state = AppState::Comparing;
    app
}

#[test]
fn compare_view_shows_differences() {
    let mut app = make_compare_app();
    app.show_all_diffs = false;
    let mut terminal = make_test_terminal(120, 30);
    terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
    let content = buffer_to_string(terminal.backend().buffer());
    insta::assert_snapshot!(content);
}

#[test]
fn compare_view_show_all_includes_equal_rows() {
    let mut app = make_compare_app();
    app.show_all_diffs = true;
    let mut terminal = make_test_terminal(120, 30);
    terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
    let content = buffer_to_string(terminal.backend().buffer());
    insta::assert_snapshot!(content);
}

#[test]
fn compare_view_no_diff_loaded_shows_placeholder() {
    let mut app = App::new(Config::default()).unwrap();
    app.add_source(TestSource::new("a.jpg"));
    app.add_source(TestSource::new("b.jpg"));
    // Neither source has been loaded — compare_selection set without loaded data.
    app.compare_selection = Some(1);
    app.state = AppState::Comparing;

    let mut terminal = make_test_terminal(100, 24);
    terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
    let content = buffer_to_string(terminal.backend().buffer());
    assert!(
        content.contains("not loaded") || content.contains("Select") || content.contains("load"),
        "should show placeholder when no manifests loaded"
    );
    insta::assert_snapshot!(content);
}

#[test]
fn error_overlay_is_rendered_centered() {
    let mut app = App::new(Config::default()).unwrap();
    app.state = AppState::Error {
        message: "Something went wrong\n\nPress any key to dismiss.".into(),
    };

    let mut terminal = make_test_terminal(100, 24);
    terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
    let content = buffer_to_string(terminal.backend().buffer());
    assert!(
        content.contains("Error"),
        "should render Error border title"
    );
    assert!(
        content.contains("Something went wrong"),
        "should render error message"
    );
    insta::assert_snapshot!(content);
}

#[test]
fn help_overlay_shows_key_bindings() {
    let mut app = App::new(Config::default()).unwrap();
    app.show_help = true;

    let mut terminal = make_test_terminal(100, 30);
    terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
    let content = buffer_to_string(terminal.backend().buffer());
    assert!(content.contains("Help"), "should show Help title");
    assert!(
        content.contains("Key bindings"),
        "should show key bindings header"
    );
    assert!(content.contains("q / Ctrl+C"), "should show quit binding");
    insta::assert_snapshot!(content);
}

#[test]
fn light_theme_renders_loaded_manifest() {
    let config = Config {
        theme: Theme::Light,
        ..Config::default()
    };
    let mut app = App::new(config).unwrap();
    app.add_source(TestSource::new("photo.jpg"));
    let nodes = vec![make_branch("Claim", vec![make_leaf("title", "My Photo")])];
    app.loaded.insert(0, LoadState::Loaded(nodes));

    let mut terminal = make_test_terminal(80, 24);
    terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
    let content = buffer_to_string(terminal.backend().buffer());
    assert!(content.contains("photo.jpg"), "should show filename");
    insta::assert_snapshot!(content);
}

#[test]
fn mono_theme_renders_loaded_manifest() {
    let config = Config {
        theme: Theme::Mono,
        ..Config::default()
    };
    let mut app = App::new(config).unwrap();
    app.add_source(TestSource::new("photo.jpg"));
    let nodes = vec![make_branch("Claim", vec![make_leaf("title", "My Photo")])];
    app.loaded.insert(0, LoadState::Loaded(nodes));

    let mut terminal = make_test_terminal(80, 24);
    terminal.draw(|f| c2pa_tui::ui::draw(f, &mut app)).unwrap();
    let content = buffer_to_string(terminal.backend().buffer());
    assert!(content.contains("photo.jpg"), "should show filename");
    insta::assert_snapshot!(content);
}
