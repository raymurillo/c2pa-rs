use async_trait::async_trait;
use std::path::PathBuf;
use url::Url;
use walkdir::WalkDir;

use crate::error::{AppError, Result};
use crate::manifest::tree::{store_to_nodes, DisplayNode, NodeValue};
use crate::remote::{Auth, RemoteClient};

/// Map a lowercase file extension to its MIME type.
/// Returns `None` for unsupported extensions.
fn ext_to_mime(ext: &str) -> Option<&'static str> {
    match ext {
        "jpg" | "jpeg" => Some("image/jpeg"),
        "png" => Some("image/png"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "tiff" | "tif" => Some("image/tiff"),
        "avif" => Some("image/avif"),
        "heic" | "heif" => Some("image/heic"),
        "mp4" | "m4v" => Some("video/mp4"),
        "mov" => Some("video/quicktime"),
        "avi" => Some("video/x-msvideo"),
        "pdf" => Some("application/pdf"),
        "c2pa" => Some("application/x-c2pa-manifest-store"),
        _ => None,
    }
}

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

    #[tracing::instrument(skip(self, _client), fields(path = %self.path.display()))]
    async fn load(&self, _client: &RemoteClient) -> Result<Vec<DisplayNode>> {
        let ext = self
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        ext_to_mime(&ext).ok_or_else(|| AppError::UnsupportedFormat(ext.clone()))?;

        let reader = c2pa::Reader::default().with_file(&self.path);
        match reader {
            Err(c2pa::Error::JumbfNotFound) | Err(c2pa::Error::ProvenanceMissing) => {
                tracing::warn!("no C2PA manifest found");
                Ok(vec![DisplayNode {
                    key: "status".into(),
                    value: NodeValue::Str("No C2PA manifest found".into()),
                    children: vec![],
                }])
            }
            Err(e) => Err(AppError::C2pa(e)),
            Ok(reader) => {
                tracing::debug!("manifest loaded successfully");
                Ok(store_to_nodes(&reader))
            }
        }
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

    /// Enumerate all supported files in the directory, sorted by name.
    pub fn entries(&self) -> Result<Vec<FileSource>> {
        let mut sources = Vec::new();
        for entry in WalkDir::new(&self.path).sort_by_file_name() {
            let entry = entry?;
            if entry.file_type().is_file() {
                let ext = entry
                    .path()
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if ext_to_mime(&ext).is_some() {
                    sources.push(FileSource::new(entry.path().to_path_buf()));
                }
            }
        }
        Ok(sources)
    }
}

#[async_trait]
impl ManifestSource for DirSource {
    fn label(&self) -> &str {
        &self.label
    }

    /// Load all supported files in the directory sequentially.
    ///
    /// Each file's nodes are wrapped under a parent node keyed by the filename.
    /// If a file fails to parse, an error node is produced instead of aborting.
    async fn load(&self, client: &RemoteClient) -> Result<Vec<DisplayNode>> {
        let entries = self.entries()?;
        let mut nodes = Vec::new();
        for file_source in entries {
            let filename = file_source
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("<unknown>")
                .to_owned();
            let file_nodes = match file_source.load(client).await {
                Ok(ns) => ns,
                Err(e) => vec![DisplayNode {
                    key: "error".into(),
                    value: NodeValue::Str(e.to_string()),
                    children: vec![],
                }],
            };
            nodes.push(DisplayNode {
                key: filename,
                value: NodeValue::Missing,
                children: file_nodes,
            });
        }
        Ok(nodes)
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

    #[tokio::test]
    async fn unsupported_ext_returns_error() {
        let src = FileSource::new(PathBuf::from("/tmp/test.txt"));
        let client = RemoteClient::default();
        let result = src.load(&client).await;
        assert!(matches!(result, Err(AppError::UnsupportedFormat(_))));
    }

    #[test]
    fn dir_entries_returns_only_supported_files() {
        use std::fs;
        use tempfile::TempDir;

        let tmp = TempDir::new().unwrap();
        let tmp_path = tmp.path();
        fs::write(tmp_path.join("a.jpg"), b"").unwrap();
        fs::write(tmp_path.join("b.png"), b"").unwrap();
        fs::write(tmp_path.join("c.txt"), b"").unwrap();

        let src = DirSource::new(tmp_path.to_path_buf());
        let entries = src.entries().unwrap();
        assert_eq!(entries.len(), 2);
        let names: Vec<String> = entries
            .iter()
            .map(|e| e.path.file_name().unwrap().to_str().unwrap().to_owned())
            .collect();
        assert!(names.contains(&"a.jpg".to_owned()));
        assert!(names.contains(&"b.png".to_owned()));
    }
}
