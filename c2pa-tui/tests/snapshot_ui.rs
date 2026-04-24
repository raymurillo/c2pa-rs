use async_trait::async_trait;
use c2pa_tui::app::{App, LoadState};
use c2pa_tui::config::Config;
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
