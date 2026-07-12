//! Background refresh scheduling.
//!
//! Refreshes every feed once shortly after launch, then on a programmable
//! interval. The refresh is concurrency-bounded inside the [`FeedRefresher`],
//! publishes the usual `FeedRefresh*` events (so the UI shows progress), and
//! feeds the Phase C auto-queue: any new episode it ingests for an auto-queue
//! feed is enqueued at publish time.

use crate::feed::FeedRefresher;
use crate::storage::Database;
use std::sync::Arc;
use std::time::Duration;

/// Delay before the launch refresh, so the UI draws and startup settles first.
const LAUNCH_DELAY: Duration = Duration::from_secs(3);

/// Spawn the auto-refresh task: refresh all feeds once at launch, then every
/// `interval_minutes`. A `0` interval disables the periodic refresh (the launch
/// refresh still runs). Returns immediately.
pub fn spawn_auto_refresh(refresher: Arc<FeedRefresher>, db: Arc<Database>, interval_minutes: u64) {
    tokio::spawn(async move {
        tokio::time::sleep(LAUNCH_DELAY).await;
        refresh_all(&refresher, &db).await;

        if interval_minutes == 0 {
            tracing::info!("Periodic auto-refresh disabled (interval 0)");
            return;
        }

        let mut ticker = tokio::time::interval(Duration::from_secs(interval_minutes * 60));
        ticker.tick().await; // the first tick fires immediately; the launch
        // refresh above already covered it.
        loop {
            ticker.tick().await;
            refresh_all(&refresher, &db).await;
        }
    });
}

async fn refresh_all(refresher: &FeedRefresher, db: &Database) {
    match db.get_all_subscriptions().await {
        Ok(subs) if !subs.is_empty() => {
            tracing::info!("Auto-refresh: refreshing {} feed(s)", subs.len());
            if let Err(e) = refresher.refresh_all(subs).await {
                tracing::warn!("Auto-refresh failed: {}", e);
            }
        }
        Ok(_) => {} // no subscriptions yet
        Err(e) => tracing::warn!("Auto-refresh: could not list subscriptions: {}", e),
    }
}
