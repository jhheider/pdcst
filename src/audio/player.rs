use crate::audio::StreamState;
use anyhow::{Context, Result};
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

pub struct AudioPlayer {
    sink: Arc<RwLock<Option<Sink>>>,
    _stream: OutputStream,
    stream_handle: OutputStreamHandle,
    current_episode: Arc<RwLock<Option<uuid::Uuid>>>,
}

impl AudioPlayer {
    pub fn new() -> Result<Self> {
        let (stream, stream_handle) =
            OutputStream::try_default().context("Failed to create audio output stream")?;

        Ok(Self {
            sink: Arc::new(RwLock::new(None)),
            _stream: stream,
            stream_handle,
            current_episode: Arc::new(RwLock::new(None)),
        })
    }

    /// Play audio from memory buffer
    pub async fn play_from_memory(&self, episode_id: uuid::Uuid, audio_data: &[u8]) -> Result<()> {
        tracing::info!(
            "Playing episode {} from memory ({} bytes)",
            episode_id,
            audio_data.len()
        );

        // Stop any currently playing audio
        self.stop().await;

        // Create a cursor over the audio data
        let cursor = Cursor::new(audio_data.to_vec());

        // Decode the audio
        let source = Decoder::new(cursor).context("Failed to decode audio data")?;

        // Create a new sink
        let sink = Sink::try_new(&self.stream_handle).context("Failed to create audio sink")?;

        // Append the source and play
        sink.append(source);

        // Store the sink
        let mut sink_guard = self.sink.write().await;
        *sink_guard = Some(sink);
        drop(sink_guard);

        // Store current episode
        let mut current = self.current_episode.write().await;
        *current = Some(episode_id);
        drop(current);

        tracing::info!("Playback started for episode {}", episode_id);
        Ok(())
    }

    /// Pause playback
    pub async fn pause(&self) {
        if let Some(sink) = self.sink.read().await.as_ref() {
            sink.pause();
            tracing::debug!("Playback paused");
        }
    }

    /// Resume playback
    pub async fn play(&self) {
        if let Some(sink) = self.sink.read().await.as_ref() {
            sink.play();
            tracing::debug!("Playback resumed");
        }
    }

    /// Stop playback
    pub async fn stop(&self) {
        let mut sink_guard = self.sink.write().await;
        if let Some(sink) = sink_guard.take() {
            sink.stop();
            tracing::debug!("Playback stopped");
        }

        let mut current = self.current_episode.write().await;
        *current = None;
    }

    /// Check if currently playing
    pub async fn is_playing(&self) -> bool {
        if let Some(sink) = self.sink.read().await.as_ref() {
            !sink.is_paused() && !sink.empty()
        } else {
            false
        }
    }

    /// Check if paused
    pub async fn is_paused(&self) -> bool {
        if let Some(sink) = self.sink.read().await.as_ref() {
            sink.is_paused() && !sink.empty()
        } else {
            false
        }
    }

    /// Check if stopped or empty
    pub async fn is_stopped(&self) -> bool {
        if let Some(sink) = self.sink.read().await.as_ref() {
            sink.empty()
        } else {
            true
        }
    }

    /// Set playback speed (1.0 = normal, 2.0 = double speed)
    pub async fn set_speed(&self, speed: f32) {
        if let Some(sink) = self.sink.read().await.as_ref() {
            sink.set_speed(speed);
            tracing::debug!("Playback speed set to {}", speed);
        }
    }

    /// Get current playback speed
    pub async fn get_speed(&self) -> f32 {
        if let Some(sink) = self.sink.read().await.as_ref() {
            sink.speed()
        } else {
            1.0
        }
    }

    /// Set volume (0.0 to 1.0)
    pub async fn set_volume(&self, volume: f32) {
        let clamped_volume = volume.clamp(0.0, 1.0);
        if let Some(sink) = self.sink.read().await.as_ref() {
            sink.set_volume(clamped_volume);
            tracing::debug!("Volume set to {}", clamped_volume);
        }
    }

    /// Get current volume
    pub async fn get_volume(&self) -> f32 {
        if let Some(sink) = self.sink.read().await.as_ref() {
            sink.volume()
        } else {
            1.0
        }
    }

    /// Get current episode ID
    pub async fn get_current_episode(&self) -> Option<uuid::Uuid> {
        *self.current_episode.read().await
    }

    /// Seek forward by duration (note: rodio doesn't support seeking directly)
    /// This is a limitation - we'd need to track position separately
    pub async fn seek_forward(&self, _duration: Duration) -> Result<()> {
        // Rodio's Sink doesn't support seeking
        // We would need to implement position tracking and restart playback at new position
        tracing::warn!("Seeking not yet implemented - rodio limitation");
        Ok(())
    }

    /// Seek backward by duration
    pub async fn seek_backward(&self, _duration: Duration) -> Result<()> {
        // Same limitation as seek_forward
        tracing::warn!("Seeking not yet implemented - rodio limitation");
        Ok(())
    }
}

impl Default for AudioPlayer {
    fn default() -> Self {
        Self::new().expect("Failed to create default audio player")
    }
}
