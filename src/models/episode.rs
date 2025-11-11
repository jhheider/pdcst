use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub enum DownloadStatus {
    NotDownloaded,
    Downloading,
    Downloaded,
    Failed,
}

impl DownloadStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::NotDownloaded => "NotDownloaded",
            Self::Downloading => "Downloading",
            Self::Downloaded => "Downloaded",
            Self::Failed => "Failed",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "Downloading" => Self::Downloading,
            "Downloaded" => Self::Downloaded,
            "Failed" => Self::Failed,
            _ => Self::NotDownloaded,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
}

impl PlaybackStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Playing => "Playing",
            Self::Paused => "Paused",
            Self::Stopped => "Stopped",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "Playing" => Self::Playing,
            "Paused" => Self::Paused,
            _ => Self::Stopped,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Episode {
    pub id: Uuid,
    pub subscription_id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub url: String,
    pub guid: String,
    pub published_at: DateTime<Utc>,
    pub duration_seconds: Option<i64>,
    pub file_size_bytes: Option<i64>,
    pub file_type: Option<String>,
    pub download_status: DownloadStatus,
    pub local_path: Option<PathBuf>,
    pub playback_position_seconds: i64,
    pub played: bool,
    pub last_played_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

impl Episode {
    pub fn new(
        subscription_id: Uuid,
        title: String,
        url: String,
        guid: String,
        published_at: DateTime<Utc>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            subscription_id,
            title,
            description: None,
            url,
            guid,
            published_at,
            duration_seconds: None,
            file_size_bytes: None,
            file_type: None,
            download_status: DownloadStatus::NotDownloaded,
            local_path: None,
            playback_position_seconds: 0,
            played: false,
            last_played_at: None,
            created_at: Utc::now(),
        }
    }

    pub fn is_downloaded(&self) -> bool {
        self.download_status == DownloadStatus::Downloaded
    }

    pub fn duration_formatted(&self) -> String {
        if let Some(seconds) = self.duration_seconds {
            let hours = seconds / 3600;
            let minutes = (seconds % 3600) / 60;

            if hours > 0 {
                format!("{}h {}m", hours, minutes)
            } else {
                format!("{}m", minutes)
            }
        } else {
            "Unknown".to_string()
        }
    }

    pub fn progress_percentage(&self) -> f64 {
        if let Some(duration) = self.duration_seconds {
            if duration > 0 {
                (self.playback_position_seconds as f64 / duration as f64) * 100.0
            } else {
                0.0
            }
        } else {
            0.0
        }
    }
}
