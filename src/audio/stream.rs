use anyhow::{Context, Result};
use futures::StreamExt;
use reqwest::Client;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct StreamState {
    pub episode_id: uuid::Uuid,
    pub buffer: Arc<RwLock<Vec<u8>>>,
    pub content_length: Option<u64>,
    /// Lock-free progress tracking (updated on every chunk)
    pub bytes_loaded: Arc<AtomicU64>,
    /// Lock-free completion flag
    pub complete: Arc<AtomicBool>,
}

impl StreamState {
    pub fn new(episode_id: uuid::Uuid) -> Self {
        Self {
            episode_id,
            buffer: Arc::new(RwLock::new(Vec::new())),
            content_length: None,
            bytes_loaded: Arc::new(AtomicU64::new(0)),
            complete: Arc::new(AtomicBool::new(false)),
        }
    }

    pub async fn get_buffer(&self) -> Vec<u8> {
        self.buffer.read().await.clone()
    }

    /// Get progress percentage (0.0 - 100.0) - lock-free
    pub async fn get_progress(&self) -> f64 {
        if let Some(total) = self.content_length {
            let loaded = self.bytes_loaded.load(Ordering::Relaxed);
            (loaded as f64 / total as f64) * 100.0
        } else {
            0.0
        }
    }

    /// Check if streaming is complete - lock-free
    pub async fn is_complete(&self) -> bool {
        self.complete.load(Ordering::Relaxed)
    }
}

pub struct AudioStreamer {
    client: Client,
}

impl AudioStreamer {
    pub fn new() -> Self {
        let client = Client::builder()
            .user_agent("podcast-tui/1.0")
            .timeout(std::time::Duration::from_secs(300)) // 5 minute timeout for large files
            .build()
            .expect("Failed to create HTTP client");

        Self { client }
    }

    /// Stream episode audio into memory
    ///
    /// Performance optimizations:
    /// - Pre-allocates buffer based on Content-Length
    /// - Collects chunks locally without locks (15,000 → 1 lock for 60MB)
    /// - Uses atomic progress tracking (lock-free)
    pub async fn stream_episode(&self, episode_id: uuid::Uuid, url: &str) -> Result<StreamState> {
        tracing::info!("Streaming episode from: {}", url);

        let response = self
            .client
            .get(url)
            .send()
            .await
            .with_context(|| format!("Failed to fetch audio from: {}", url))?;

        if !response.status().is_success() {
            anyhow::bail!("HTTP error {}: {}", response.status(), url);
        }

        let content_length = response.content_length();
        let mut state = StreamState::new(episode_id);
        state.content_length = content_length;

        // Pre-allocate buffer with known capacity (avoids reallocations)
        let capacity = content_length.unwrap_or(50_000_000) as usize; // Default 50MB
        let mut local_buffer = Vec::with_capacity(capacity);

        let mut stream = response.bytes_stream();
        let mut total_bytes = 0u64;

        // Collect all chunks into local buffer (no locks during streaming)
        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.context("Failed to read audio stream chunk")?;

            total_bytes += chunk.len() as u64;
            local_buffer.extend_from_slice(&chunk);

            // Update progress atomically (lock-free, ~7,500x for 60MB file)
            state.bytes_loaded.store(total_bytes, Ordering::Relaxed);

            if total_bytes.is_multiple_of(1_000_000) {
                // Log every 1MB to reduce noise
                tracing::debug!(
                    "Streamed {} bytes ({:.1}%)",
                    total_bytes,
                    state.get_progress().await
                );
            }
        }

        // Store final buffer (single lock acquisition)
        {
            let mut buffer = state.buffer.write().await;
            *buffer = local_buffer;
        }

        // Mark as complete atomically (lock-free)
        state.complete.store(true, Ordering::Release);

        tracing::info!("Streaming complete: {} bytes", total_bytes);
        Ok(state)
    }

    /// Load audio from local file into memory
    pub async fn load_from_file(
        &self,
        episode_id: uuid::Uuid,
        path: &std::path::Path,
    ) -> Result<StreamState> {
        tracing::info!("Loading audio from file: {}", path.display());

        let data = tokio::fs::read(path)
            .await
            .with_context(|| format!("Failed to read audio file: {}", path.display()))?;

        let mut state = StreamState::new(episode_id);
        let file_size = data.len() as u64;
        state.content_length = Some(file_size);

        // Store buffer (single lock)
        {
            let mut buffer = state.buffer.write().await;
            *buffer = data;
        }

        // Update progress atomically (lock-free)
        state.bytes_loaded.store(file_size, Ordering::Relaxed);

        // Mark complete atomically (lock-free)
        state.complete.store(true, Ordering::Release);

        tracing::info!("Loaded {} bytes from file", file_size);
        Ok(state)
    }
}

impl Default for AudioStreamer {
    fn default() -> Self {
        Self::new()
    }
}
