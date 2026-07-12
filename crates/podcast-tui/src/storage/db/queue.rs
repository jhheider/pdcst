use crate::models::QueueItem;
use anyhow::Result;
use sqlx::Row;
use uuid::Uuid;

use super::Database;

impl Database {
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

    /// Insert an item at a logical `position`, shifting everything at or after
    /// it down by one so the queue stays contiguous and ordered. Used by
    /// auto-enqueue to unshift (top) or insert mid-queue for interleaving.
    pub async fn insert_into_queue_at(&self, item: &QueueItem, position: i64) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("UPDATE queue_items SET position = position + 1 WHERE position >= ?")
            .bind(position)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO queue_items (id, episode_id, position, priority, added_at) VALUES (?, ?, ?, ?, ?)",
        )
        .bind(item.id.to_string())
        .bind(item.episode_id.to_string())
        .bind(position)
        .bind(item.priority.as_str())
        .bind(item.added_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
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
                priority: row.try_get::<String, _>("priority")?.parse().unwrap(),
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
}
