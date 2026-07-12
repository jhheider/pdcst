use crate::app::events::{EventBus, StateEvent};
use crate::models::{Episode, QueueItem};
use crate::queue::ordering::nearest_legal_position;
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

    /// Auto-add a newly-published episode to the queue (the publish-time hook).
    ///
    /// - `to_top`: unshift to the front vs push to the back.
    /// - `max_depth`: skip if the queue already holds this many (0 = unlimited).
    /// - `interleave`: avoid placing two episodes of the same podcast adjacent.
    ///
    /// The currently-playing episode (the queue head, when the player has it
    /// loaded) is never displaced: auto-fill happens around it. Returns whether
    /// the episode was enqueued.
    pub async fn auto_enqueue(
        &self,
        episode: &Episode,
        to_top: bool,
        max_depth: usize,
        interleave: bool,
    ) -> Result<bool> {
        let entries = self.get_queue_with_episodes().await?;

        if max_depth > 0 && entries.len() >= max_depth {
            return Ok(false); // queue full: do not exceed the cap
        }
        if entries.iter().any(|(_, ep)| ep.id == episode.id) {
            return Ok(false); // already queued
        }

        // Protect the current item: if the queue head is what the player has
        // loaded, never insert before it.
        let protect_head = match entries.first() {
            Some((head, _)) => {
                self.db
                    .get_playback_state()
                    .await
                    .ok()
                    .and_then(|state| state.current_episode_id)
                    == Some(head.episode_id)
            }
            None => false,
        };
        let floor = usize::from(protect_head);

        let subs: Vec<Uuid> = entries.iter().map(|(_, ep)| ep.subscription_id).collect();
        let mut position = if to_top { floor } else { subs.len() };
        if interleave {
            position =
                nearest_legal_position(&subs, episode.subscription_id, position, floor, to_top);
        }

        let item = QueueItem::new(episode.id, position as i64);
        self.db.insert_into_queue_at(&item, position as i64).await?;
        tracing::info!("Auto-enqueued '{}' at position {}", episode.title, position);

        self.event_bus.publish(StateEvent::QueueUpdated);
        Ok(true)
    }

    /// Advance the queue past `finished_episode_id`: optionally mark it played,
    /// remove it from the queue, and return the next episode to play (if any).
    ///
    /// This is the single path shared by natural completion (mark played) and
    /// manual skip / retry-on-failure (with or without marking played). The
    /// caller is responsible for actually playing the returned episode, since
    /// playback needs the audio player, which the queue does not own.
    pub async fn advance(
        &self,
        finished_episode_id: Uuid,
        mark_played: bool,
    ) -> Result<Option<Episode>> {
        if mark_played {
            self.db
                .mark_episode_played(finished_episode_id, true)
                .await?;
            self.event_bus.publish(StateEvent::EpisodeMarkedPlayed {
                episode_id: finished_episode_id,
            });
        }

        // Removes the finished episode and normalizes positions (publishes
        // QueueUpdated). A no-op if it was not in the queue (e.g. played from
        // the Episodes view).
        self.remove_episode(finished_episode_id).await?;

        match self.get_next().await? {
            Some(item) => Ok(self.db.get_episode(item.episode_id).await?),
            None => Ok(None),
        }
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
