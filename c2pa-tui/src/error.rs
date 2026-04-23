/// All errors that can occur within c2pa-tui.
#[derive(thiserror::Error, Debug)]
pub enum AppError {
    #[error("c2pa error: {0}")]
    C2pa(#[from] c2pa::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("authentication error: {0}")]
    Auth(String),

    #[error("unsupported file type: {0}")]
    UnsupportedFormat(String),

    #[error("manifest not found in {0}")]
    NoManifest(String),

    #[error("terminal error: {0}")]
    Terminal(String),

    #[error("directory walk error: {0}")]
    Walk(#[from] walkdir::Error),

    #[error("invalid URL: {0}")]
    Url(#[from] url::ParseError),

    #[error("invalid glob pattern: {0}")]
    Glob(#[from] glob::PatternError),
}

/// Convenience alias for `Result` with [`AppError`].
pub type Result<T> = std::result::Result<T, AppError>;
