use crate::models::Subscription;
use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::Row;
use uuid::Uuid;

use super::Database;

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
             categories, auto_queue, auto_queue_to_top, priority, auto_download,
             last_refreshed, created_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
                   artwork_path, categories, auto_queue, auto_queue_to_top, priority,
                   auto_download, last_refreshed, created_at
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
                auto_queue_to_top: row.try_get("auto_queue_to_top")?,
                priority: row.try_get::<String, _>("priority")?.parse().unwrap(),
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
                   artwork_path, categories, auto_queue, auto_queue_to_top, priority,
                   auto_download, last_refreshed, created_at
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
                auto_queue_to_top: row.try_get("auto_queue_to_top")?,
                priority: row.try_get::<String, _>("priority")?.parse().unwrap(),
                auto_download: row.try_get("auto_download")?,
                last_refreshed: row.try_get("last_refreshed")?,
                created_at: row.try_get("created_at")?,
            }))
        } else {
            Ok(None)
        }
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
