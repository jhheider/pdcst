//! Cache retention: keep on-disk audio from growing without bound.
//!
//! Two policies, both configurable:
//! - **Delete on finish** (default on): when an episode finishes playing, its
//!   downloaded file is removed. That lives in the completion path
//!   (`app/mod.rs`); this module owns the size caps below.
//! - **Size caps**: at most `max_cache_episodes` downloaded episodes and
//!   `max_cache_megabytes` of audio on disk. The least-recently-played downloads
//!   are evicted first; the currently-playing episode is never evicted. A cap of
//!   0 means unlimited.
//!
//! Enforced once at startup (which also clears stale stream temp files) and then
//! periodically, so a session left open for hours or days cannot drift over the
//! caps.

use crate::audio::{AudioPlayer, stream};
use crate::download::DownloadManager;
use crate::models::DownloadStatus;
use crate::storage::Database;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// How often the background task re-checks the caps while the app stays open.
const SWEEP_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

const BYTES_PER_MB: u64 = 1024 * 1024;

/// Enforces the on-disk cache retention policy (see module docs).
pub struct RetentionManager {
    db: Arc<Database>,
    download_manager: Arc<DownloadManager>,
    audio_player: Arc<AudioPlayer>,
    stream_cache_dir: PathBuf,
    /// Max number of downloaded episodes to keep; 0 = unlimited.
    max_episodes: usize,
    /// Max total bytes of downloaded audio to keep; 0 = unlimited.
    max_bytes: u64,
}

impl RetentionManager {
    pub fn new(
        db: Arc<Database>,
        download_manager: Arc<DownloadManager>,
        audio_player: Arc<AudioPlayer>,
        stream_cache_dir: PathBuf,
        max_cache_episodes: usize,
        max_cache_megabytes: u64,
    ) -> Self {
        Self {
            db,
            download_manager,
            audio_player,
            stream_cache_dir,
            max_episodes: max_cache_episodes,
            max_bytes: max_cache_megabytes.saturating_mul(BYTES_PER_MB),
        }
    }

    /// Startup cleanup: clear stale stream temp files (nothing is streaming yet)
    /// and bring downloads under the size caps.
    pub async fn run_startup(&self) {
        stream::purge_all(&self.stream_cache_dir);
        self.enforce().await;
    }

    /// Spawn a background task that re-enforces the caps every [`SWEEP_INTERVAL`]
    /// so a long-running session cannot drift over them.
    pub fn spawn_periodic(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
            ticker.tick().await; // the first tick fires immediately; skip it
            loop {
                ticker.tick().await;
                self.enforce().await;
            }
        });
    }

    /// Evict downloaded episodes, least-recently-played first, until both caps
    /// are satisfied. Never evicts the currently-playing episode, and reconciles
    /// DB rows whose file has vanished.
    pub async fn enforce(&self) {
        if self.max_episodes == 0 && self.max_bytes == 0 {
            return; // both unlimited: nothing to enforce
        }

        let downloaded = match self.db.get_downloaded_episodes().await {
            Ok(eps) => eps,
            Err(e) => {
                tracing::warn!("retention: failed to list downloads: {e}");
                return;
            }
        };

        let protected = self.audio_player.get_current_episode().await;

        // Measure real on-disk sizes; reconcile rows whose file is gone.
        let mut entries = Vec::new();
        let mut total_bytes = 0u64;
        for episode in downloaded {
            let Some(path) = episode.local_path.clone() else {
                continue;
            };
            match tokio::fs::metadata(&path).await {
                Ok(meta) => {
                    total_bytes += meta.len();
                    entries.push((episode, meta.len()));
                }
                Err(_) => {
                    tracing::debug!(
                        "retention: '{}' marked downloaded but file missing, reconciling",
                        episode.title
                    );
                    let _ = self
                        .db
                        .update_episode_download_status(
                            episode.id,
                            DownloadStatus::NotDownloaded,
                            None,
                        )
                        .await;
                }
            }
        }

        // get_downloaded_episodes returns oldest-activity first, so entries are
        // already in eviction order.
        let mut count = entries.len();
        let mut evicted = 0usize;
        for (episode, size) in entries {
            let over_count = self.max_episodes > 0 && count > self.max_episodes;
            let over_bytes = self.max_bytes > 0 && total_bytes > self.max_bytes;
            if !over_count && !over_bytes {
                break;
            }
            if Some(episode.id) == protected {
                continue; // never evict what is playing
            }
            match self.download_manager.delete_download(&episode).await {
                Ok(()) => {
                    total_bytes = total_bytes.saturating_sub(size);
                    count = count.saturating_sub(1);
                    evicted += 1;
                    tracing::info!("retention: evicted download '{}'", episode.title);
                }
                Err(e) => {
                    tracing::warn!("retention: failed to evict '{}': {e}", episode.title);
                }
            }
        }

        if evicted > 0 {
            tracing::info!(
                "retention: evicted {evicted} download(s); ~{} MiB, {count} episode(s) now cached",
                total_bytes / BYTES_PER_MB
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::events::EventBus;
    use crate::models::{Episode, Subscription};
    use chrono::{Duration as ChronoDuration, Utc};
    use std::io::Write;
    use tempfile::TempDir;

    /// Build a RetentionManager over a fresh DB + real download dir. Returns the
    /// manager, the db (for assertions), and the tempdir (kept alive).
    async fn setup(
        max_episodes: usize,
        max_megabytes: u64,
    ) -> (RetentionManager, Arc<Database>, TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::new(&dir.path().join("test.db")).await.unwrap());
        let event_bus = Arc::new(EventBus::new());
        let download_manager = Arc::new(DownloadManager::new(
            dir.path().join("downloads"),
            3,
            db.clone(),
            event_bus.clone(),
        ));
        let audio_player = Arc::new(AudioPlayer::new(event_bus).unwrap());
        let manager = RetentionManager::new(
            db.clone(),
            download_manager,
            audio_player,
            dir.path().join("stream-cache"),
            max_episodes,
            max_megabytes,
        );
        (manager, db, dir)
    }

    /// Insert a downloaded episode with a real file of `size` bytes, played
    /// `age_hours` ago (older = evicted first).
    async fn add_download(
        db: &Database,
        dir: &TempDir,
        title: &str,
        size: usize,
        age_hours: i64,
    ) -> Episode {
        let sub = Subscription::new("Test".to_string(), format!("https://x/{title}.xml"));
        db.insert_subscription(&sub).await.unwrap();

        let path = dir.path().join(format!("{title}.mp3"));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&vec![0u8; size]).unwrap();

        let mut ep = Episode::new(
            sub.id,
            title.to_string(),
            format!("https://x/{title}.mp3"),
            format!("guid-{title}"),
            Utc::now(),
        );
        ep.download_status = DownloadStatus::Downloaded;
        ep.local_path = Some(path);
        ep.last_played_at = Some(Utc::now() - ChronoDuration::hours(age_hours));
        db.insert_episode(&ep).await.unwrap();
        ep
    }

    #[tokio::test]
    async fn evicts_oldest_over_episode_cap() {
        let (manager, db, dir) = setup(2, 0).await;
        let old = add_download(&db, &dir, "old", 100, 72).await;
        let mid = add_download(&db, &dir, "mid", 100, 24).await;
        let new = add_download(&db, &dir, "new", 100, 1).await;

        manager.enforce().await;

        // Oldest evicted; two newest kept.
        assert!(!old.local_path.unwrap().exists(), "oldest file deleted");
        assert!(mid.local_path.unwrap().exists());
        assert!(new.local_path.unwrap().exists());
        let remaining = db.get_downloaded_episodes().await.unwrap();
        assert_eq!(remaining.len(), 2);
    }

    #[tokio::test]
    async fn evicts_over_byte_cap() {
        // 1 MiB cap; three 512 KiB downloads -> keep the two newest (1 MiB).
        let (manager, db, dir) = setup(0, 1).await;
        let half_mib = 512 * 1024;
        let old = add_download(&db, &dir, "old", half_mib, 72).await;
        let _mid = add_download(&db, &dir, "mid", half_mib, 24).await;
        let _new = add_download(&db, &dir, "new", half_mib, 1).await;

        manager.enforce().await;

        assert!(
            !old.local_path.unwrap().exists(),
            "oldest evicted to fit byte cap"
        );
        assert_eq!(db.get_downloaded_episodes().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn never_evicts_currently_playing() {
        let (manager, db, dir) = setup(1, 0).await;
        let old = add_download(&db, &dir, "old", 100, 72).await;
        let new = add_download(&db, &dir, "new", 100, 1).await;

        // Mark the oldest as the one currently playing; it must survive even
        // though it is the eviction candidate under a cap of 1.
        manager
            .audio_player
            .play_from_file(old.id, old.local_path.clone().unwrap(), Duration::ZERO)
            .await
            .unwrap();

        manager.enforce().await;

        assert!(old.local_path.unwrap().exists(), "currently-playing kept");
        assert!(
            !new.local_path.unwrap().exists(),
            "the other one evicted instead"
        );
        assert_eq!(db.get_downloaded_episodes().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn reconciles_missing_file() {
        // A cap makes enforce scan the files (0/0 short-circuits).
        let (manager, db, dir) = setup(10, 0).await;
        let ep = add_download(&db, &dir, "gone", 100, 1).await;
        std::fs::remove_file(ep.local_path.as_ref().unwrap()).unwrap();

        manager.enforce().await;

        assert!(
            db.get_downloaded_episodes().await.unwrap().is_empty(),
            "a download whose file vanished is reconciled to not-downloaded"
        );
    }
}
