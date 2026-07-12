use crate::app::events::{EventBus, StateEvent};
use crate::models::{Episode, QueueItem};
use crate::storage::Database;
use anyhow::Result;
use std::sync::Arc;
use uuid::Uuid;

pub struct QueueManager {
    db: Arc<Database>,
    event_bus: Arc<EventBus>,
}

impl QueueManager {
    pub fn new(db: Arc<Database>, event_bus: Arc<EventBus>) -> Self {
        Self { db, event_bus }
    }

    pub async fn add_episode(&self, episode_id: Uuid) -> Result<()> {
        // Get current queue to determine position
        let queue = self.db.get_queue().await?;
        let position = queue.len() as i64;

        let item = QueueItem::new(episode_id, position);
        self.db.add_to_queue(&item).await?;

        tracing::info!(
            "Added episode {} to queue at position {}",
            episode_id,
            position
        );

        // Emit queue updated event
        self.event_bus.publish(StateEvent::QueueUpdated);

        Ok(())
    }

    pub async fn remove_episode(&self, episode_id: Uuid) -> Result<()> {
        self.db.remove_from_queue(episode_id).await?;
        tracing::info!("Removed episode {} from queue", episode_id);

        // Emit queue updated event
        self.event_bus.publish(StateEvent::QueueUpdated);

        Ok(())
    }

    pub async fn move_episode(&self, episode_id: Uuid, new_position: i64) -> Result<()> {
        self.db.reorder_queue_item(episode_id, new_position).await?;
        tracing::info!("Moved episode {} to position {}", episode_id, new_position);

        // Emit queue updated event
        self.event_bus.publish(StateEvent::QueueUpdated);

        Ok(())
    }

    pub async fn move_up(&self, episode_id: Uuid) -> Result<()> {
        let queue = self.db.get_queue().await?;

        if let Some((index, _item)) = queue
            .iter()
            .enumerate()
            .find(|(_, item)| item.episode_id == episode_id)
            && index > 0
        {
            let new_position = index as i64 - 1;
            self.move_episode(episode_id, new_position).await?;
        }

        Ok(())
    }

    pub async fn move_down(&self, episode_id: Uuid) -> Result<()> {
        let queue = self.db.get_queue().await?;

        if let Some((index, _item)) = queue
            .iter()
            .enumerate()
            .find(|(_, item)| item.episode_id == episode_id)
            && index < queue.len() - 1
        {
            let new_position = index as i64 + 1;
            self.move_episode(episode_id, new_position).await?;
        }

        Ok(())
    }

    pub async fn get_queue(&self) -> Result<Vec<QueueItem>> {
        self.db.get_queue().await
    }

    pub async fn get_next(&self) -> Result<Option<QueueItem>> {
        let queue = self.db.get_queue().await?;
        Ok(queue.into_iter().next())
    }

    pub async fn clear(&self) -> Result<()> {
        self.db.clear_queue().await?;
        tracing::info!("Cleared queue");

        // Emit queue updated event
        self.event_bus.publish(StateEvent::QueueUpdated);

        Ok(())
    }

    pub async fn get_queue_with_episodes(&self) -> Result<Vec<(QueueItem, Episode)>> {
        let queue = self.get_queue().await?;
        let mut result = Vec::new();

        for item in queue {
            if let Some(episode) = self.db.get_episode(item.episode_id).await? {
                result.push((item, episode));
            }
        }

        Ok(result)
    }
}
