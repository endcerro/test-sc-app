use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DataError {
    #[error("HTTP request to {url} failed: {message}")]
    Http { url: String, message: String },

    #[error("unexpected response from {url}: {message}")]
    InvalidResponse { url: String, message: String },

    #[error("I/O error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to parse {what}: {source}")]
    Json {
        what: String,
        #[source]
        source: serde_json::Error,
    },

    /// Network failed and nothing usable was cached.
    #[error("universe data unavailable: {0}")]
    Unavailable(String),
}

impl DataError {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
