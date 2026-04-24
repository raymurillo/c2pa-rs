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
    app.add_source(src.clone());

    let client = RemoteClient::default();
    let nodes = src.load(&client).await.unwrap();
    app.loaded.insert(0, LoadState::Loaded(nodes));

    assert!(matches!(app.loaded.get(&0), Some(LoadState::Loaded(n)) if !n.is_empty()));
}
