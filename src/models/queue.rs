use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum QueuePriority {
    High,
    Medium,
    Low,
}

impl QueuePriority {
    pub fn as_str(&self) -> &str {
        match self {
            Self::High => "High",
            Self::Medium => "Medium",
            Self::Low => "Low",
        }
    }
}

impl FromStr for QueuePriority {
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
pub struct QueueItem {
    pub id: Uuid,
    pub episode_id: Uuid,
    pub position: i64,
    pub priority: QueuePriority,
    pub added_at: DateTime<Utc>,
}

impl QueueItem {
    pub fn new(episode_id: Uuid, position: i64) -> Self {
        Self {
            id: Uuid::new_v4(),
            episode_id,
            position,
            priority: QueuePriority::Medium,
            added_at: Utc::now(),
        }
    }
}
