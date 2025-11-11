//! Audio playback module
//!
//! Provides a thread-safe audio player with seeking, speed control,
//! and volume management. The player runs in a dedicated thread to
//! avoid blocking async operations and to work around the !Send nature
//! of rodio's OutputStream.
//!
//! # Lock Ordering Convention
//!
//! To prevent deadlocks, always acquire locks in this order:
//! 1. operation_lock (ensures atomic operations)
//! 2. state (audio state)
//!
//! Never acquire a lower-numbered lock while holding a higher-numbered lock.

use anyhow::{Context, Result};
use rodio::{Decoder, OutputStream, Sink, Source};
use std::io::Cursor;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Mutex as TokioMutex;
use uuid::Uuid;

/// Commands sent to the audio thread
enum AudioCommand {
    Play {
        episode_id: Uuid,
        data: Vec<u8>,
    },
    Pause,
    Resume,
    Stop,
    Seek(Duration),
    SetSpeed(f32),
    SetVolume(f32),
    Shutdown,
}

/// Shared audio state accessible from async code
struct AudioState {
    current_episode: Mutex<Option<Uuid>>,
    position_ms: AtomicU64,
    is_playing: AtomicBool,
    is_paused: AtomicBool,
    speed: Mutex<f32>,
    volume: Mutex<f32>,
    audio_buffer: Mutex<Option<Vec<u8>>>,
}

impl AudioState {
    fn new() -> Self {
        Self {
            current_episode: Mutex::new(None),
            position_ms: AtomicU64::new(0),
            is_playing: AtomicBool::new(false),
            is_paused: AtomicBool::new(false),
            speed: Mutex::new(1.0),
            volume: Mutex::new(1.0),
            audio_buffer: Mutex::new(None),
        }
    }

    fn set_position(&self, position: Duration) {
        self.position_ms
            .store(position.as_millis() as u64, Ordering::Relaxed);
    }

    fn get_position(&self) -> Duration {
        let ms = self.position_ms.load(Ordering::Relaxed);
        Duration::from_millis(ms)
    }

    fn set_playing(&self, playing: bool) {
        self.is_playing.store(playing, Ordering::Relaxed);
        if playing {
            self.is_paused.store(false, Ordering::Relaxed);
        }
    }

    fn set_paused(&self, paused: bool) {
        self.is_paused.store(paused, Ordering::Relaxed);
        if paused {
            self.is_playing.store(false, Ordering::Relaxed);
        }
    }
}

/// Thread-safe audio player
pub struct AudioPlayer {
    command_tx: mpsc::Sender<AudioCommand>,
    state: Arc<AudioState>,
    /// Ensures operations like seek are atomic
    operation_lock: Arc<TokioMutex<()>>,
}

impl AudioPlayer {
    /// Create a new audio player
    ///
    /// Spawns a dedicated audio thread that owns the OutputStream.
    pub fn new() -> Result<Self> {
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(AudioState::new());

        // Spawn dedicated audio thread
        let state_clone = state.clone();
        std::thread::spawn(move || {
            audio_thread(rx, state_clone);
        });

        Ok(Self {
            command_tx: tx,
            state,
            operation_lock: Arc::new(TokioMutex::new(())),
        })
    }

    /// Play audio from memory buffer
    pub async fn play_from_memory(&self, episode_id: Uuid, audio_data: &[u8]) -> Result<()> {
        tracing::info!(
            "Playing episode {} from memory ({} bytes)",
            episode_id,
            audio_data.len()
        );

        // Store audio buffer for seeking
        {
            let mut buffer = self.state.audio_buffer.lock().unwrap();
            *buffer = Some(audio_data.to_vec());
        }

        self.command_tx
            .send(AudioCommand::Play {
                episode_id,
                data: audio_data.to_vec(),
            })
            .context("Failed to send play command")?;

        // Update current episode
        {
            let mut current = self.state.current_episode.lock().unwrap();
            *current = Some(episode_id);
        }

        Ok(())
    }

    /// Pause playback
    pub async fn pause(&self) {
        let _ = self.command_tx.send(AudioCommand::Pause);
        self.state.set_paused(true);
        tracing::debug!("Playback paused");
    }

    /// Resume playback
    pub async fn play(&self) {
        let _ = self.command_tx.send(AudioCommand::Resume);
        self.state.set_playing(true);
        tracing::debug!("Playback resumed");
    }

    /// Stop playback
    pub async fn stop(&self) {
        let _ = self.command_tx.send(AudioCommand::Stop);
        self.state.set_playing(false);
        self.state.set_paused(false);
        self.state.set_position(Duration::ZERO);

        let mut current = self.state.current_episode.lock().unwrap();
        *current = None;

        tracing::debug!("Playback stopped");
    }

    /// Check if currently playing
    pub async fn is_playing(&self) -> bool {
        self.state.is_playing.load(Ordering::Relaxed)
    }

    /// Check if paused
    pub async fn is_paused(&self) -> bool {
        self.state.is_paused.load(Ordering::Relaxed)
    }

    /// Check if stopped or empty
    pub async fn is_stopped(&self) -> bool {
        !self.is_playing().await && !self.is_paused().await
    }

    /// Get current playback position in seconds
    pub async fn get_position(&self) -> f64 {
        self.state.get_position().as_secs_f64()
    }

    /// Seek forward by duration
    pub async fn seek_forward(&self, duration: Duration) -> Result<()> {
        let _guard = self.operation_lock.lock().await;

        let current_pos = self.state.get_position();
        let new_pos = current_pos + duration;

        self.command_tx
            .send(AudioCommand::Seek(new_pos))
            .context("Failed to send seek command")?;

        Ok(())
    }

    /// Seek backward by duration
    pub async fn seek_backward(&self, duration: Duration) -> Result<()> {
        let _guard = self.operation_lock.lock().await;

        let current_pos = self.state.get_position();
        let new_pos = current_pos.saturating_sub(duration);

        self.command_tx
            .send(AudioCommand::Seek(new_pos))
            .context("Failed to send seek command")?;

        Ok(())
    }

    /// Seek to a specific position
    pub async fn seek_to(&self, position: Duration) -> Result<()> {
        let _guard = self.operation_lock.lock().await;

        tracing::debug!("Seeking to position: {:?}", position);

        self.command_tx
            .send(AudioCommand::Seek(position))
            .context("Failed to send seek command")?;

        Ok(())
    }

    /// Set playback speed (1.0 = normal, 2.0 = double speed)
    pub async fn set_speed(&self, speed: f32) {
        let clamped_speed = speed.max(0.1).min(4.0);
        {
            let mut s = self.state.speed.lock().unwrap();
            *s = clamped_speed;
        }

        let _ = self.command_tx.send(AudioCommand::SetSpeed(clamped_speed));
        tracing::debug!("Playback speed set to {}", clamped_speed);
    }

    /// Get current playback speed
    pub async fn get_speed(&self) -> f32 {
        *self.state.speed.lock().unwrap()
    }

    /// Set volume (0.0 to 1.0)
    pub async fn set_volume(&self, volume: f32) {
        let clamped_volume = volume.clamp(0.0, 1.0);
        {
            let mut v = self.state.volume.lock().unwrap();
            *v = clamped_volume;
        }

        let _ = self
            .command_tx
            .send(AudioCommand::SetVolume(clamped_volume));
        tracing::debug!("Volume set to {}", clamped_volume);
    }

    /// Get current volume
    pub async fn get_volume(&self) -> f32 {
        *self.state.volume.lock().unwrap()
    }

    /// Get current episode ID
    pub async fn get_current_episode(&self) -> Option<Uuid> {
        *self.state.current_episode.lock().unwrap()
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        let _ = self.command_tx.send(AudioCommand::Shutdown);
    }
}

impl Default for AudioPlayer {
    fn default() -> Self {
        Self::new().expect("Failed to create default audio player")
    }
}

/// Audio thread that owns the OutputStream and Sink
///
/// This runs in a dedicated std::thread (not tokio) because OutputStream
/// and Sink are !Send and must stay on the same thread.
fn audio_thread(rx: mpsc::Receiver<AudioCommand>, state: Arc<AudioState>) {
    // Create audio output stream - this stays on this thread
    let Ok((_stream, stream_handle)) = OutputStream::try_default() else {
        tracing::error!("Failed to create audio output stream");
        return;
    };

    let mut sink: Option<Sink> = None;
    let mut playback_start_time: Option<Instant> = None;
    let mut start_position = Duration::ZERO;

    loop {
        // Use recv_timeout to periodically update position
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(cmd) => {
                let should_shutdown = matches!(cmd, AudioCommand::Shutdown);
                handle_command(
                    cmd,
                    &mut sink,
                    &stream_handle,
                    &state,
                    &mut playback_start_time,
                    &mut start_position,
                );
                if should_shutdown {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Update position
                if let Some(start_time) = playback_start_time {
                    if state.is_playing.load(Ordering::Relaxed) {
                        let elapsed = start_time.elapsed();
                        let position = start_position + elapsed;
                        state.set_position(position);
                    }
                }

                // Check if playback completed
                if let Some(ref s) = sink {
                    if s.empty() && state.is_playing.load(Ordering::Relaxed) {
                        state.set_playing(false);
                        state.set_paused(false);
                        playback_start_time = None;
                        tracing::debug!("Playback completed");
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break;
            }
        }
    }

    tracing::debug!("Audio thread exited");
}

fn handle_command(
    cmd: AudioCommand,
    sink: &mut Option<Sink>,
    stream_handle: &rodio::OutputStreamHandle,
    state: &Arc<AudioState>,
    playback_start_time: &mut Option<Instant>,
    start_position: &mut Duration,
) {
        match cmd {
            AudioCommand::Play { episode_id, data } => {
                tracing::debug!("Audio thread: Playing episode {}", episode_id);

                // Stop any existing playback
                if let Some(s) = sink.take() {
                    s.stop();
                }

                // Decode audio
                match Decoder::new(Cursor::new(data)) {
                    Ok(source) => {
                        // Create new sink
                        match Sink::try_new(&stream_handle) {
                            Ok(new_sink) => {
                                new_sink.append(source);

                                // Apply current settings
                                let speed = *state.speed.lock().unwrap();
                                let volume = *state.volume.lock().unwrap();
                                new_sink.set_speed(speed);
                                new_sink.set_volume(volume);

                                *sink = Some(new_sink);
                                *start_position = Duration::ZERO;
                                *playback_start_time = Some(Instant::now());
                                state.set_playing(true);
                                state.set_position(Duration::ZERO);
                            }
                            Err(e) => {
                                tracing::error!("Failed to create sink: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to decode audio: {}", e);
                    }
                }
            }

            AudioCommand::Pause => {
                if let Some(ref s) = sink {
                    s.pause();
                    state.set_paused(true);

                    // Update start position for accurate resume
                    if let Some(start_time) = *playback_start_time {
                        *start_position = *start_position + start_time.elapsed();
                    }
                    *playback_start_time = None;
                }
            }

            AudioCommand::Resume => {
                if let Some(ref s) = sink {
                    s.play();
                    state.set_playing(true);
                    *playback_start_time = Some(Instant::now());
                }
            }

            AudioCommand::Stop => {
                if let Some(s) = sink.take() {
                    s.stop();
                }
                state.set_playing(false);
                state.set_paused(false);
                state.set_position(Duration::ZERO);
                *playback_start_time = None;
                *start_position = Duration::ZERO;
            }

            AudioCommand::Seek(position) => {
                tracing::debug!("Audio thread: Seeking to {:?}", position);

                // Get audio buffer
                // Note: This clones the buffer (~60MB for typical episode).
                // Performance: skip_duration() is O(n) but provides ±1s accuracy,
                // which is acceptable for podcast playback.
                // Future optimization: Use symphonia for frame-accurate O(1) seeking if needed.
                let buffer_opt = state.audio_buffer.lock().unwrap().clone();

                if let Some(data) = buffer_opt {
                    // Stop current playback
                    if let Some(s) = sink.take() {
                        s.stop();
                    }

                    // Re-decode and skip to position
                    match Decoder::new(Cursor::new(data)) {
                        Ok(source) => {
                            let source = source.skip_duration(position);

                            match Sink::try_new(&stream_handle) {
                                Ok(new_sink) => {
                                    new_sink.append(source);

                                    // Apply current settings
                                    let speed = *state.speed.lock().unwrap();
                                    let volume = *state.volume.lock().unwrap();
                                    new_sink.set_speed(speed);
                                    new_sink.set_volume(volume);

                                    // Start paused if we were paused
                                    if state.is_paused.load(Ordering::Relaxed) {
                                        new_sink.pause();
                                    }

                                    *sink = Some(new_sink);
                                    *start_position = position;
                                    *playback_start_time = if state.is_playing.load(Ordering::Relaxed)
                                    {
                                        Some(Instant::now())
                                    } else {
                                        None
                                    };
                                    state.set_position(position);
                                }
                                Err(e) => {
                                    tracing::error!("Failed to create sink after seek: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to decode audio for seek: {}", e);
                        }
                    }
                } else {
                    tracing::warn!("No audio buffer available for seeking");
                }
            }

            AudioCommand::SetSpeed(speed) => {
                if let Some(ref s) = sink {
                    s.set_speed(speed);
                }
            }

            AudioCommand::SetVolume(volume) => {
                if let Some(ref s) = sink {
                    s.set_volume(volume);
                }
            }

            AudioCommand::Shutdown => {
                tracing::debug!("Audio thread shutting down");
                if let Some(s) = sink.take() {
                    s.stop();
                }
                // Shutdown is handled in the main loop
            }
        }
}

// Verify that AudioPlayer is Send + Sync
fn _assert_send_sync() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    assert_send::<AudioPlayer>();
    assert_sync::<AudioPlayer>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_audio_player_is_send_sync() {
        // This test verifies AudioPlayer can be moved across thread boundaries
        let player = Arc::new(AudioPlayer::new().unwrap());

        let p1 = player.clone();
        let p2 = player.clone();

        let h1 = tokio::spawn(async move {
            p1.pause().await;
        });

        let h2 = tokio::spawn(async move {
            let _ = p2.get_position().await;
        });

        h1.await.unwrap();
        h2.await.unwrap();

        // If this test compiles and runs, AudioPlayer is Send + Sync
    }

    #[tokio::test]
    async fn test_new_player_is_stopped() {
        let player = AudioPlayer::new().unwrap();
        assert!(!player.is_playing().await);
        assert_eq!(player.get_position().await, 0.0);
    }

    #[tokio::test]
    async fn test_volume_clamps_to_valid_range() {
        let player = AudioPlayer::new().unwrap();

        player.set_volume(-1.0).await;
        assert_eq!(player.get_volume().await, 0.0);

        player.set_volume(2.0).await;
        assert_eq!(player.get_volume().await, 1.0);

        player.set_volume(0.5).await;
        assert_eq!(player.get_volume().await, 0.5);
    }

    #[tokio::test]
    async fn test_speed_clamps_to_valid_range() {
        let player = AudioPlayer::new().unwrap();

        player.set_speed(0.05).await;
        let speed = player.get_speed().await;
        assert!(speed >= 0.1, "Speed should be clamped to 0.1, got {}", speed);

        player.set_speed(10.0).await;
        let speed = player.get_speed().await;
        assert!(speed <= 4.0, "Speed should be clamped to 4.0, got {}", speed);
    }
}
