use crate::models::{
    DownloadStatus, Episode, PlaybackStatus, QueueItem, QueuePriority, Subscription,
    SubscriptionPriority,
};
use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions};
use sqlx::Row;
use std::path::Path;
use std::str::FromStr;
use uuid::Uuid;

pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn new(db_path: &Path) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", db_path.display()))?
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .context("Failed to connect to database")?;

        let db = Self { pool };

        // Run migrations using sqlx's built-in migration system
        sqlx::migrate!("./migrations")
            .run(&db.pool)
            .await
            .context("Failed to run database migrations")?;

        Ok(db)
    }

    // Subscription methods
    pub async fn insert_subscription(&self, sub: &Subscription) -> Result<()> {
        let categories_json = serde_json::to_string(&sub.categories)?;
        let artwork_path = sub
            .artwork_path
            .as_ref()
            .map(|p| p.to_string_lossy().to_string());

        sqlx::query(
            r#"
            INSERT INTO subscriptions
            (id, title, description, author, rss_url, website_url, artwork_url, artwork_path,
             categories, auto_queue, priority, auto_download, last_refreshed, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(sub.id.to_string())
        .bind(&sub.title)
        .bind(&sub.description)
        .bind(&sub.author)
        .bind(&sub.rss_url)
        .bind(&sub.website_url)
        .bind(&sub.artwork_url)
        .bind(artwork_path)
        .bind(categories_json)
        .bind(sub.auto_queue)
        .bind(sub.priority.as_str())
        .bind(sub.auto_download)
        .bind(sub.last_refreshed)
        .bind(sub.created_at)
        .execute(&self.pool)
        .await
        .context("Failed to insert subscription")?;

        Ok(())
    }

    pub async fn get_all_subscriptions(&self) -> Result<Vec<Subscription>> {
        let rows = sqlx::query(
            r#"
            SELECT id, title, description, author, rss_url, website_url, artwork_url,
                   artwork_path, categories, auto_queue, priority, auto_download,
                   last_refreshed, created_at
            FROM subscriptions
            ORDER BY title
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        let mut subscriptions = Vec::new();
        for row in rows {
            let categories_json: String = row.try_get("categories")?;
            let categories: Vec<String> =
                serde_json::from_str(&categories_json).unwrap_or_default();
            let artwork_path: Option<String> = row.try_get("artwork_path")?;

            subscriptions.push(Subscription {
                id: Uuid::parse_str(&row.try_get::<String, _>("id")?)?,
                title: row.try_get("title")?,
                description: row.try_get("description")?,
                author: row.try_get("author")?,
                rss_url: row.try_get("rss_url")?,
                website_url: row.try_get("website_url")?,
                artwork_url: row.try_get("artwork_url")?,
                artwork_path: artwork_path.map(|p| p.into()),
                categories,
                auto_queue: row.try_get("auto_queue")?,
                priority: SubscriptionPriority::from_str(&row.try_get::<String, _>("priority")?),
                auto_download: row.try_get("auto_download")?,
                last_refreshed: row.try_get("last_refreshed")?,
                created_at: row.try_get("created_at")?,
            });
        }

        Ok(subscriptions)
    }

    pub async fn get_subscription(&self, id: Uuid) -> Result<Option<Subscription>> {
        let row = sqlx::query(
            r#"
            SELECT id, title, description, author, rss_url, website_url, artwork_url,
                   artwork_path, categories, auto_queue, priority, auto_download,
                   last_refreshed, created_at
            FROM subscriptions
            WHERE id = ?
            "#,
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = row {
            let categories_json: String = row.try_get("categories")?;
            let categories: Vec<String> =
                serde_json::from_str(&categories_json).unwrap_or_default();
            let artwork_path: Option<String> = row.try_get("artwork_path")?;

            Ok(Some(Subscription {
                id: Uuid::parse_str(&row.try_get::<String, _>("id")?)?,
                title: row.try_get("title")?,
                description: row.try_get("description")?,
                author: row.try_get("author")?,
                rss_url: row.try_get("rss_url")?,
                website_url: row.try_get("website_url")?,
                artwork_url: row.try_get("artwork_url")?,
                artwork_path: artwork_path.map(|p| p.into()),
                categories,
                auto_queue: row.try_get("auto_queue")?,
                priority: SubscriptionPriority::from_str(&row.try_get::<String, _>("priority")?),
                auto_download: row.try_get("auto_download")?,
                last_refreshed: row.try_get("last_refreshed")?,
                created_at: row.try_get("created_at")?,
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn update_subscription_last_refreshed(&self, id: Uuid) -> Result<()> {
        sqlx::query("UPDATE subscriptions SET last_refreshed = ? WHERE id = ?")
            .bind(Utc::now())
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_subscription(&self, id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM subscriptions WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // Episode methods
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

    fn parse_episodes(&self, rows: Vec<sqlx::sqlite::SqliteRow>) -> Result<Vec<Episode>> {
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
                download_status: DownloadStatus::from_str(
                    &row.try_get::<String, _>("download_status")?,
                ),
                local_path: local_path.map(|p| p.into()),
                playback_position_seconds: row.try_get("playback_position_seconds")?,
                played: row.try_get("played")?,
                last_played_at: row.try_get("last_played_at")?,
                created_at: row.try_get("created_at")?,
            });
        }
        Ok(episodes)
    }

    // Queue methods
    pub async fn add_to_queue(&self, item: &QueueItem) -> Result<()> {
        sqlx::query(
            "INSERT INTO queue_items (id, episode_id, position, priority, added_at) VALUES (?, ?, ?, ?, ?)"
        )
        .bind(item.id.to_string())
        .bind(item.episode_id.to_string())
        .bind(item.position)
        .bind(item.priority.as_str())
        .bind(item.added_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn get_queue(&self) -> Result<Vec<QueueItem>> {
        let rows = sqlx::query(
            "SELECT id, episode_id, position, priority, added_at FROM queue_items ORDER BY position"
        )
        .fetch_all(&self.pool)
        .await?;

        let mut items = Vec::new();
        for row in rows {
            items.push(QueueItem {
                id: Uuid::parse_str(&row.try_get::<String, _>("id")?)?,
                episode_id: Uuid::parse_str(&row.try_get::<String, _>("episode_id")?)?,
                position: row.try_get("position")?,
                priority: QueuePriority::from_str(&row.try_get::<String, _>("priority")?),
                added_at: row.try_get("added_at")?,
            });
        }
        Ok(items)
    }

    pub async fn remove_from_queue(&self, episode_id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM queue_items WHERE episode_id = ?")
            .bind(episode_id.to_string())
            .execute(&self.pool)
            .await?;

        // Reorder remaining items
        self.reorder_queue().await?;
        Ok(())
    }

    pub async fn reorder_queue_item(&self, episode_id: Uuid, new_position: i64) -> Result<()> {
        // First, update the position
        sqlx::query("UPDATE queue_items SET position = ? WHERE episode_id = ?")
            .bind(new_position)
            .bind(episode_id.to_string())
            .execute(&self.pool)
            .await?;

        // Then reorder all items
        self.reorder_queue().await?;
        Ok(())
    }

    async fn reorder_queue(&self) -> Result<()> {
        // Get all queue items
        let items = self.get_queue().await?;

        // Update positions to be sequential
        for (index, item) in items.iter().enumerate() {
            sqlx::query("UPDATE queue_items SET position = ? WHERE id = ?")
                .bind(index as i64)
                .bind(item.id.to_string())
                .execute(&self.pool)
                .await?;
        }
        Ok(())
    }

    pub async fn clear_queue(&self) -> Result<()> {
        sqlx::query("DELETE FROM queue_items")
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // Playback state methods
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
            status: PlaybackStatus::from_str(&row.try_get::<String, _>("status")?),
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn create_test_db() -> (Database, TempDir) {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let db = Database::new(&db_path).await.unwrap();
        (db, temp_dir)
    }

    #[tokio::test]
    async fn test_migrations_run_successfully() {
        let (db, _temp) = create_test_db().await;

        // Verify _sqlx_migrations table exists
        let result: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM _sqlx_migrations")
            .fetch_one(&db.pool)
            .await
            .expect("_sqlx_migrations table should exist");

        assert!(result.0 > 0, "Should have at least one migration");
    }

    #[tokio::test]
    async fn test_migrations_are_idempotent() {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");

        // First run
        let db1 = Database::new(&db_path).await.unwrap();

        // Verify initial migration ran
        let count1: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM _sqlx_migrations")
            .fetch_one(&db1.pool)
            .await
            .unwrap();

        drop(db1);

        // Second run - should not error
        let db2 = Database::new(&db_path).await.unwrap();

        // Verify same number of migrations
        let count2: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM _sqlx_migrations")
            .fetch_one(&db2.pool)
            .await
            .unwrap();

        assert_eq!(count1.0, count2.0, "Migration count should be stable");
    }

    #[tokio::test]
    async fn test_schema_created_correctly() {
        let (db, _temp) = create_test_db().await;

        // Verify all expected tables exist
        let tables: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_sqlx_%' ORDER BY name"
        )
        .fetch_all(&db.pool)
        .await
        .unwrap();

        let table_names: Vec<String> = tables.into_iter().map(|(name,)| name).collect();

        assert!(table_names.contains(&"subscriptions".to_string()));
        assert!(table_names.contains(&"episodes".to_string()));
        assert!(table_names.contains(&"queue_items".to_string()));
        assert!(table_names.contains(&"playback_state".to_string()));
        assert!(table_names.contains(&"config".to_string()));
    }
}
