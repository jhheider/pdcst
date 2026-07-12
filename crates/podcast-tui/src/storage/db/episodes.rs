use crate::models::{DownloadStatus, Episode};
use anyhow::Result;
use chrono::Utc;
use sqlx::Row;
use std::path::Path;
use uuid::Uuid;

use super::Database;

impl Database {
    pub async fn insert_episode(&self, episode: &Episode) -> Result<()> {
        let local_path = episode
            .local_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string());

        sqlx::query(
            r#"
            INSERT OR REPLACE INTO episodes
            (id, subscription_id, title, description, url, guid, published_at,
             duration_seconds, file_size_bytes, file_type, download_status, local_path,
             playback_position_seconds, played, last_played_at, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(episode.id.to_string())
        .bind(episode.subscription_id.to_string())
        .bind(&episode.title)
        .bind(&episode.description)
        .bind(&episode.url)
        .bind(&episode.guid)
        .bind(episode.published_at)
        .bind(episode.duration_seconds)
        .bind(episode.file_size_bytes)
        .bind(&episode.file_type)
        .bind(episode.download_status.as_str())
        .bind(local_path)
        .bind(episode.playback_position_seconds)
        .bind(episode.played)
        .bind(episode.last_played_at)
        .bind(episode.created_at)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn get_episodes_for_subscription(
        &self,
        subscription_id: Uuid,
    ) -> Result<Vec<Episode>> {
        let rows = sqlx::query(
            r#"
            SELECT id, subscription_id, title, description, url, guid, published_at,
                   duration_seconds, file_size_bytes, file_type, download_status, local_path,
                   playback_position_seconds, played, last_played_at, created_at
            FROM episodes
            WHERE subscription_id = ?
            ORDER BY published_at DESC
            "#,
        )
        .bind(subscription_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        self.parse_episodes(rows)
    }

    /// All episodes with a completed download on disk, oldest activity first
    /// (least-recently-played, then oldest created). Used by cache retention to
    /// evict the least-useful downloads first.
    pub async fn get_downloaded_episodes(&self) -> Result<Vec<Episode>> {
        let rows = sqlx::query(
            r#"
            SELECT id, subscription_id, title, description, url, guid, published_at,
                   duration_seconds, file_size_bytes, file_type, download_status, local_path,
                   playback_position_seconds, played, last_played_at, created_at
            FROM episodes
            WHERE download_status = ?
            ORDER BY COALESCE(last_played_at, created_at) ASC
            "#,
        )
        .bind(DownloadStatus::Downloaded.as_str())
        .fetch_all(&self.pool)
        .await?;

        self.parse_episodes(rows)
    }

    /// Whether an episode with this guid already exists for the subscription.
    /// Used to tell a genuinely-new episode from a re-seen one on refresh (the
    /// `INSERT OR REPLACE` in `insert_episode` cannot distinguish them).
    pub async fn episode_exists(&self, subscription_id: Uuid, guid: &str) -> Result<bool> {
        let row =
            sqlx::query("SELECT 1 FROM episodes WHERE subscription_id = ? AND guid = ? LIMIT 1")
                .bind(subscription_id.to_string())
                .bind(guid)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.is_some())
    }

    pub async fn get_episode(&self, id: Uuid) -> Result<Option<Episode>> {
        let row = sqlx::query(
            r#"
            SELECT id, subscription_id, title, description, url, guid, published_at,
                   duration_seconds, file_size_bytes, file_type, download_status, local_path,
                   playback_position_seconds, played, last_played_at, created_at
            FROM episodes
            WHERE id = ?
            "#,
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = row {
            let episodes = self.parse_episodes(vec![row])?;
            Ok(episodes.into_iter().next())
        } else {
            Ok(None)
        }
    }

    pub async fn update_episode_playback_position(
        &self,
        id: Uuid,
        position_seconds: i64,
    ) -> Result<()> {
        sqlx::query(
            "UPDATE episodes SET playback_position_seconds = ?, last_played_at = ? WHERE id = ?",
        )
        .bind(position_seconds)
        .bind(Utc::now())
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn mark_episode_played(&self, id: Uuid, played: bool) -> Result<()> {
        sqlx::query("UPDATE episodes SET played = ? WHERE id = ?")
            .bind(played)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn update_episode_download_status(
        &self,
        id: Uuid,
        status: DownloadStatus,
        local_path: Option<&Path>,
    ) -> Result<()> {
        let path_str = local_path.map(|p| p.to_string_lossy().to_string());
        sqlx::query("UPDATE episodes SET download_status = ?, local_path = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(path_str)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub(super) fn parse_episodes(
        &self,
        rows: Vec<sqlx::sqlite::SqliteRow>,
    ) -> Result<Vec<Episode>> {
        let mut episodes = Vec::new();
        for row in rows {
            let local_path: Option<String> = row.try_get("local_path")?;
            episodes.push(Episode {
                id: Uuid::parse_str(&row.try_get::<String, _>("id")?)?,
                subscription_id: Uuid::parse_str(&row.try_get::<String, _>("subscription_id")?)?,
                title: row.try_get("title")?,
                description: row.try_get("description")?,
                url: row.try_get("url")?,
                guid: row.try_get("guid")?,
                published_at: row.try_get("published_at")?,
                duration_seconds: row.try_get("duration_seconds")?,
                file_size_bytes: row.try_get("file_size_bytes")?,
                file_type: row.try_get("file_type")?,
                download_status: row
                    .try_get::<String, _>("download_status")?
                    .parse()
                    .unwrap(),
                local_path: local_path.map(|p| p.into()),
                playback_position_seconds: row.try_get("playback_position_seconds")?,
                played: row.try_get("played")?,
                last_played_at: row.try_get("last_played_at")?,
                created_at: row.try_get("created_at")?,
            });
        }
        Ok(episodes)
    }
}
