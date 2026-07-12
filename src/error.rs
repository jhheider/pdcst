//! Typed error system for podcast-tui
//!
//! This module provides a structured error type hierarchy using `thiserror`.
//! Each error variant provides context-specific information and proper error chaining.

use std::path::PathBuf;
use thiserror::Error;

/// Main error type for the application
#[derive(Error, Debug)]
pub enum PodcastError {
    // Audio errors
    #[error("Audio playback error: {0}")]
    AudioPlayback(String),

    #[error("Audio decoding error: {0}")]
    AudioDecoding(String),

    #[error("Failed to initialize audio output: {0}")]
    AudioOutput(String),

    // Streaming errors
    #[error("Failed to stream from URL {url}: {source}")]
    StreamingFailed { url: String, source: reqwest::Error },

    #[error("HTTP error {status} while fetching {url}")]
    HttpError { status: u16, url: String },

    #[error("Stream interrupted after {bytes_received} bytes")]
    StreamInterrupted { bytes_received: u64 },

    // Database errors
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Database migration failed: {0}")]
    Migration(String),

    #[error("Entity not found: {entity_type} with id {id}")]
    NotFound { entity_type: String, id: String },

    #[error("Duplicate entity: {entity_type} already exists")]
    Duplicate { entity_type: String },

    // Feed errors
    #[error("Failed to parse RSS feed from {url}: {source}")]
    FeedParse {
        url: String,
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    #[error("Failed to fetch feed from {url}: {source}")]
    FeedFetch { url: String, source: reqwest::Error },

    #[error("Invalid feed URL: {0}")]
    InvalidFeedUrl(String),

    // File I/O errors
    #[error("Failed to read file {path}: {source}")]
    FileRead {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("Failed to write file {path}: {source}")]
    FileWrite {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("File not found: {0}")]
    FileNotFound(PathBuf),

    // Download errors
    #[error("Download failed for episode {episode_id}: {reason}")]
    DownloadFailed { episode_id: String, reason: String },

    #[error("Insufficient disk space: need {needed} bytes, have {available} bytes")]
    InsufficientSpace { needed: u64, available: u64 },

    // Configuration errors
    #[error("Invalid configuration: {0}")]
    InvalidConfig(String),

    #[error("Failed to load configuration from {path}: {source}")]
    ConfigLoad {
        path: PathBuf,
        source: std::io::Error,
    },

    // Serialization errors
    #[error("Failed to serialize data: {0}")]
    Serialization(String),

    #[error("Failed to deserialize data: {0}")]
    Deserialization(String),

    // Channel communication errors
    #[error("Channel send error: {0}")]
    ChannelSend(String),

    #[error("Channel receive error: {0}")]
    ChannelReceive(String),

    // General errors
    #[error("Operation timed out after {seconds} seconds")]
    Timeout { seconds: u64 },

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Operation cancelled")]
    Cancelled,

    #[error("Internal error: {0}")]
    Internal(String),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Result type alias for PodcastError
pub type Result<T> = std::result::Result<T, PodcastError>;

impl PodcastError {
    /// Check if error is retryable (transient network/server issues)
    pub fn is_retryable(&self) -> bool {
        match self {
            PodcastError::StreamingFailed { .. } => true,
            PodcastError::HttpError { status, .. } if *status >= 500 => true,
            PodcastError::FeedFetch { .. } => true,
            PodcastError::StreamInterrupted { .. } => true,
            PodcastError::Timeout { .. } => true,
            _ => false,
        }
    }

    /// Check if error is permanent (won't succeed on retry)
    pub fn is_permanent(&self) -> bool {
        match self {
            PodcastError::NotFound { .. } => true,
            PodcastError::InvalidFeedUrl(_) => true,
            PodcastError::FileNotFound(_) => true,
            PodcastError::InvalidInput(_) => true,
            PodcastError::HttpError { status, .. } if *status == 404 || *status == 410 => true,
            _ => false,
        }
    }

    /// Get error category for logging/metrics
    pub fn category(&self) -> &'static str {
        match self {
            PodcastError::AudioPlayback(_)
            | PodcastError::AudioDecoding(_)
            | PodcastError::AudioOutput(_) => "audio",

            PodcastError::StreamingFailed { .. }
            | PodcastError::HttpError { .. }
            | PodcastError::StreamInterrupted { .. } => "network",

            PodcastError::Database(_)
            | PodcastError::Migration(_)
            | PodcastError::NotFound { .. }
            | PodcastError::Duplicate { .. } => "database",

            PodcastError::FeedParse { .. }
            | PodcastError::FeedFetch { .. }
            | PodcastError::InvalidFeedUrl(_) => "feed",

            PodcastError::FileRead { .. }
            | PodcastError::FileWrite { .. }
            | PodcastError::FileNotFound(_) => "filesystem",

            PodcastError::DownloadFailed { .. } | PodcastError::InsufficientSpace { .. } => {
                "download"
            }

            PodcastError::InvalidConfig(_) | PodcastError::ConfigLoad { .. } => "config",

            PodcastError::Serialization(_) | PodcastError::Deserialization(_) => "serialization",

            PodcastError::ChannelSend(_) | PodcastError::ChannelReceive(_) => "channel",

            PodcastError::Timeout { .. }
            | PodcastError::InvalidInput(_)
            | PodcastError::Cancelled
            | PodcastError::Internal(_)
            | PodcastError::Other(_) => "general",
        }
    }
}

// Conversion helpers for common error types
impl From<std::io::Error> for PodcastError {
    fn from(err: std::io::Error) -> Self {
        PodcastError::Internal(err.to_string())
    }
}

impl From<serde_json::Error> for PodcastError {
    fn from(err: serde_json::Error) -> Self {
        PodcastError::Deserialization(err.to_string())
    }
}

impl From<toml::de::Error> for PodcastError {
    fn from(err: toml::de::Error) -> Self {
        PodcastError::Deserialization(err.to_string())
    }
}

impl From<toml::ser::Error> for PodcastError {
    fn from(err: toml::ser::Error) -> Self {
        PodcastError::Serialization(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retryable_errors() {
        let err = PodcastError::HttpError {
            status: 503,
            url: "http://example.com".to_string(),
        };
        assert!(err.is_retryable());
        assert!(!err.is_permanent());
    }

    #[test]
    fn test_permanent_errors() {
        let err = PodcastError::NotFound {
            entity_type: "Episode".to_string(),
            id: "123".to_string(),
        };
        assert!(err.is_permanent());
        assert!(!err.is_retryable());
    }

    #[test]
    fn test_error_categories() {
        let audio_err = PodcastError::AudioPlayback("test".to_string());
        assert_eq!(audio_err.category(), "audio");

        let db_err = PodcastError::Database(sqlx::Error::RowNotFound);
        assert_eq!(db_err.category(), "database");

        let net_err = PodcastError::HttpError {
            status: 500,
            url: "test".to_string(),
        };
        assert_eq!(net_err.category(), "network");
    }

    #[test]
    fn test_error_display() {
        let err = PodcastError::HttpError {
            status: 404,
            url: "http://example.com/audio.mp3".to_string(),
        };

        let msg = err.to_string();
        assert!(msg.contains("http://example.com/audio.mp3"));
        assert!(msg.contains("404"));
    }

    #[test]
    fn test_file_errors() {
        let err = PodcastError::FileNotFound(PathBuf::from("/tmp/missing.mp3"));
        assert_eq!(err.category(), "filesystem");
        assert!(err.is_permanent());
    }
}
