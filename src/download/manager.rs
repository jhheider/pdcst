use crate::models::{DownloadStatus, Episode};
use crate::storage::Database;
use anyhow::{Context, Result};
use futures::StreamExt;
use reqwest::Client;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use tokio::sync::Semaphore;

pub struct DownloadManager {
    client: Client,
    download_dir: PathBuf,
    db: Arc<Database>,
    semaphore: Arc<Semaphore>,
}

impl DownloadManager {
    pub fn new(download_dir: PathBuf, max_concurrent: usize, db: Arc<Database>) -> Self {
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
        }
    }

    pub async fn download_episode(&self, episode: &Episode) -> Result<PathBuf> {
        let _permit = self.semaphore.acquire().await.unwrap();

        tracing::info!("Downloading episode: {}", episode.title);

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
        match self.download_file(&episode.url, &filepath).await {
            Ok(_) => {
                // Update status to downloaded
                self.db
                    .update_episode_download_status(
                        episode.id,
                        DownloadStatus::Downloaded,
                        Some(&filepath),
                    )
                    .await?;

                tracing::info!(
                    "Downloaded episode: {} -> {}",
                    episode.title,
                    filepath.display()
                );
                Ok(filepath)
            }
            Err(e) => {
                // Update status to failed
                self.db
                    .update_episode_download_status(episode.id, DownloadStatus::Failed, None)
                    .await?;

                tracing::error!("Failed to download episode {}: {}", episode.title, e);
                Err(e)
            }
        }
    }

    async fn download_file(&self, url: &str, filepath: &Path) -> Result<()> {
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

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.context("Failed to read download chunk")?;
            file.write_all(&chunk)
                .await
                .context("Failed to write to file")?;

            total_bytes += chunk.len() as u64;
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
