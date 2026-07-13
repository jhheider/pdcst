use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::str::FromStr;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum SubscriptionPriority {
    High,
    Medium,
    Low,
}

impl SubscriptionPriority {
    pub fn as_str(&self) -> &str {
        match self {
            Self::High => "High",
            Self::Medium => "Medium",
            Self::Low => "Low",
        }
    }
}

impl FromStr for SubscriptionPriority {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "High" => Self::High,
            "Low" => Self::Low,
            _ => Self::Medium,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subscription {
    pub id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub rss_url: String,
    pub website_url: Option<String>,
    pub artwork_url: Option<String>,
    pub artwork_path: Option<PathBuf>,
    #[serde(default)]
    pub categories: Vec<String>,
    /// When true, new episodes from this feed are auto-added to the queue.
    pub auto_queue: bool,
    /// Auto-add direction: true = prepend to the top (unshift), false = append
    /// to the bottom (push). Only meaningful when `auto_queue` is set.
    #[serde(default)]
    pub auto_queue_to_top: bool,
    pub priority: SubscriptionPriority,
    pub auto_download: bool,
    pub last_refreshed: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    /// The most recent refresh failure, or `None` if the last refresh succeeded
    /// (or none has run). Surfaced in the subscription row so a dead URL or an
    /// unparseable feed is visible at a glance, not just in a passing status.
    #[serde(default)]
    pub last_error: Option<String>,

    // Aggregate stats, computed by a join at load time (see
    // `get_all_subscriptions`), not stored on the row. Skipped in (de)serialize
    // so OPML round-trips and the DB insert ignore them; default to zero/None.
    #[serde(skip)]
    pub episode_count: i64,
    #[serde(skip)]
    pub unplayed_count: i64,
    #[serde(skip)]
    pub latest_episode_at: Option<DateTime<Utc>>,
}

impl Subscription {
    pub fn new(title: String, rss_url: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            title,
            description: None,
            author: None,
            rss_url,
            website_url: None,
            artwork_url: None,
            artwork_path: None,
            categories: Vec::new(),
            auto_queue: false,
            auto_queue_to_top: false,
            priority: SubscriptionPriority::Medium,
            auto_download: false,
            last_refreshed: now,
            created_at: now,
            last_error: None,
            episode_count: 0,
            unplayed_count: 0,
            latest_episode_at: None,
        }
    }
}
