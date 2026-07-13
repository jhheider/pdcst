use crate::models::Subscription;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::Row;
use sqlx::sqlite::SqliteRow;
use uuid::Uuid;

use super::Database;

/// The subscription-list query: every subscription with its joined episode stats
/// (`episode_count`, `new_count`, `latest_episode_at`), ordered by title.
const SELECT_ALL_SUBSCRIPTIONS: &str = r#"
    SELECT s.id, s.title, s.description, s.author, s.rss_url, s.website_url,
           s.artwork_url, s.artwork_path, s.categories, s.auto_queue,
           s.auto_queue_to_top, s.queue_oldest_first, s.priority, s.auto_download, s.last_refreshed,
           s.created_at, s.last_error,
           COUNT(e.id) AS episode_count,
           COALESCE(SUM(CASE WHEN e.is_new = 1 AND e.played = 0 THEN 1 ELSE 0 END), 0) AS new_count,
           MAX(e.published_at) AS latest_episode_at
    FROM subscriptions s
    LEFT JOIN episodes e ON e.subscription_id = s.id
    GROUP BY s.id
    ORDER BY s.title
"#;

/// The same projection as [`SELECT_ALL_SUBSCRIPTIONS`] for a single subscription.
const SELECT_ONE_SUBSCRIPTION: &str = r#"
    SELECT s.id, s.title, s.description, s.author, s.rss_url, s.website_url,
           s.artwork_url, s.artwork_path, s.categories, s.auto_queue,
           s.auto_queue_to_top, s.queue_oldest_first, s.priority, s.auto_download, s.last_refreshed,
           s.created_at, s.last_error,
           COUNT(e.id) AS episode_count,
           COALESCE(SUM(CASE WHEN e.is_new = 1 AND e.played = 0 THEN 1 ELSE 0 END), 0) AS new_count,
           MAX(e.published_at) AS latest_episode_at
    FROM subscriptions s
    LEFT JOIN episodes e ON e.subscription_id = s.id
    WHERE s.id = ?
    GROUP BY s.id
"#;

fn row_to_subscription(row: &SqliteRow) -> Result<Subscription> {
    let categories_json: String = row.try_get("categories")?;
    let categories: Vec<String> = serde_json::from_str(&categories_json).unwrap_or_default();
    let artwork_path: Option<String> = row.try_get("artwork_path")?;

    Ok(Subscription {
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
        auto_queue_to_top: row.try_get("auto_queue_to_top")?,
        queue_oldest_first: row.try_get("queue_oldest_first")?,
        priority: row.try_get::<String, _>("priority")?.parse().unwrap(),
        auto_download: row.try_get("auto_download")?,
        last_refreshed: row.try_get("last_refreshed")?,
        created_at: row.try_get("created_at")?,
        last_error: row.try_get("last_error")?,
        episode_count: row.try_get("episode_count")?,
        new_count: row.try_get("new_count")?,
        latest_episode_at: row.try_get::<Option<DateTime<Utc>>, _>("latest_episode_at")?,
    })
}

impl Database {
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
             categories, auto_queue, auto_queue_to_top, queue_oldest_first, priority, auto_download,
             last_refreshed, created_at, last_error)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
        .bind(sub.auto_queue_to_top)
        .bind(sub.queue_oldest_first)
        .bind(sub.priority.as_str())
        .bind(sub.auto_download)
        .bind(sub.last_refreshed)
        .bind(sub.created_at)
        .bind(&sub.last_error)
        .execute(&self.pool)
        .await
        .context("Failed to insert subscription")?;

        Ok(())
    }

    pub async fn get_all_subscriptions(&self) -> Result<Vec<Subscription>> {
        let rows = sqlx::query(SELECT_ALL_SUBSCRIPTIONS)
            .fetch_all(&self.pool)
            .await?;

        rows.iter().map(row_to_subscription).collect()
    }

    pub async fn get_subscription(&self, id: Uuid) -> Result<Option<Subscription>> {
        let row = sqlx::query(SELECT_ONE_SUBSCRIPTION)
            .bind(id.to_string())
            .fetch_optional(&self.pool)
            .await?;

        row.as_ref().map(row_to_subscription).transpose()
    }

    /// Record (or clear, with `None`) the most recent refresh failure for a feed.
    /// Cleared on a successful refresh, set on a failed one, so the row can show
    /// why a feed is not updating.
    pub async fn set_subscription_error(&self, id: Uuid, error: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE subscriptions SET last_error = ? WHERE id = ?")
            .bind(error)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Update a subscription's auto-queue setting (on/off and direction).
    pub async fn update_subscription_auto_queue(
        &self,
        id: Uuid,
        auto_queue: bool,
        to_top: bool,
    ) -> Result<()> {
        sqlx::query("UPDATE subscriptions SET auto_queue = ?, auto_queue_to_top = ? WHERE id = ?")
            .bind(auto_queue)
            .bind(to_top)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Set a subscription's episode order (false = newest-first, true =
    /// oldest-first).
    pub async fn update_subscription_queue_order(
        &self,
        id: Uuid,
        oldest_first: bool,
    ) -> Result<()> {
        sqlx::query("UPDATE subscriptions SET queue_oldest_first = ? WHERE id = ?")
            .bind(oldest_first)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Re-point a subscription at a new feed URL (feed migration / recovery).
    /// `rss_url` is UNIQUE, so this errors if the URL already belongs to another
    /// subscription; the caller surfaces that rather than silently merging feeds.
    pub async fn update_subscription_rss_url(&self, id: Uuid, rss_url: &str) -> Result<()> {
        sqlx::query("UPDATE subscriptions SET rss_url = ?, last_error = NULL WHERE id = ?")
            .bind(rss_url)
            .bind(id.to_string())
            .execute(&self.pool)
            .await
            .context("Failed to update feed URL (is it already subscribed?)")?;
        Ok(())
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
}
