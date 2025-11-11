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
}

impl FeedRefresher {
    pub fn new(max_concurrent: usize, db: Arc<Database>) -> Self {
        Self {
            fetcher: FeedFetcher::new(),
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            db,
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

                tokio::spawn(async move {
                    let _permit = semaphore
                        .acquire()
                        .await
                        .map_err(|e| anyhow::anyhow!("Semaphore closed: {}", e))?;
                    Self::refresh_feed(fetcher, db, sub).await
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
        Self::refresh_feed(self.fetcher.clone(), self.db.clone(), subscription).await
    }

    async fn refresh_feed(
        fetcher: FeedFetcher,
        db: Arc<Database>,
        sub: Subscription,
    ) -> Result<()> {
        tracing::debug!("Refreshing feed: {}", sub.title);

        let rss_content = fetcher.fetch_feed(&sub.rss_url).await?;
        let channel = FeedParser::parse_channel(&rss_content)?;

        // Parse episodes from the channel
        let episodes = FeedParser::episodes_from_channel(sub.id, &channel);

        // Insert new episodes (database will handle duplicates via UNIQUE constraint)
        for episode in episodes {
            if let Err(e) = db.insert_episode(&episode).await {
                // Log but don't fail - might be duplicate
                tracing::debug!("Failed to insert episode '{}': {}", episode.title, e);
            }
        }

        // Update last refreshed timestamp
        db.update_subscription_last_refreshed(sub.id).await?;

        tracing::info!("Refreshed feed: {}", sub.title);
        Ok(())
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
