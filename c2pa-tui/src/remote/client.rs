use crate::error::Result;

/// HTTP client used for fetching remote manifests.
#[derive(Debug, Clone)]
pub struct RemoteClient {
    inner: reqwest::Client,
}

impl RemoteClient {
    /// Construct a new `RemoteClient` with sensible defaults.
    ///
    /// Stub: not yet implemented. Implemented in spec-02.
    pub fn new() -> Result<Self> {
        todo!("spec-02: implement RemoteClient::new")
    }

    /// Return a reference to the inner `reqwest::Client`.
    pub fn client(&self) -> &reqwest::Client {
        &self.inner
    }
}

impl Default for RemoteClient {
    fn default() -> Self {
        Self {
            inner: reqwest::Client::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_constructs_without_panic() {
        let client = RemoteClient::default();
        // Verify client() accessor returns a reference (doesn't panic).
        let _ = client.client();
    }

    #[test]
    fn clone_preserves_client() {
        let c1 = RemoteClient::default();
        let c2 = c1.clone();
        // Both accessors should succeed without panic.
        let _ = c1.client();
        let _ = c2.client();
    }
}
