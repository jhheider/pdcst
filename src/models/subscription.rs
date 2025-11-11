use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
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

    pub fn from_str(s: &str) -> Self {
        match s {
            "High" => Self::High,
            "Low" => Self::Low,
            _ => Self::Medium,
        }
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
    pub auto_queue: bool,
    pub priority: SubscriptionPriority,
    pub auto_download: bool,
    pub last_refreshed: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
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
            priority: SubscriptionPriority::Medium,
            auto_download: false,
            last_refreshed: now,
            created_at: now,
        }
    }
}
