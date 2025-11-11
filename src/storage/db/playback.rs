use crate::models::PlaybackStatus;
use anyhow::Result;
use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use super::Database;

#[derive(Debug, Clone)]
pub struct PlaybackState {
    pub current_episode_id: Option<Uuid>,
    pub position_seconds: f64,
    pub playback_rate: f32,
    pub volume: f32,
    pub status: PlaybackStatus,
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            current_episode_id: None,
            position_seconds: 0.0,
            playback_rate: 1.0,
            volume: 1.0,
            status: PlaybackStatus::Stopped,
        }
    }
}

impl Database {
    pub async fn get_playback_state(&self) -> Result<PlaybackState> {
        let row = sqlx::query(
            "SELECT current_episode_id, position_seconds, playback_rate, volume, status FROM playback_state WHERE id = 1"
        )
        .fetch_one(&self.pool)
        .await?;

        let episode_id: Option<String> = row.try_get("current_episode_id")?;
        Ok(PlaybackState {
            current_episode_id: episode_id.and_then(|s| Uuid::parse_str(&s).ok()),
            position_seconds: row.try_get("position_seconds")?,
            playback_rate: row.try_get("playback_rate")?,
            volume: row.try_get("volume")?,
            status: row.try_get::<String, _>("status")?.parse().unwrap(),
        })
    }

    pub async fn update_playback_state(&self, state: &PlaybackState) -> Result<()> {
        let episode_id = state.current_episode_id.map(|id| id.to_string());
        sqlx::query(
            r#"
            UPDATE playback_state
            SET current_episode_id = ?, position_seconds = ?, playback_rate = ?, volume = ?, status = ?, updated_at = ?
            WHERE id = 1
            "#
        )
        .bind(episode_id)
        .bind(state.position_seconds)
        .bind(state.playback_rate)
        .bind(state.volume)
        .bind(state.status.as_str())
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
