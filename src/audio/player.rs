use anyhow::{Context, Result};
use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};
use std::io::Cursor;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

pub struct AudioPlayer {
    sink: Arc<RwLock<Option<Sink>>>,
    _stream: OutputStream,
    stream_handle: OutputStreamHandle,
    current_episode: Arc<RwLock<Option<uuid::Uuid>>>,
    // Position tracking
    audio_buffer: Arc<RwLock<Option<Vec<u8>>>>,
    start_position: Arc<RwLock<Duration>>,
    playback_started_at: Arc<RwLock<Option<Instant>>>,
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
            audio_buffer: Arc::new(RwLock::new(None)),
            start_position: Arc::new(RwLock::new(Duration::ZERO)),
            playback_started_at: Arc::new(RwLock::new(None)),
        })
    }

    /// Play audio from memory buffer
    pub async fn play_from_memory(
        &self,
        episode_id: uuid::Uuid,
        audio_data: &[u8],
    ) -> Result<()> {
        tracing::info!(
            "Playing episode {} from memory ({} bytes)",
            episode_id,
            audio_data.len()
        );

        // Stop any currently playing audio
        self.stop().await;

        // Store the audio buffer for seeking
        let mut buffer = self.audio_buffer.write().await;
        *buffer = Some(audio_data.to_vec());
        drop(buffer);

        // Reset position tracking
        let mut start_pos = self.start_position.write().await;
        *start_pos = Duration::ZERO;
        drop(start_pos);

        // Start playback from the beginning
        self.play_from_position(Duration::ZERO).await?;

        // Store current episode
        let mut current = self.current_episode.write().await;
        *current = Some(episode_id);
        drop(current);

        tracing::info!("Playback started for episode {}", episode_id);
        Ok(())
    }

    /// Internal method to play from a specific position
    async fn play_from_position(&self, position: Duration) -> Result<()> {
        let buffer_guard = self.audio_buffer.read().await;
        let Some(audio_data) = buffer_guard.as_ref() else {
            anyhow::bail!("No audio buffer loaded");
        };

        // Create a cursor over the audio data
        let cursor = Cursor::new(audio_data.clone());
        drop(buffer_guard);

        // Decode the audio
        let source = Decoder::new(cursor).context("Failed to decode audio data")?;

        // Skip to the desired position
        let source = source.skip_duration(position);

        // Create a new sink
        let sink =
            Sink::try_new(&self.stream_handle).context("Failed to create audio sink")?;

        // Append the source and play
        sink.append(source);

        // Store the sink
        let mut sink_guard = self.sink.write().await;
        *sink_guard = Some(sink);
        drop(sink_guard);

        // Update position tracking
        let mut start_pos = self.start_position.write().await;
        *start_pos = position;
        drop(start_pos);

        let mut started_at = self.playback_started_at.write().await;
        *started_at = Some(Instant::now());
        drop(started_at);

        Ok(())
    }

    /// Pause playback
    pub async fn pause(&self) {
        if let Some(sink) = self.sink.read().await.as_ref() {
            sink.pause();
            // Clear playback start time when pausing
            let mut started_at = self.playback_started_at.write().await;
            *started_at = None;
            tracing::debug!("Playback paused");
        }
    }

    /// Resume playback
    pub async fn play(&self) {
        if let Some(sink) = self.sink.read().await.as_ref() {
            sink.play();
            // Update playback start time when resuming
            let mut started_at = self.playback_started_at.write().await;
            *started_at = Some(Instant::now());
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

        let mut started_at = self.playback_started_at.write().await;
        *started_at = None;
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

    /// Get current playback position in seconds
    pub async fn get_position(&self) -> f64 {
        let start_pos = self.start_position.read().await;
        let started_at = self.playback_started_at.read().await;

        let position = if let Some(instant) = *started_at {
            // Currently playing - calculate elapsed time
            let elapsed = instant.elapsed();
            *start_pos + elapsed
        } else {
            // Paused or stopped - return start position
            *start_pos
        };

        position.as_secs_f64()
    }

    /// Seek forward by duration
    pub async fn seek_forward(&self, duration: Duration) -> Result<()> {
        let current_pos = Duration::from_secs_f64(self.get_position().await);
        let new_pos = current_pos + duration;

        self.seek_to(new_pos).await
    }

    /// Seek backward by duration
    pub async fn seek_backward(&self, duration: Duration) -> Result<()> {
        let current_pos = Duration::from_secs_f64(self.get_position().await);
        let new_pos = current_pos.saturating_sub(duration);

        self.seek_to(new_pos).await
    }

    /// Seek to a specific position
    pub async fn seek_to(&self, position: Duration) -> Result<()> {
        tracing::debug!("Seeking to position: {:?}", position);

        let was_playing = self.is_playing().await;

        // Stop current playback
        self.stop_playback_only().await;

        // Start playback from new position
        self.play_from_position(position).await?;

        // If we were paused, pause again
        if !was_playing {
            self.pause().await;
        }

        Ok(())
    }

    /// Stop playback but keep the audio buffer
    async fn stop_playback_only(&self) {
        let mut sink_guard = self.sink.write().await;
        if let Some(sink) = sink_guard.take() {
            sink.stop();
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
}

impl Default for AudioPlayer {
    fn default() -> Self {
        Self::new().expect("Failed to create default audio player")
    }
}
