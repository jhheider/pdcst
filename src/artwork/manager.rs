use anyhow::{Context, Result};
use reqwest::Client;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;
use tokio::sync::RwLock;
use std::collections::HashMap;
use uuid::Uuid;

/// Manages downloading and caching of podcast artwork
pub struct ArtworkManager {
    client: Client,
    artwork_dir: PathBuf,
    cache: Arc<RwLock<HashMap<Uuid, PathBuf>>>,
}

impl ArtworkManager {
    pub fn new(artwork_dir: PathBuf) -> Self {
        crate::ensure_crypto_provider();
        let client = Client::builder()
            .user_agent("podcast-tui/1.0")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            artwork_dir,
            cache: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Download artwork for a subscription
    ///
    /// Downloads the artwork from the URL and saves it to the artwork directory.
    /// Returns the path to the cached artwork file.
    pub async fn download_artwork(
        &self,
        subscription_id: Uuid,
        artwork_url: &str,
    ) -> Result<PathBuf> {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(path) = cache.get(&subscription_id) {
                if path.exists() {
                    tracing::debug!("Artwork cache hit for {}", subscription_id);
                    return Ok(path.clone());
                }
            }
        }

        tracing::info!("Downloading artwork for subscription {}", subscription_id);

        // Create artwork directory if it doesn't exist
        fs::create_dir_all(&self.artwork_dir)
            .await
            .context("Failed to create artwork directory")?;

        // Determine file extension from URL
        let extension = Self::get_extension_from_url(artwork_url).unwrap_or("jpg");

        // Generate filename
        let filename = format!("{}.{}", subscription_id, extension);
        let filepath = self.artwork_dir.join(&filename);

        // Download the artwork
        let response = self
            .client
            .get(artwork_url)
            .send()
            .await
            .with_context(|| format!("Failed to fetch artwork: {}", artwork_url))?;

        if !response.status().is_success() {
            anyhow::bail!("HTTP error {}: {}", response.status(), artwork_url);
        }

        let bytes = response
            .bytes()
            .await
            .context("Failed to read artwork bytes")?;

        // Save to file
        fs::write(&filepath, &bytes)
            .await
            .with_context(|| format!("Failed to write artwork file: {}", filepath.display()))?;

        tracing::info!("Downloaded artwork to: {}", filepath.display());

        // Update cache
        self.cache.write().await.insert(subscription_id, filepath.clone());

        Ok(filepath)
    }

    /// Get the path to cached artwork for a subscription
    ///
    /// Returns None if the artwork is not cached.
    pub async fn get_cached_artwork(&self, subscription_id: Uuid) -> Option<PathBuf> {
        let cache = self.cache.read().await;
        cache.get(&subscription_id).filter(|p| p.exists()).cloned()
    }

    /// Delete cached artwork for a subscription
    pub async fn delete_artwork(&self, subscription_id: Uuid) -> Result<()> {
        let mut cache = self.cache.write().await;

        if let Some(path) = cache.remove(&subscription_id) {
            if path.exists() {
                fs::remove_file(&path)
                    .await
                    .with_context(|| format!("Failed to delete artwork: {}", path.display()))?;
                tracing::info!("Deleted artwork: {}", path.display());
            }
        }

        Ok(())
    }

    /// Load cached artwork from disk into memory
    ///
    /// Scans the artwork directory and populates the cache.
    pub async fn load_cache_from_disk(&self) -> Result<()> {
        tracing::info!("Loading artwork cache from disk");

        if !self.artwork_dir.exists() {
            return Ok(());
        }

        let mut entries = fs::read_dir(&self.artwork_dir)
            .await
            .context("Failed to read artwork directory")?;

        let mut cache = self.cache.write().await;
        let mut count = 0;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            if path.is_file() {
                // Extract subscription ID from filename
                if let Some(filename) = path.file_stem() {
                    if let Some(filename_str) = filename.to_str() {
                        if let Ok(uuid) = Uuid::parse_str(filename_str) {
                            cache.insert(uuid, path);
                            count += 1;
                        }
                    }
                }
            }
        }

        tracing::info!("Loaded {} artwork files into cache", count);
        Ok(())
    }

    fn get_extension_from_url(url: &str) -> Option<&str> {
        let path = url.split('?').next()?;
        let extension = path.rsplit('.').next()?;

        // Validate it's a known image extension
        match extension.to_lowercase().as_str() {
            "jpg" | "jpeg" | "png" | "gif" | "webp" => Some(extension),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_get_extension_from_url() {
        assert_eq!(
            ArtworkManager::get_extension_from_url("https://example.com/image.jpg"),
            Some("jpg")
        );
        assert_eq!(
            ArtworkManager::get_extension_from_url("https://example.com/image.png?size=large"),
            Some("png")
        );
        assert_eq!(
            ArtworkManager::get_extension_from_url("https://example.com/image.jpeg"),
            Some("jpeg")
        );
        assert_eq!(
            ArtworkManager::get_extension_from_url("https://example.com/image.txt"),
            None
        );
    }

    #[tokio::test]
    async fn test_new_artwork_manager() {
        let temp_dir = TempDir::new().unwrap();
        let artwork_dir = temp_dir.path().join("artwork");

        let manager = ArtworkManager::new(artwork_dir.clone());
        assert_eq!(manager.artwork_dir, artwork_dir);
        assert!(manager.cache.read().await.is_empty());
    }

    #[tokio::test]
    async fn test_get_cached_artwork_empty() {
        let temp_dir = TempDir::new().unwrap();
        let manager = ArtworkManager::new(temp_dir.path().to_path_buf());

        let id = Uuid::new_v4();
        let result = manager.get_cached_artwork(id).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_delete_artwork_nonexistent() {
        let temp_dir = TempDir::new().unwrap();
        let manager = ArtworkManager::new(temp_dir.path().to_path_buf());

        let id = Uuid::new_v4();
        let result = manager.delete_artwork(id).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_load_cache_from_disk_empty() {
        let temp_dir = TempDir::new().unwrap();
        let manager = ArtworkManager::new(temp_dir.path().join("artwork"));

        let result = manager.load_cache_from_disk().await;
        assert!(result.is_ok());
        assert!(manager.cache.read().await.is_empty());
    }

    #[tokio::test]
    async fn test_load_cache_from_disk_with_files() {
        let temp_dir = TempDir::new().unwrap();
        let artwork_dir = temp_dir.path().join("artwork");
        fs::create_dir_all(&artwork_dir).await.unwrap();

        // Create test artwork files
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();

        let file1 = artwork_dir.join(format!("{}.jpg", id1));
        let file2 = artwork_dir.join(format!("{}.png", id2));

        fs::write(&file1, b"fake image data").await.unwrap();
        fs::write(&file2, b"fake image data").await.unwrap();

        let manager = ArtworkManager::new(artwork_dir);
        manager.load_cache_from_disk().await.unwrap();

        let cache = manager.cache.read().await;
        assert_eq!(cache.len(), 2);
        assert!(cache.contains_key(&id1));
        assert!(cache.contains_key(&id2));
    }
}
