use crate::app::events::{EventBus, StateEvent};
use crate::feed::{FeedFetcher, FeedParser};
use crate::models::Subscription;
use crate::storage::Database;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::Semaphore;

pub struct FeedRefresher {
    fetcher: FeedFetcher,
    semaphore: Arc<Semaphore>,
    db: Arc<Database>,
    event_bus: Arc<EventBus>,
}

impl FeedRefresher {
    pub fn new(max_concurrent: usize, db: Arc<Database>, event_bus: Arc<EventBus>) -> Self {
        Self {
            fetcher: FeedFetcher::new(),
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            db,
            event_bus,
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

                tokio::spawn(async move {
                    let _permit = semaphore
                        .acquire()
                        .await
                        .map_err(|e| anyhow::anyhow!("Semaphore closed: {}", e))?;
                    Self::refresh_feed(fetcher, db, event_bus, sub).await
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
        Self::refresh_feed(self.fetcher.clone(), self.db.clone(), self.event_bus.clone(), subscription).await
    }

    async fn refresh_feed(
        fetcher: FeedFetcher,
        db: Arc<Database>,
        event_bus: Arc<EventBus>,
        sub: Subscription,
    ) -> Result<()> {
        tracing::debug!("Refreshing feed: {}", sub.title);

        // Emit feed refresh started event
        event_bus.publish(StateEvent::FeedRefreshStarted { subscription_id: sub.id });

        match async {
            let rss_content = fetcher.fetch_feed(&sub.rss_url).await?;
            let channel = FeedParser::parse_channel(&rss_content)?;

            // Parse episodes from the channel
            let episodes = FeedParser::episodes_from_channel(sub.id, &channel);

            // Insert new episodes (database will handle duplicates via UNIQUE constraint)
            let mut new_episodes = 0;
            for episode in episodes {
                if db.insert_episode(&episode).await.is_ok() {
                    new_episodes += 1;
                }
            }

            // Update last refreshed timestamp
            db.update_subscription_last_refreshed(sub.id).await?;

            tracing::info!("Refreshed feed: {} ({} new episodes)", sub.title, new_episodes);

            Ok::<usize, anyhow::Error>(new_episodes)
        }.await {
            Ok(new_episodes) => {
                // Emit feed refresh completed event
                event_bus.publish(StateEvent::FeedRefreshCompleted {
                    subscription_id: sub.id,
                    new_episodes,
                });
                Ok(())
            }
            Err(e) => {
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

// Make FeedFetcher cloneable
impl Clone for FeedFetcher {
    fn clone(&self) -> Self {
        Self {
            client: self.client.clone(),
        }
    }
}
