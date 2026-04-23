# Spec 02 — Remote HTTP Layer

**Phase:** 1 (concurrent with spec-01, spec-03, spec-04, spec-05)  
**Depends on:** spec-00 foundation committed and compiling  
**Produces:** `remote/auth.rs`, `remote/client.rs` fully implemented; `RemoteSource::load` implemented in `manifest/loader.rs`

---

## Goal

Implement authenticated HTTP fetching of C2PA-embedded assets. Follow **TDD order**:
write tests first, then implement. No `.unwrap()` in production code — use `?` and
`.map_err`. Add `#[tracing::instrument]` to all async public methods. A `RemoteSource`
fetches the raw asset bytes from a URL, writes them to a temp file, and passes
that file to the `c2pa` SDK for parsing. The `RemoteClient` wraps `reqwest` with
retry logic, timeouts, and auth injection.

---

## Files to modify

- `src/remote/auth.rs` — implement `Auth::from_spec` and `Auth::apply`
- `src/remote/client.rs` — implement `RemoteClient::new` and `RemoteClient::fetch`
- `src/manifest/loader.rs` — implement `RemoteSource::load`

Do **not** change anything in `manifest/tree.rs` or `manifest/filter.rs`.

---

## `remote/auth.rs`

### `Auth::from_spec`

Parse the CLI `--auth` flag string. Format: `scheme:arg1:arg2`.

| Input string | Output |
|---|---|
| `none` or empty | `Auth::None` |
| `basic:username:password` | `Auth::Basic { username, password }` |
| `bearer:token` | `Auth::Bearer { token }` |
| `digest:username:password` | `Auth::Digest { username, password }` |

If the string doesn't match any pattern, return `AppError::Auth("invalid auth spec: ...")`.

Note: the `password` field in `basic:` and `digest:` may itself contain `:`, so split
on `:` a maximum of 2 times for those variants.

```rust
pub fn from_spec(s: &str) -> Result<Self> {
    if s.is_empty() || s == "none" {
        return Ok(Auth::None);
    }
    let parts: Vec<&str> = s.splitn(3, ':').collect();
    match parts.as_slice() {
        ["bearer", token] => Ok(Auth::Bearer { token: token.to_string() }),
        ["basic", user, pass] => Ok(Auth::Basic {
            username: user.to_string(),
            password: pass.to_string(),
        }),
        ["digest", user, pass] => Ok(Auth::Digest {
            username: user.to_string(),
            password: pass.to_string(),
        }),
        _ => Err(AppError::Auth(format!("invalid auth spec: {s:?}"))),
    }
}
```

### `Auth::apply`

```rust
pub fn apply(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    match self {
        Auth::None => builder,
        Auth::Basic { username, password } =>
            builder.basic_auth(username, Some(password)),
        Auth::Bearer { token } =>
            builder.bearer_auth(token),
        Auth::Digest { username, password } => {
            // reqwest does not natively support Digest auth.
            // Apply Basic as a fallback and document this limitation.
            // A future improvement could use a dedicated digest-auth crate.
            builder.basic_auth(username, Some(password))
        }
    }
}
```

> Document the Digest fallback limitation in a code comment (this is one of the
> rare cases where a comment explaining WHY is warranted).

---

## `remote/client.rs`

### `RemoteClient`

```rust
pub struct RemoteClient {
    inner: reqwest::Client,
}

impl RemoteClient {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .user_agent(concat!("c2pa-tui/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(AppError::Http)?;
        Ok(Self { inner: client })
    }

    /// Fetch raw asset bytes from a URL, applying authentication and retrying on
    /// transient network errors (up to 2 retries with exponential back-off).
    ///
    /// Returns `AppError::Auth` on 401/403, `AppError::NoManifest` on 404.
    #[tracing::instrument(skip(self, auth), fields(url = %url))]
    pub async fn fetch(&self, url: &url::Url, auth: &Auth) -> Result<bytes::Bytes> {
        let mut attempts = 0u8;
        loop {
            let builder = self.inner.get(url.as_str());
            let builder = auth.apply(builder);
            let response = builder.send().await;
            match response {
                Ok(resp) => {
                    let status = resp.status();
                    if status == reqwest::StatusCode::UNAUTHORIZED
                        || status == reqwest::StatusCode::FORBIDDEN {
                        return Err(AppError::Auth(
                            format!("HTTP {status} from {url}")
                        ));
                    }
                    if status == reqwest::StatusCode::NOT_FOUND {
                        return Err(AppError::NoManifest(url.to_string()));
                    }
                    if !status.is_success() {
                        return Err(AppError::Http(
                            // reqwest doesn't expose arbitrary status errors directly;
                            // use resp.error_for_status() to convert
                            resp.error_for_status().unwrap_err()
                        ));
                    }
                    return resp.bytes().await.map_err(AppError::Http);
                }
                Err(e) if attempts < 2 && e.is_connect() => {
                    attempts += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(300 * u64::from(attempts))).await;
                }
                Err(e) => return Err(AppError::Http(e)),
            }
        }
    }
}
```

Add `bytes` to `Cargo.toml` dev/dependencies or use `reqwest`'s `bytes` re-export
(`reqwest::Response::bytes()` returns `bytes::Bytes`).

---

## `manifest/loader.rs` — `RemoteSource::load`

```rust
async fn load(&self, client: &RemoteClient) -> Result<Vec<DisplayNode>> {
    use std::io::Write;

    let bytes = client.fetch(&self.url, &self.auth).await?;

    // Write to a named temp file so c2pa can detect format by extension.
    // Derive extension from URL path, fall back to ".bin".
    let ext = self.url.path_segments()
        .and_then(|segs| segs.last())
        .and_then(|seg| seg.rsplit('.').next())
        .unwrap_or("bin");
    let mut tmp = tempfile::Builder::new()
        .suffix(&format!(".{ext}"))
        .tempfile()?;
    tmp.write_all(&bytes)?;
    tmp.flush()?;

    let path = tmp.path().to_path_buf();
    let src = crate::manifest::loader::FileSource::new(path);
    // keep tmp alive until load completes
    let result = src.load(client).await;
    drop(tmp);
    result
}
```

Add `tempfile = "3"` to `Cargo.toml`.

---

## Property-based tests for `Auth::from_spec`

Add a `proptest!` block in `auth.rs`:

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn from_spec_never_panics(s in ".*") {
        // Must not panic for any input — only Ok or Err
        let _ = Auth::from_spec(&s);
    }

    #[test]
    fn bearer_roundtrip(token in "[a-zA-Z0-9_\\-]{1,64}") {
        let spec = format!("bearer:{token}");
        let auth = Auth::from_spec(&spec).unwrap();
        assert!(matches!(auth, Auth::Bearer { token: t } if t == token));
    }
}
```

## Unit tests — `remote/auth.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_none() {
        assert!(matches!(Auth::from_spec("none").unwrap(), Auth::None));
        assert!(matches!(Auth::from_spec("").unwrap(), Auth::None));
    }

    #[test]
    fn parse_bearer() {
        let auth = Auth::from_spec("bearer:mytoken123").unwrap();
        assert!(matches!(auth, Auth::Bearer { token } if token == "mytoken123"));
    }

    #[test]
    fn parse_basic() {
        let auth = Auth::from_spec("basic:alice:s3cr3t").unwrap();
        assert!(matches!(auth, Auth::Basic { username, password }
            if username == "alice" && password == "s3cr3t"));
    }

    #[test]
    fn parse_basic_password_with_colon() {
        let auth = Auth::from_spec("basic:alice:pass:word").unwrap();
        assert!(matches!(auth, Auth::Basic { password, .. } if password == "pass:word"));
    }

    #[test]
    fn parse_invalid_returns_error() {
        assert!(Auth::from_spec("oauth:something").is_err());
    }
}
```

---

## Integration tests — `tests/integration_remote.rs`

Use `wiremock` to stand up a local HTTP server.

```rust
use wiremock::{MockServer, Mock, ResponseTemplate};
use wiremock::matchers::{method, path};
use c2pa_tui::remote::{RemoteClient, Auth};
use c2pa_tui::manifest::loader::RemoteSource;

#[tokio::test]
async fn remote_source_loads_signed_asset() {
    let server = MockServer::start().await;
    let fixture = std::fs::read("tests/fixtures/signed.jpg").unwrap();
    Mock::given(method("GET")).and(path("/asset.jpg"))
        .respond_with(ResponseTemplate::new(200)
            .set_body_bytes(fixture)
            .insert_header("Content-Type", "image/jpeg"))
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
    Mock::given(method("GET")).and(path("/protected.jpg"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let url = url::Url::parse(&format!("{}/protected.jpg", server.uri())).unwrap();
    let client = RemoteClient::new().unwrap();
    let src = RemoteSource::new(url, Auth::None);
    let err = src.load(&client).await.unwrap_err();
    assert!(matches!(err, c2pa_tui::error::AppError::Auth(_)));
}

#[tokio::test]
async fn remote_source_returns_no_manifest_on_404() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/missing.jpg"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let url = url::Url::parse(&format!("{}/missing.jpg", server.uri())).unwrap();
    let client = RemoteClient::new().unwrap();
    let src = RemoteSource::new(url, Auth::None);
    let err = src.load(&client).await.unwrap_err();
    assert!(matches!(err, c2pa_tui::error::AppError::NoManifest(_)));
}

#[tokio::test]
async fn bearer_auth_header_is_sent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/secured.jpg"))
        .and(wiremock::matchers::header("Authorization", "Bearer secrettoken"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![]))
        // will return 200 only if Authorization header matches
        .mount(&server)
        .await;

    let url = url::Url::parse(&format!("{}/secured.jpg", server.uri())).unwrap();
    let client = RemoteClient::new().unwrap();
    let auth = Auth::from_spec("bearer:secrettoken").unwrap();
    // fetch directly to verify header is set (don't call RemoteSource::load as
    // the empty body would fail c2pa parsing)
    client.fetch(&url, &auth).await.unwrap();
}
```

---

## Done criteria

```
cargo test --lib remote
cargo test --test integration_remote
cargo build
cargo fmt -- --check
cargo clippy -- -D warnings
```
