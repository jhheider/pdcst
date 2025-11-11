use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub struct ArtworkCache {
    cache_dir: PathBuf,
}

impl ArtworkCache {
    pub fn new(cache_dir: PathBuf) -> Self {
        Self { cache_dir }
    }

    pub fn get(&self, id: Uuid) -> Option<PathBuf> {
        // Try different extensions
        for ext in &["jpg", "jpeg", "png", "gif", "webp"] {
            let filepath = self.cache_dir.join(format!("{}.{}", id, ext));
            if filepath.exists() {
                return Some(filepath);
            }
        }
        None
    }

    pub async fn clear(&self) -> Result<()> {
        if self.cache_dir.exists() {
            tokio::fs::remove_dir_all(&self.cache_dir)
                .await
                .context("Failed to clear artwork cache")?;

            tokio::fs::create_dir_all(&self.cache_dir)
                .await
                .context("Failed to recreate artwork cache directory")?;

            tracing::info!("Cleared artwork cache");
        }
        Ok(())
    }

    pub async fn remove(&self, id: Uuid) -> Result<()> {
        for ext in &["jpg", "jpeg", "png", "gif", "webp"] {
            let filepath = self.cache_dir.join(format!("{}.{}", id, ext));
            if filepath.exists() {
                tokio::fs::remove_file(&filepath)
                    .await
                    .with_context(|| format!("Failed to remove file: {}", filepath.display()))?;
                tracing::debug!("Removed cached artwork: {}", filepath.display());
            }
        }
        Ok(())
    }

    pub async fn size(&self) -> Result<u64> {
        let mut total_size = 0u64;

        if !self.cache_dir.exists() {
            return Ok(0);
        }

        let mut entries = tokio::fs::read_dir(&self.cache_dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            if let Ok(metadata) = entry.metadata().await {
                total_size += metadata.len();
            }
        }

        Ok(total_size)
    }
}
