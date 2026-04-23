use c2pa_tui::error::AppError;
use c2pa_tui::manifest::loader::{ManifestSource, RemoteSource};
use c2pa_tui::remote::{Auth, RemoteClient};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Requires spec-01 FileSource::load (store_to_nodes) to be implemented.
#[ignore = "requires spec-01: FileSource::load not yet implemented"]
#[tokio::test]
async fn remote_source_loads_signed_asset() {
    let server = MockServer::start().await;
    let fixture = std::fs::read("tests/fixtures/C.jpg").unwrap();
    Mock::given(method("GET"))
        .and(path("/asset.jpg"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(fixture)
                .insert_header("Content-Type", "image/jpeg"),
        )
        .mount(&server)
        .await;

    let url = url::Url::parse(&format!("{}/asset.jpg", server.uri())).unwrap();
    let client = RemoteClient::new().unwrap();
    let src = RemoteSource::new(url, Auth::None);
    let nodes = src.load(&client).await.unwrap();
    assert!(!nodes.is_empty());
}

#[tokio::test]
async fn remote_source_returns_auth_error_on_401() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/protected.jpg"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let url = url::Url::parse(&format!("{}/protected.jpg", server.uri())).unwrap();
    let client = RemoteClient::new().unwrap();
    let src = RemoteSource::new(url, Auth::None);
    let err = src.load(&client).await.unwrap_err();
    assert!(matches!(err, AppError::Auth(_)));
}

#[tokio::test]
async fn remote_source_returns_no_manifest_on_404() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/missing.jpg"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let url = url::Url::parse(&format!("{}/missing.jpg", server.uri())).unwrap();
    let client = RemoteClient::new().unwrap();
    let src = RemoteSource::new(url, Auth::None);
    let err = src.load(&client).await.unwrap_err();
    assert!(matches!(err, AppError::NoManifest(_)));
}

#[tokio::test]
async fn bearer_auth_header_is_sent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/secured.jpg"))
        .and(wiremock::matchers::header(
            "Authorization",
            "Bearer secrettoken",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![]))
        .mount(&server)
        .await;

    let url = url::Url::parse(&format!("{}/secured.jpg", server.uri())).unwrap();
    let client = RemoteClient::new().unwrap();
    let auth = Auth::from_spec("bearer:secrettoken").unwrap();
    // Fetch directly to verify the header is set; don't call RemoteSource::load
    // since the empty body would fail c2pa parsing.
    client.fetch(&url, &auth).await.unwrap();
}
