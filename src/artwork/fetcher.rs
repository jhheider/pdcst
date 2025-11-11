use anyhow::{Context, Result};
use reqwest::Client;
use std::path::{Path, PathBuf};
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

pub struct ArtworkFetcher {
    client: Client,
    cache_dir: PathBuf,
}

impl ArtworkFetcher {
    pub fn new(cache_dir: PathBuf) -> Self {
        let client = Client::builder()
            .user_agent("podcast-tui/1.0")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self { client, cache_dir }
    }

    pub async fn fetch(&self, url: &str, id: Uuid) -> Result<PathBuf> {
        tracing::debug!("Fetching artwork from: {}", url);

        // Create cache directory if it doesn't exist
        tokio::fs::create_dir_all(&self.cache_dir)
            .await
            .context("Failed to create artwork cache directory")?;

        // Determine file extension from URL
        let extension = url
            .rsplit('.')
            .next()
            .and_then(|ext| {
                let ext = ext.split('?').next()?; // Remove query params
                if matches!(ext, "jpg" | "jpeg" | "png" | "gif" | "webp") {
                    Some(ext)
                } else {
                    None
                }
            })
            .unwrap_or("jpg");

        // Generate cache filename
        let filename = format!("{}.{}", id, extension);
        let filepath = self.cache_dir.join(&filename);

        // Check if already cached
        if filepath.exists() {
            tracing::debug!("Artwork already cached: {}", filepath.display());
            return Ok(filepath);
        }

        // Download the artwork
        let response = self
            .client
            .get(url)
            .send()
            .await
            .with_context(|| format!("Failed to fetch artwork from: {}", url))?;

        if !response.status().is_success() {
            anyhow::bail!("HTTP error {}: {}", response.status(), url);
        }

        let bytes = response
            .bytes()
            .await
            .context("Failed to read artwork bytes")?;

        // Save to cache
        let mut file = File::create(&filepath)
            .await
            .with_context(|| format!("Failed to create file: {}", filepath.display()))?;

        file.write_all(&bytes)
            .await
            .context("Failed to write artwork to file")?;

        file.flush().await.context("Failed to flush file")?;

        tracing::info!("Downloaded artwork to: {}", filepath.display());
        Ok(filepath)
    }
}
