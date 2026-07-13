use crate::app::events::{EventBus, StateEvent};
use crate::feed::{FeedFetcher, FeedParser};
use crate::models::Subscription;
use crate::queue::QueueManager;
use crate::storage::Database;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::Semaphore;

/// Global auto-queue policy applied when a refresh finds new episodes.
#[derive(Debug, Clone, Copy)]
pub struct AutoQueuePolicy {
    pub max_depth: usize,
    pub interleave: bool,
}

pub struct FeedRefresher {
    fetcher: FeedFetcher,
    semaphore: Arc<Semaphore>,
    db: Arc<Database>,
    event_bus: Arc<EventBus>,
    queue_manager: Arc<QueueManager>,
    policy: AutoQueuePolicy,
}

impl FeedRefresher {
    pub fn new(
        max_concurrent: usize,
        db: Arc<Database>,
        event_bus: Arc<EventBus>,
        queue_manager: Arc<QueueManager>,
        policy: AutoQueuePolicy,
    ) -> Self {
        Self {
            fetcher: FeedFetcher::new(),
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            db,
            event_bus,
            queue_manager,
            policy,
        }
    }

    pub async fn refresh_all(&self, subscriptions: Vec<Subscription>) -> Result<()> {
        tracing::info!("Refreshing {} subscriptions", subscriptions.len());

        let tasks: Vec<_> = subscriptions
            .into_iter()
            .map(|sub| {
                let semaphore = self.semaphore.clone();
                let fetcher = self.fetcher.clone();
                let db = self.db.clone();
                let event_bus = self.event_bus.clone();
                let queue_manager = self.queue_manager.clone();
                let policy = self.policy;

                tokio::spawn(async move {
                    let _permit = semaphore
                        .acquire()
                        .await
                        .map_err(|e| anyhow::anyhow!("Semaphore closed: {}", e))?;
                    Self::refresh_feed(fetcher, db, event_bus, queue_manager, policy, sub).await
                })
            })
            .collect();

        // Wait for all tasks and collect results
        let mut errors = Vec::new();
        for task in tasks {
            if let Err(e) = task.await? {
                errors.push(e);
            }
        }

        if !errors.is_empty() {
            tracing::warn!("Failed to refresh {} feeds", errors.len());
            for error in &errors {
                tracing::warn!("Refresh error: {}", error);
            }
        }

        Ok(())
    }

    pub async fn refresh_one(&self, subscription: Subscription) -> Result<()> {
        Self::refresh_feed(
            self.fetcher.clone(),
            self.db.clone(),
            self.event_bus.clone(),
            self.queue_manager.clone(),
            self.policy,
            subscription,
        )
        .await
    }

    async fn refresh_feed(
        fetcher: FeedFetcher,
        db: Arc<Database>,
        event_bus: Arc<EventBus>,
        queue_manager: Arc<QueueManager>,
        policy: AutoQueuePolicy,
        sub: Subscription,
    ) -> Result<()> {
        tracing::debug!("Refreshing feed: {}", sub.title);

        // Emit feed refresh started event
        event_bus.publish(StateEvent::FeedRefreshStarted {
            subscription_id: sub.id,
        });

        match async {
            let rss_content = fetcher.fetch_feed(&sub.rss_url).await?;

            // Parse episodes, trying strict RSS then Atom (see parse_episodes).
            let episodes = FeedParser::parse_episodes(sub.id, &rss_content)?;

            // Insert episodes, counting only genuinely-new ones (insert_episode
            // is an upsert, so check existence first). New episodes from an
            // auto-queue feed get enqueued at publish time.
            let mut new_episodes = 0;
            for episode in episodes {
                let is_new = !db
                    .episode_exists(sub.id, &episode.guid)
                    .await
                    .unwrap_or(false);
                db.insert_episode(&episode).await?;

                if is_new {
                    new_episodes += 1;
                    if sub.auto_queue
                        && !episode.played
                        && let Err(e) = queue_manager
                            .auto_enqueue(
                                &episode,
                                sub.auto_queue_to_top,
                                policy.max_depth,
                                policy.interleave,
                            )
                            .await
                    {
                        tracing::warn!("Auto-enqueue failed for '{}': {}", episode.title, e);
                    }
                }
            }

            // Update last refreshed timestamp
            db.update_subscription_last_refreshed(sub.id).await?;

            tracing::info!(
                "Refreshed feed: {} ({} new episodes)",
                sub.title,
                new_episodes
            );

            Ok::<usize, anyhow::Error>(new_episodes)
        }
        .await
        {
            Ok(new_episodes) => {
                // The feed is healthy again: clear any stale error on the row.
                if let Err(e) = db.set_subscription_error(sub.id, None).await {
                    tracing::warn!("Failed to clear feed error for '{}': {}", sub.title, e);
                }
                // Emit feed refresh completed event
                event_bus.publish(StateEvent::FeedRefreshCompleted {
                    subscription_id: sub.id,
                    new_episodes,
                });
                Ok(())
            }
            Err(e) => {
                // Persist why this feed failed so its row can show it (a dead URL,
                // an unparseable feed), not just a transient status that scrolls
                // away. Kept short - the row has limited width.
                let detail = truncate_error(&e.to_string());
                if let Err(db_err) = db.set_subscription_error(sub.id, Some(&detail)).await {
                    tracing::warn!(
                        "Failed to record feed error for '{}': {}",
                        sub.title,
                        db_err
                    );
                }
                // Emit feed refresh failed event
                event_bus.publish(StateEvent::FeedRefreshFailed {
                    subscription_id: sub.id,
                    error: e.to_string(),
                });
                Err(e)
            }
        }
    }
}

/// Shorten a refresh error to something that fits a subscription row's detail
/// line, keeping the leading cause (which names the failure) and dropping the
/// long tail some parse/HTTP errors carry.
fn truncate_error(msg: &str) -> String {
    const MAX: usize = 120;
    let one_line = msg.replace('\n', " ");
    if one_line.chars().count() <= MAX {
        one_line
    } else {
        let kept: String = one_line.chars().take(MAX - 3).collect();
        format!("{kept}...")
    }
}

// Make FeedFetcher cloneable
impl Clone for FeedFetcher {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
        }
    }
}
