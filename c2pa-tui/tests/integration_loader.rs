use c2pa_tui::error::AppError;
use c2pa_tui::manifest::loader::{DirSource, FileSource, ManifestSource};
use c2pa_tui::manifest::tree::NodeValue;
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
    let src = FileSource::new("tests/fixtures/unsigned.jpg".into());
    let nodes = src
        .load(&client)
        .await
        .expect("should not error on unsigned file");
    assert_eq!(nodes.len(), 1);
    assert!(
        matches!(&nodes[0].value, NodeValue::Str(s) if s.contains("No C2PA manifest")),
        "expected 'No C2PA manifest' node, got: {:?}",
        nodes[0].value
    );
}

#[tokio::test]
async fn unsupported_extension_returns_error() {
    let client = RemoteClient::default();
    let src = FileSource::new("tests/fixtures/document.txt".into());
    let err = src.load(&client).await.unwrap_err();
    assert!(
        matches!(err, AppError::UnsupportedFormat(_)),
        "expected UnsupportedFormat, got: {err:?}"
    );
}

#[tokio::test]
async fn dir_source_discovers_all_supported_files() {
    let src = DirSource::new("tests/fixtures/".into());
    let entries = src.entries().unwrap();
    // fixtures/ contains at minimum: signed.jpg, unsigned.jpg, signed.png, C.jpg, sample1.png
    assert!(
        entries.len() >= 2,
        "expected at least 2 supported files, found {}",
        entries.len()
    );
    assert!(
        entries
            .iter()
            .any(|e| e.path.extension().map(|x| x == "jpg").unwrap_or(false)),
        "should discover at least one .jpg file"
    );
}

#[tokio::test]
async fn manifest_tree_has_claim_and_assertions_sections() {
    let client = RemoteClient::default();
    let src = FileSource::new("tests/fixtures/signed.jpg".into());
    let nodes = src.load(&client).await.unwrap();
    let manifest = nodes.iter().find(|n| n.key.contains("Manifest")).unwrap();
    assert!(
        manifest.children.iter().any(|n| n.key == "Claim"),
        "should have Claim child, children: {:?}",
        manifest.children.iter().map(|c| &c.key).collect::<Vec<_>>()
    );
    assert!(
        manifest
            .children
            .iter()
            .any(|n| n.key.starts_with("Assertions")),
        "should have Assertions child"
    );
}

#[tokio::test]
async fn manifest_tree_has_validation_section() {
    let client = RemoteClient::default();
    let src = FileSource::new("tests/fixtures/signed.jpg".into());
    let nodes = src.load(&client).await.unwrap();
    let manifest = nodes.iter().find(|n| n.key.contains("Manifest")).unwrap();
    assert!(
        manifest.children.iter().any(|n| n.key == "Validation"),
        "should have Validation child"
    );
}

/// Verify that an App can be constructed and sources added programmatically.
#[test]
fn app_add_source_increments_count() {
    use c2pa_tui::app::App;
    use c2pa_tui::config::Config;
    use std::sync::Arc;

    let mut app = App::new(Config::default()).unwrap();
    assert_eq!(app.sources.len(), 0);
    app.add_source(Arc::new(FileSource::new(
        "tests/fixtures/signed.jpg".into(),
    )));
    assert_eq!(app.sources.len(), 1);
}

/// Verify that loading a source populates the loaded map correctly.
#[tokio::test]
async fn loaded_map_contains_nodes_after_load() {
    use c2pa_tui::app::{App, LoadState};
    use c2pa_tui::config::Config;
    use std::sync::Arc;

    let mut app = App::new(Config::default()).unwrap();
    let src = Arc::new(FileSource::new("tests/fixtures/signed.jpg".into()));
    let id = app.add_source(src.clone());

    let client = RemoteClient::default();
    let nodes = src.load(&client).await.unwrap();
    app.loaded.insert(id, LoadState::Loaded(nodes));

    assert!(matches!(app.loaded.get(&id), Some(LoadState::Loaded(n)) if !n.is_empty()));
}

/// Verify that `App::add_dir` expands a directory into individual sources,
/// each with a unique `SourceId`.
#[tokio::test]
async fn add_dir_creates_individual_sources() {
    use c2pa_tui::app::App;
    use c2pa_tui::config::Config;

    let mut app = App::new(Config::default()).unwrap();
    let ids = app
        .add_dir("tests/fixtures/".into())
        .await
        .expect("add_dir should succeed");
    assert!(
        ids.len() >= 2,
        "directory should expand to multiple sources, got {}",
        ids.len()
    );
    assert_eq!(app.sources.len(), ids.len());
    let id_set: std::collections::HashSet<_> = ids.iter().copied().collect();
    assert_eq!(id_set.len(), ids.len(), "each SourceId must be unique");
    // The first add_source call must initialise `selected_left` to id[0].
    assert_eq!(app.selected_left, Some(ids[0]));
}

/// Empty directory: `add_dir` must succeed with no registered sources and
/// leave `selected_left` unset so the TUI renders an empty file list.
#[tokio::test]
async fn add_dir_empty_directory_returns_no_ids() {
    use c2pa_tui::app::App;
    use c2pa_tui::config::Config;
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    let mut app = App::new(Config::default()).unwrap();
    let ids = app
        .add_dir(tmp.path().to_path_buf())
        .await
        .expect("empty dir should not error");
    assert!(ids.is_empty(), "expected no ids, got {ids:?}");
    assert!(app.sources.is_empty());
    assert!(app.selected_left.is_none());
}

/// Directory containing only unsupported files: behaves identically to an
/// empty directory — no sources registered.
#[tokio::test]
async fn add_dir_skips_unsupported_extensions() {
    use c2pa_tui::app::App;
    use c2pa_tui::config::Config;
    use std::fs;
    use tempfile::TempDir;

    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("readme.txt"), b"hello").unwrap();
    fs::write(tmp.path().join("notes.md"), b"world").unwrap();

    let mut app = App::new(Config::default()).unwrap();
    let ids = app.add_dir(tmp.path().to_path_buf()).await.unwrap();
    assert!(ids.is_empty());
    assert!(app.sources.is_empty());
}

/// Non-existent path: `add_dir` must propagate the walkdir error rather
/// than silently succeed, so `main.rs` can surface a warning to the user.
#[tokio::test]
async fn add_dir_nonexistent_path_returns_error() {
    use c2pa_tui::app::App;
    use c2pa_tui::config::Config;

    let mut app = App::new(Config::default()).unwrap();
    let result = app
        .add_dir("/definitely/does/not/exist/c2pa-tui-test".into())
        .await;
    assert!(result.is_err(), "expected Err for missing path");
    // Source list must remain unchanged on error.
    assert!(app.sources.is_empty());
    assert!(app.selected_left.is_none());
}
