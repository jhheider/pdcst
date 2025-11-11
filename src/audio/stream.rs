use anyhow::{Context, Result};
use bytes::Bytes;
use futures::StreamExt;
use reqwest::Client;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone)]
pub struct StreamState {
    pub episode_id: uuid::Uuid,
    pub buffer: Arc<RwLock<Vec<u8>>>,
    pub content_length: Option<u64>,
    pub bytes_loaded: Arc<RwLock<u64>>,
    pub complete: Arc<RwLock<bool>>,
}

impl StreamState {
    pub fn new(episode_id: uuid::Uuid) -> Self {
        Self {
            episode_id,
            buffer: Arc::new(RwLock::new(Vec::new())),
            content_length: None,
            bytes_loaded: Arc::new(RwLock::new(0)),
            complete: Arc::new(RwLock::new(false)),
        }
    }

    pub async fn get_buffer(&self) -> Vec<u8> {
        self.buffer.read().await.clone()
    }

    pub async fn get_progress(&self) -> f64 {
        if let Some(total) = self.content_length {
            let loaded = *self.bytes_loaded.read().await;
            (loaded as f64 / total as f64) * 100.0
        } else {
            0.0
        }
    }

    pub async fn is_complete(&self) -> bool {
        *self.complete.read().await
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

        let mut stream = response.bytes_stream();
        let mut total_bytes = 0u64;

        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result.context("Failed to read audio stream chunk")?;

            total_bytes += chunk.len() as u64;

            // Append to buffer
            let mut buffer = state.buffer.write().await;
            buffer.extend_from_slice(&chunk);
            drop(buffer);

            // Update progress
            let mut bytes_loaded = state.bytes_loaded.write().await;
            *bytes_loaded = total_bytes;
            drop(bytes_loaded);

            tracing::debug!(
                "Streamed {} bytes ({:.1}%)",
                total_bytes,
                state.get_progress().await
            );
        }

        // Mark as complete
        let mut complete = state.complete.write().await;
        *complete = true;
        drop(complete);

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
        state.content_length = Some(data.len() as u64);

        let mut buffer = state.buffer.write().await;
        *buffer = data;
        drop(buffer);

        let mut bytes_loaded = state.bytes_loaded.write().await;
        *bytes_loaded = state.content_length.unwrap();
        drop(bytes_loaded);

        let mut complete = state.complete.write().await;
        *complete = true;
        drop(complete);

        tracing::info!("Loaded {} bytes from file", state.content_length.unwrap());
        Ok(state)
    }
}

impl Default for AudioStreamer {
    fn default() -> Self {
        Self::new()
    }
}
