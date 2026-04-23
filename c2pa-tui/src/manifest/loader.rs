use async_trait::async_trait;
use std::path::PathBuf;
use url::Url;

use crate::error::Result;
use crate::manifest::tree::DisplayNode;
use crate::remote::{Auth, RemoteClient};

/// Abstraction over all manifest origins: local files, directories, and remote URLs.
///
/// Implementors must be `Send + Sync` so they can be stored in `App` and loaded
/// from background tokio tasks.
#[async_trait]
#[cfg_attr(test, mockall::automock)]
pub trait ManifestSource: Send + Sync {
    /// Human-readable label shown in the file list pane.
    fn label(&self) -> &str;
    /// Load and parse the manifest, returning a `DisplayNode` tree.
    async fn load(&self, client: &RemoteClient) -> Result<Vec<DisplayNode>>;
    /// Returns `true` for sources that can be re-fetched (e.g. HTTP URLs).
    fn is_remote(&self) -> bool {
        false
    }
}

/// A manifest source backed by a single local file.
pub struct FileSource {
    /// Path to the local file.
    pub path: PathBuf,
    label: String,
}

impl FileSource {
    /// Create a new `FileSource` from the given path.
    pub fn new(path: PathBuf) -> Self {
        let label = path.display().to_string();
        Self { path, label }
    }
}

#[async_trait]
impl ManifestSource for FileSource {
    fn label(&self) -> &str {
        &self.label
    }

    async fn load(&self, _client: &RemoteClient) -> Result<Vec<DisplayNode>> {
        todo!("spec-01: implement FileSource::load")
    }
}

/// A manifest source backed by a directory of files.
pub struct DirSource {
    /// Path to the directory.
    pub path: PathBuf,
    label: String,
}

impl DirSource {
    /// Create a new `DirSource` from the given directory path.
    pub fn new(path: PathBuf) -> Self {
        let label = path.display().to_string();
        Self { path, label }
    }

    /// Enumerate all supported files. Stub: returns empty vec. Implemented in spec-01.
    pub fn entries(&self) -> crate::error::Result<Vec<FileSource>> {
        todo!("spec-01: implement DirSource::entries")
    }
}

#[async_trait]
impl ManifestSource for DirSource {
    fn label(&self) -> &str {
        &self.label
    }

    async fn load(&self, _client: &RemoteClient) -> Result<Vec<DisplayNode>> {
        todo!("spec-01: implement DirSource::load")
    }
}

/// A manifest source backed by a remote HTTP URL.
pub struct RemoteSource {
    /// URL of the remote manifest.
    pub url: Url,
    /// Authentication credentials to use when fetching.
    pub auth: Auth,
    label: String,
}

impl RemoteSource {
    /// Create a new `RemoteSource` for the given URL and auth method.
    pub fn new(url: Url, auth: Auth) -> Self {
        let label = url.to_string();
        Self { url, auth, label }
    }
}

#[async_trait]
impl ManifestSource for RemoteSource {
    fn label(&self) -> &str {
        &self.label
    }

    fn is_remote(&self) -> bool {
        true
    }

    async fn load(&self, client: &RemoteClient) -> Result<Vec<DisplayNode>> {
        use std::io::Write;

        let bytes = client.fetch(&self.url, &self.auth).await?;

        // Write to a named temp file so c2pa can detect format by extension.
        // Derive the extension from the URL path and allowlist it to prevent unexpected
        // characters (path separators, null bytes, overly long strings) from reaching
        // the filesystem via the suffix. Fall back to ".bin" for unknown extensions.
        let raw_ext = self
            .url
            .path_segments()
            .and_then(|mut segs| segs.next_back())
            .and_then(|seg| seg.rsplit('.').next())
            .unwrap_or("bin");
        let ext = if raw_ext.len() <= 10 && raw_ext.chars().all(|c| c.is_ascii_alphanumeric()) {
            raw_ext
        } else {
            "bin"
        };
        let mut tmp = tempfile::Builder::new()
            .suffix(&format!(".{ext}"))
            .tempfile()?;
        tmp.write_all(&bytes)?;
        tmp.flush()?;

        let path = tmp.path().to_path_buf();
        let src = FileSource::new(path);
        // keep tmp alive until load completes so the temp file isn't deleted early
        let result = src.load(client).await;
        drop(tmp);
        result
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use url::Url;

    use super::*;
    use crate::remote::Auth;

    #[test]
    fn file_source_label_matches_path() {
        let path = PathBuf::from("/tmp/test.jpg");
        let src = FileSource::new(path.clone());
        assert_eq!(src.label(), "/tmp/test.jpg");
        assert_eq!(src.path, path);
    }

    #[test]
    fn file_source_is_not_remote() {
        let src = FileSource::new(PathBuf::from("/tmp/x.jpg"));
        assert!(!src.is_remote());
    }

    #[test]
    fn dir_source_label_matches_path() {
        let path = PathBuf::from("/tmp/manifests");
        let src = DirSource::new(path.clone());
        assert_eq!(src.label(), "/tmp/manifests");
        assert_eq!(src.path, path);
    }

    #[test]
    fn dir_source_is_not_remote() {
        let src = DirSource::new(PathBuf::from("/tmp/dir"));
        assert!(!src.is_remote());
    }

    #[test]
    fn remote_source_label_matches_url() {
        let url = Url::parse("https://example.com/manifest.jpg").unwrap();
        let src = RemoteSource::new(url.clone(), Auth::None);
        assert_eq!(src.label(), url.as_str());
        assert_eq!(src.url, url);
    }

    #[test]
    fn remote_source_is_remote() {
        let url = Url::parse("https://example.com/asset.png").unwrap();
        let src = RemoteSource::new(url, Auth::None);
        assert!(src.is_remote());
    }
}
