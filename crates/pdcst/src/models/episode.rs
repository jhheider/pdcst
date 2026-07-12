use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::str::FromStr;
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
}

impl FromStr for DownloadStatus {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "Downloading" => Self::Downloading,
            "Downloaded" => Self::Downloaded,
            "Failed" => Self::Failed,
            _ => Self::NotDownloaded,
        })
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
}

impl FromStr for PlaybackStatus {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "Playing" => Self::Playing,
            "Paused" => Self::Paused,
            _ => Self::Stopped,
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_download_status_conversions() {
        assert_eq!(
            "Downloaded".parse::<DownloadStatus>().unwrap(),
            DownloadStatus::Downloaded
        );
        assert_eq!(
            "Downloading".parse::<DownloadStatus>().unwrap(),
            DownloadStatus::Downloading
        );
        assert_eq!(
            "Failed".parse::<DownloadStatus>().unwrap(),
            DownloadStatus::Failed
        );
        assert_eq!(
            "Unknown".parse::<DownloadStatus>().unwrap(),
            DownloadStatus::NotDownloaded
        );
    }

    #[test]
    fn test_playback_status_conversions() {
        assert_eq!(
            "Playing".parse::<PlaybackStatus>().unwrap(),
            PlaybackStatus::Playing
        );
        assert_eq!(
            "Paused".parse::<PlaybackStatus>().unwrap(),
            PlaybackStatus::Paused
        );
        assert_eq!(
            "Stopped".parse::<PlaybackStatus>().unwrap(),
            PlaybackStatus::Stopped
        );
        assert_eq!(
            "Unknown".parse::<PlaybackStatus>().unwrap(),
            PlaybackStatus::Stopped
        );
    }

    #[test]
    fn test_episode_creation() {
        let sub_id = Uuid::new_v4();
        let episode = Episode::new(
            sub_id,
            "Test Episode".to_string(),
            "https://example.com/episode.mp3".to_string(),
            "guid123".to_string(),
            Utc::now(),
        );

        assert_eq!(episode.subscription_id, sub_id);
        assert_eq!(episode.title, "Test Episode");
        assert_eq!(episode.download_status, DownloadStatus::NotDownloaded);
        assert!(!episode.played);
        assert_eq!(episode.playback_position_seconds, 0);
    }

    #[test]
    fn test_duration_formatting() {
        let sub_id = Uuid::new_v4();
        let mut episode = Episode::new(
            sub_id,
            "Test".to_string(),
            "url".to_string(),
            "guid".to_string(),
            Utc::now(),
        );

        // Test hours and minutes
        episode.duration_seconds = Some(3665); // 1h 1m 5s
        assert_eq!(episode.duration_formatted(), "1h 1m");

        // Test minutes only
        episode.duration_seconds = Some(125); // 2m 5s
        assert_eq!(episode.duration_formatted(), "2m");

        // Test unknown
        episode.duration_seconds = None;
        assert_eq!(episode.duration_formatted(), "Unknown");
    }

    #[test]
    fn test_progress_percentage() {
        let sub_id = Uuid::new_v4();
        let mut episode = Episode::new(
            sub_id,
            "Test".to_string(),
            "url".to_string(),
            "guid".to_string(),
            Utc::now(),
        );

        episode.duration_seconds = Some(100);
        episode.playback_position_seconds = 50;
        assert_eq!(episode.progress_percentage(), 50.0);

        episode.playback_position_seconds = 0;
        assert_eq!(episode.progress_percentage(), 0.0);

        episode.playback_position_seconds = 100;
        assert_eq!(episode.progress_percentage(), 100.0);

        // Test with no duration
        episode.duration_seconds = None;
        assert_eq!(episode.progress_percentage(), 0.0);
    }

    #[test]
    fn test_is_downloaded() {
        let sub_id = Uuid::new_v4();
        let mut episode = Episode::new(
            sub_id,
            "Test".to_string(),
            "url".to_string(),
            "guid".to_string(),
            Utc::now(),
        );

        assert!(!episode.is_downloaded());

        episode.download_status = DownloadStatus::Downloaded;
        assert!(episode.is_downloaded());

        episode.download_status = DownloadStatus::Downloading;
        assert!(!episode.is_downloaded());
    }
}
