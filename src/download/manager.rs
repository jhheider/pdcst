use crate::app::events::{EventBus, StateEvent};
use crate::download::DownloadProgress;
use crate::models::{DownloadStatus, Episode};
use crate::storage::Database;
use anyhow::{Context, Result};
use futures::StreamExt;
use reqwest::Client;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::{Semaphore, RwLock};
use uuid::Uuid;

pub struct DownloadManager {
    client: Client,
    download_dir: PathBuf,
    db: Arc<Database>,
    semaphore: Arc<Semaphore>,
    active_downloads: Arc<RwLock<HashMap<Uuid, Arc<DownloadProgress>>>>,
    cancel_signals: Arc<RwLock<HashMap<Uuid, tokio::sync::watch::Sender<bool>>>>,
    event_bus: Arc<EventBus>,
}

impl DownloadManager {
    pub fn new(download_dir: PathBuf, max_concurrent: usize, db: Arc<Database>, event_bus: Arc<EventBus>) -> Self {
        crate::ensure_crypto_provider();
        let client = Client::builder()
            .user_agent("podcast-tui/1.0")
            .timeout(std::time::Duration::from_secs(600)) // 10 minute timeout
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            download_dir,
            db,
            semaphore: Arc::new(Semaphore::new(max_concurrent)),
            active_downloads: Arc::new(RwLock::new(HashMap::new())),
            cancel_signals: Arc::new(RwLock::new(HashMap::new())),
            event_bus,
        }
    }

    /// Get the progress of an active download
    pub async fn get_download_progress(&self, episode_id: Uuid) -> Option<Arc<DownloadProgress>> {
        self.active_downloads.read().await.get(&episode_id).cloned()
    }

    /// Get all active downloads
    pub async fn get_active_downloads(&self) -> Vec<Arc<DownloadProgress>> {
        self.active_downloads
            .read()
            .await
            .values()
            .cloned()
            .collect()
    }

    /// Cancel an active download
    pub async fn cancel_download(&self, episode_id: Uuid) -> Result<()> {
        match self.cancel_signals.write().await.remove(&episode_id) { Some(cancel_tx) => {
            let _ = cancel_tx.send(true);
            tracing::info!("Cancellation requested for episode: {}", episode_id);

            // Emit download cancelled event
            self.event_bus.publish(StateEvent::DownloadCancelled { episode_id });

            Ok(())
        } _ => {
            anyhow::bail!("No active download for episode: {}", episode_id)
        }}
    }

    pub async fn download_episode(&self, episode: &Episode) -> Result<PathBuf> {
        let _permit = self.semaphore.acquire().await.unwrap();

        tracing::info!("Downloading episode: {}", episode.title);

        // Emit download started event
        self.event_bus.publish(StateEvent::DownloadStarted { episode_id: episode.id });

        // Get content length for progress tracking
        let total_size = self.get_content_length(&episode.url).await.ok();

        // Create progress tracker
        let progress = Arc::new(DownloadProgress::new(episode.id, total_size));

        // Create cancellation signal
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);

        // Register progress and cancel signal
        self.active_downloads
            .write()
            .await
            .insert(episode.id, progress.clone());
        self.cancel_signals.write().await.insert(episode.id, cancel_tx);

        // Update status to downloading
        self.db
            .update_episode_download_status(episode.id, DownloadStatus::Downloading, None)
            .await?;

        // Create download directory if it doesn't exist
        tokio::fs::create_dir_all(&self.download_dir)
            .await
            .context("Failed to create download directory")?;

        // Generate filename
        let filename = self.generate_filename(episode);
        let filepath = self.download_dir.join(&filename);

        // Download the file
        let result = self.download_file(&episode.url, &filepath, progress.clone(), cancel_rx).await;

        // Clean up tracking
        self.active_downloads.write().await.remove(&episode.id);
        self.cancel_signals.write().await.remove(&episode.id);

        match result {
            Ok(_) => {
                // Update status to downloaded
                self.db
                    .update_episode_download_status(
                        episode.id,
                        DownloadStatus::Downloaded,
                        Some(&filepath),
                    )
                    .await?;

                progress.set_complete().await;

                tracing::info!(
                    "Downloaded episode: {} -> {}",
                    episode.title,
                    filepath.display()
                );

                // Emit download completed event
                self.event_bus.publish(StateEvent::DownloadCompleted { episode_id: episode.id });

                Ok(filepath)
            }
            Err(e) => {
                // Check if it was cancelled
                let status = if e.to_string().contains("cancelled") {
                    DownloadStatus::NotDownloaded
                } else {
                    DownloadStatus::Failed
                };

                // Update status
                self.db
                    .update_episode_download_status(episode.id, status, None)
                    .await?;

                // Delete partial file
                let _ = tokio::fs::remove_file(&filepath).await;

                tracing::error!("Failed to download episode {}: {}", episode.title, e);

                // Emit download failed event (only if not cancelled - cancellation event already sent)
                if !e.to_string().contains("cancelled") {
                    self.event_bus.publish(StateEvent::DownloadFailed {
                        episode_id: episode.id,
                        error: e.to_string()
                    });
                }

                Err(e)
            }
        }
    }

    async fn get_content_length(&self, url: &str) -> Result<u64> {
        let response = self
            .client
            .head(url)
            .send()
            .await
            .context("Failed to send HEAD request")?;

        response
            .content_length()
            .ok_or_else(|| anyhow::anyhow!("No content-length header"))
    }

    async fn download_file(
        &self,
        url: &str,
        filepath: &Path,
        progress: Arc<DownloadProgress>,
        mut cancel_rx: tokio::sync::watch::Receiver<bool>,
    ) -> Result<()> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .with_context(|| format!("Failed to fetch: {}", url))?;

        if !response.status().is_success() {
            anyhow::bail!("HTTP error {}: {}", response.status(), url);
        }

        let mut file = File::create(filepath)
            .await
            .with_context(|| format!("Failed to create file: {}", filepath.display()))?;

        let mut stream = response.bytes_stream();
        let mut total_bytes = 0u64;
        let mut last_progress_event_time = tokio::time::Instant::now();
        let episode_id = progress.episode_id;

        while let Some(chunk_result) = stream.next().await {
            // Check for cancellation
            if *cancel_rx.borrow_and_update() {
                file.flush().await.ok();
                anyhow::bail!("Download cancelled");
            }

            let chunk = chunk_result.context("Failed to read download chunk")?;
            file.write_all(&chunk)
                .await
                .context("Failed to write to file")?;

            total_bytes += chunk.len() as u64;

            // Update progress
            progress.update(chunk.len() as u64).await;

            // Emit progress event every 1 second (throttled)
            if last_progress_event_time.elapsed() >= tokio::time::Duration::from_secs(1) {
                let percent = progress.get_progress().await as f32;
                self.event_bus.publish(StateEvent::DownloadProgress {
                    episode_id,
                    percent,
                });
                last_progress_event_time = tokio::time::Instant::now();
            }
        }

        file.flush().await.context("Failed to flush file")?;

        tracing::debug!("Downloaded {} bytes to {}", total_bytes, filepath.display());
        Ok(())
    }

    fn generate_filename(&self, episode: &Episode) -> String {
        // Sanitize the title for use as a filename
        let safe_title = episode
            .title
            .chars()
            .map(|c| match c {
                '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
                _ => c,
            })
            .collect::<String>();

        // Get file extension from URL or use mp3 as default
        let extension = episode
            .url
            .rsplit('.')
            .next()
            .and_then(|ext| {
                if ext.len() <= 4 && ext.chars().all(|c| c.is_alphanumeric()) {
                    Some(ext)
                } else {
                    None
                }
            })
            .unwrap_or("mp3");

        format!("{}.{}", safe_title, extension)
    }

    pub async fn delete_download(&self, episode: &Episode) -> Result<()> {
        if let Some(local_path) = &episode.local_path {
            if local_path.exists() {
                tokio::fs::remove_file(local_path)
                    .await
                    .with_context(|| format!("Failed to delete file: {}", local_path.display()))?;

                tracing::info!("Deleted download: {}", local_path.display());
            }

            // Update database
            self.db
                .update_episode_download_status(episode.id, DownloadStatus::NotDownloaded, None)
                .await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::events::EventBus;
    use crate::models::{Episode, Subscription};
    use chrono::Utc;
    use tempfile::TempDir;

    async fn setup_test_db() -> (Arc<Database>, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let db = Database::new(&db_path).await.unwrap();
        (Arc::new(db), temp_dir)
    }

    fn create_test_episode(title: &str, url: &str) -> Episode {
        let subscription = Subscription::new("Test Podcast".to_string(), "https://example.com/feed.xml".to_string());
        let mut episode = Episode::new(
            subscription.id,
            title.to_string(),
            url.to_string(),
            format!("guid-{}", title),
            Utc::now(),
        );
        episode.description = Some("Test description".to_string());
        episode
    }

    #[tokio::test]
    async fn test_new_download_manager() {
        let (db, temp_dir) = setup_test_db().await;
        let download_dir = temp_dir.path().join("downloads");

        let manager = DownloadManager::new(download_dir.clone(), 3, db, Arc::new(EventBus::new()));

        assert_eq!(manager.download_dir, download_dir);
        assert!(manager.active_downloads.read().await.is_empty());
        assert!(manager.cancel_signals.read().await.is_empty());
    }

    #[tokio::test]
    async fn test_get_active_downloads_empty() {
        let (db, temp_dir) = setup_test_db().await;
        let download_dir = temp_dir.path().join("downloads");
        let manager = DownloadManager::new(download_dir, 3, db, Arc::new(EventBus::new()));

        let active = manager.get_active_downloads().await;
        assert_eq!(active.len(), 0);
    }

    #[tokio::test]
    async fn test_get_download_progress_nonexistent() {
        let (db, temp_dir) = setup_test_db().await;
        let download_dir = temp_dir.path().join("downloads");
        let manager = DownloadManager::new(download_dir, 3, db, Arc::new(EventBus::new()));

        let episode_id = Uuid::new_v4();
        let progress = manager.get_download_progress(episode_id).await;
        assert!(progress.is_none());
    }

    #[tokio::test]
    async fn test_cancel_download_nonexistent() {
        let (db, temp_dir) = setup_test_db().await;
        let download_dir = temp_dir.path().join("downloads");
        let manager = DownloadManager::new(download_dir, 3, db, Arc::new(EventBus::new()));

        let episode_id = Uuid::new_v4();
        let result = manager.cancel_download(episode_id).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No active download"));
    }

    #[tokio::test]
    async fn test_generate_filename_basic() {
        let (db, temp_dir) = setup_test_db().await;
        let download_dir = temp_dir.path().join("downloads");
        let manager = DownloadManager::new(download_dir, 3, db, Arc::new(EventBus::new()));

        let episode = create_test_episode("Test Episode", "https://example.com/audio.mp3");
        let filename = manager.generate_filename(&episode);

        assert_eq!(filename, "Test Episode.mp3");
    }

    #[tokio::test]
    async fn test_generate_filename_sanitize() {
        let (db, temp_dir) = setup_test_db().await;
        let download_dir = temp_dir.path().join("downloads");
        let manager = DownloadManager::new(download_dir, 3, db, Arc::new(EventBus::new()));

        let episode = create_test_episode(
            "Test/Episode:With*Bad?Chars",
            "https://example.com/audio.mp3",
        );
        let filename = manager.generate_filename(&episode);

        assert_eq!(filename, "Test_Episode_With_Bad_Chars.mp3");
        assert!(!filename.contains('/'));
        assert!(!filename.contains(':'));
    }

    #[tokio::test]
    async fn test_generate_filename_extension() {
        let (db, temp_dir) = setup_test_db().await;
        let download_dir = temp_dir.path().join("downloads");
        let manager = DownloadManager::new(download_dir, 3, db, Arc::new(EventBus::new()));

        let episode = create_test_episode("Episode", "https://example.com/audio.m4a");
        let filename = manager.generate_filename(&episode);
        assert_eq!(filename, "Episode.m4a");

        let episode2 = create_test_episode("Episode", "https://example.com/audio.ogg");
        let filename2 = manager.generate_filename(&episode2);
        assert_eq!(filename2, "Episode.ogg");
    }

    #[tokio::test]
    async fn test_generate_filename_no_extension() {
        let (db, temp_dir) = setup_test_db().await;
        let download_dir = temp_dir.path().join("downloads");
        let manager = DownloadManager::new(download_dir, 3, db, Arc::new(EventBus::new()));

        let episode = create_test_episode("Episode", "https://example.com/audio");
        let filename = manager.generate_filename(&episode);
        assert_eq!(filename, "Episode.mp3"); // Default to mp3
    }

    #[tokio::test]
    async fn test_download_progress_tracking() {
        let episode_id = Uuid::new_v4();
        let progress = DownloadProgress::new(episode_id, Some(1000));

        assert_eq!(progress.episode_id, episode_id);
        assert_eq!(progress.total_bytes, Some(1000));
        assert!(!progress.is_complete().await);
        assert_eq!(progress.get_progress().await, 0.0);

        // Simulate progress
        progress.update(250).await;
        assert_eq!(progress.get_progress().await, 25.0);

        progress.update(250).await;
        assert_eq!(progress.get_progress().await, 50.0);

        progress.update(500).await;
        assert_eq!(progress.get_progress().await, 100.0);

        progress.set_complete().await;
        assert!(progress.is_complete().await);
    }

    #[tokio::test]
    async fn test_download_progress_no_total() {
        let episode_id = Uuid::new_v4();
        let progress = DownloadProgress::new(episode_id, None);

        assert_eq!(progress.get_progress().await, 0.0);

        progress.update(1000).await;
        // Without total, progress is 0
        assert_eq!(progress.get_progress().await, 0.0);
    }

    #[tokio::test]
    async fn test_concurrent_download_tracking() {
        let (db, temp_dir) = setup_test_db().await;
        let download_dir = temp_dir.path().join("downloads");
        let manager = Arc::new(DownloadManager::new(download_dir, 3, db, Arc::new(EventBus::new())));

        // Simulate adding downloads to tracking
        let episode1_id = Uuid::new_v4();
        let episode2_id = Uuid::new_v4();

        let progress1 = Arc::new(DownloadProgress::new(episode1_id, Some(1000)));
        let progress2 = Arc::new(DownloadProgress::new(episode2_id, Some(2000)));

        manager
            .active_downloads
            .write()
            .await
            .insert(episode1_id, progress1.clone());
        manager
            .active_downloads
            .write()
            .await
            .insert(episode2_id, progress2.clone());

        // Check we can query them
        let active = manager.get_active_downloads().await;
        assert_eq!(active.len(), 2);

        let p1 = manager.get_download_progress(episode1_id).await;
        assert!(p1.is_some());
        assert_eq!(p1.unwrap().episode_id, episode1_id);

        // Simulate completion
        manager.active_downloads.write().await.remove(&episode1_id);

        let active = manager.get_active_downloads().await;
        assert_eq!(active.len(), 1);
    }

    // Integration test with mockito would go here
    // Skipping for now as it requires setting up mock HTTP server
}
