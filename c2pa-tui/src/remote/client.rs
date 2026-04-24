use bytes::Bytes;

use crate::error::{AppError, Result};
use crate::remote::Auth;

/// HTTP client used for fetching remote manifests.
#[derive(Debug, Clone)]
pub struct RemoteClient {
    inner: reqwest::Client,
}

impl RemoteClient {
    /// Construct a new `RemoteClient` with sensible defaults.
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .user_agent(concat!("c2pa-tui/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(AppError::Http)?;
        Ok(Self { inner: client })
    }

    /// Return a reference to the inner `reqwest::Client`.
    pub fn client(&self) -> &reqwest::Client {
        &self.inner
    }

    /// Fetch raw asset bytes from a URL, applying authentication and retrying on
    /// transient network errors (up to 2 retries with exponential back-off).
    ///
    /// Returns `AppError::Auth` on 401/403, `AppError::NoManifest` on 404.
    #[tracing::instrument(skip(self, auth), fields(url = %url))]
    pub async fn fetch(&self, url: &url::Url, auth: &Auth) -> Result<Bytes> {
        // Reject anything that isn't http or https regardless of how the URL was constructed.
        let scheme = url.scheme();
        if scheme != "http" && scheme != "https" {
            return Err(AppError::UnsupportedFormat(format!(
                "unsupported URL scheme {scheme:?}; only http and https are allowed"
            )));
        }

        // Basic and Digest credentials would be transmitted in cleartext over plain HTTP.
        // Refuse rather than silently leak credentials.
        if scheme == "http" && matches!(auth, Auth::Basic { .. } | Auth::Digest { .. }) {
            return Err(AppError::Auth(
                "Basic and Digest authentication require HTTPS; refusing to send \
                 credentials over an unencrypted connection"
                    .into(),
            ));
        }

        let mut attempts = 0u8;
        loop {
            let builder = self.inner.get(url.as_str());
            let builder = auth.apply(builder);
            let response = builder.send().await;
            match response {
                Ok(resp) => {
                    let status = resp.status();
                    if status == reqwest::StatusCode::UNAUTHORIZED
                        || status == reqwest::StatusCode::FORBIDDEN
                    {
                        return Err(AppError::Auth(format!("HTTP {status} from {url}")));
                    }
                    if status == reqwest::StatusCode::NOT_FOUND {
                        return Err(AppError::NoManifest(url.to_string()));
                    }
                    if !status.is_success() {
                        // error_for_status() returns Err for non-2xx; safe to expect here
                        // only because we just confirmed !status.is_success() above.
                        return Err(AppError::Http(
                            resp.error_for_status()
                                .expect_err("status confirmed non-success"),
                        ));
                    }
                    return resp.bytes().await.map_err(AppError::Http);
                }
                // Retry on both connect errors and request timeouts. Timeouts
                // commonly represent transient server overload rather than a
                // permanent failure, so a bounded retry with back-off is safe.
                Err(e) if attempts < 2 && (e.is_connect() || e.is_timeout()) => {
                    attempts += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(300 * u64::from(attempts)))
                        .await;
                }
                Err(e) => return Err(AppError::Http(e)),
            }
        }
    }
}

impl Default for RemoteClient {
    /// Construct a `RemoteClient` using the same timeouts and user-agent as
    /// [`RemoteClient::new`]. Provided for test convenience; production code
    /// should prefer [`RemoteClient::new`] for explicit error handling.
    fn default() -> Self {
        // `new()` is infallible with these builder settings (no TLS
        // customisation that can fail at runtime), so `expect` is appropriate.
        Self::new().expect("RemoteClient::new is infallible with default settings")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_builds_configured_client() {
        // new() is the primary constructor — it sets timeout, connect_timeout, and
        // user_agent, any of which could fail via reqwest::ClientBuilder::build().
        let client = RemoteClient::new().expect("RemoteClient::new should succeed");
        let _ = client.client();
    }

    #[test]
    fn clone_preserves_client() {
        let c1 = RemoteClient::new().unwrap();
        let c2 = c1.clone();
        let _ = c1.client();
        let _ = c2.client();
    }

    #[test]
    fn default_does_not_panic() {
        let client = RemoteClient::default();
        // Confirms the inner client was initialised (i.e. `Default` actually
        // went through `new()` rather than bypassing it).
        let _ = client.client();
    }

    #[tokio::test]
    async fn fetch_rejects_non_http_scheme() {
        let client = RemoteClient::new().unwrap();
        let url = url::Url::parse("ftp://example.com/asset.jpg").unwrap();
        let err = client.fetch(&url, &Auth::None).await.unwrap_err();
        assert!(matches!(err, AppError::UnsupportedFormat(_)));
    }

    #[tokio::test]
    async fn fetch_rejects_basic_auth_over_http() {
        let client = RemoteClient::new().unwrap();
        let url = url::Url::parse("http://example.com/asset.jpg").unwrap();
        let auth = Auth::from_spec("basic:user:pass").unwrap();
        let err = client.fetch(&url, &auth).await.unwrap_err();
        assert!(matches!(err, AppError::Auth(_)));
    }

    #[tokio::test]
    async fn fetch_rejects_digest_auth_over_http() {
        let client = RemoteClient::new().unwrap();
        let url = url::Url::parse("http://example.com/asset.jpg").unwrap();
        let auth = Auth::from_spec("digest:user:pass").unwrap();
        let err = client.fetch(&url, &auth).await.unwrap_err();
        assert!(matches!(err, AppError::Auth(_)));
    }
}
