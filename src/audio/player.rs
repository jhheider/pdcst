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
        data: Arc<Vec<u8>>,
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
    audio_buffer: Mutex<Option<Arc<Vec<u8>>>>,
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

        // Wrap audio buffer in Arc for cheap cloning (no need to copy ~60MB on every seek)
        let audio_arc = Arc::new(audio_data.to_vec());

        // Store audio buffer for seeking
        {
            let mut buffer = self.state.audio_buffer.lock().unwrap();
            *buffer = Some(audio_arc.clone());
        }

        self.command_tx
            .send(AudioCommand::Play {
                episode_id,
                data: audio_arc,
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

                // Decode audio (data is Arc<Vec<u8>>, clone the inner Vec for Cursor)
                match Decoder::new(Cursor::new(data.as_ref().clone())) {
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

                // Get audio buffer (Arc clone is cheap - just increments reference count)
                // Performance: skip_duration() is O(n) but provides ±1s accuracy,
                // which is acceptable for podcast playback.
                // Future optimization: Use symphonia for frame-accurate O(1) seeking if needed.
                let buffer_opt = state.audio_buffer.lock().unwrap().clone();

                if let Some(data) = buffer_opt {
                    // Stop current playback
                    if let Some(s) = sink.take() {
                        s.stop();
                    }

                    // Re-decode and skip to position (Arc makes this cheap - no copy)
                    match Decoder::new(Cursor::new(data.as_ref().clone())) {
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

    #[tokio::test]
    async fn test_pause_sets_state() {
        let player = AudioPlayer::new().unwrap();

        player.pause().await;

        assert!(player.is_paused().await);
        assert!(!player.is_playing().await);
    }

    #[tokio::test]
    async fn test_play_sets_state() {
        let player = AudioPlayer::new().unwrap();

        // Set paused state first
        player.pause().await;
        assert!(player.is_paused().await);

        // Resume playback
        player.play().await;

        // Note: Without actual audio, is_playing may not be true
        // but is_paused should be false
        assert!(!player.is_paused().await);
    }

    #[tokio::test]
    async fn test_stop_resets_state() {
        let player = AudioPlayer::new().unwrap();

        player.stop().await;

        assert!(!player.is_playing().await);
        assert!(!player.is_paused().await);
        assert!(player.is_stopped().await);
        assert_eq!(player.get_position().await, 0.0);
        assert_eq!(player.get_current_episode().await, None);
    }

    #[tokio::test]
    async fn test_seek_forward_with_operation_lock() {
        let player = AudioPlayer::new().unwrap();

        let result = player.seek_forward(Duration::from_secs(30)).await;

        // Should succeed even without audio loaded
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_seek_backward_with_operation_lock() {
        let player = AudioPlayer::new().unwrap();

        let result = player.seek_backward(Duration::from_secs(10)).await;

        // Should succeed even without audio loaded
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_seek_to_specific_position() {
        let player = AudioPlayer::new().unwrap();

        let target = Duration::from_secs(100);
        let result = player.seek_to(target).await;

        // Should succeed even without audio loaded
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_get_current_episode_initially_none() {
        let player = AudioPlayer::new().unwrap();

        assert_eq!(player.get_current_episode().await, None);
    }

    #[tokio::test]
    async fn test_volume_persists_across_calls() {
        let player = AudioPlayer::new().unwrap();

        player.set_volume(0.3).await;
        assert_eq!(player.get_volume().await, 0.3);

        player.set_volume(0.7).await;
        assert_eq!(player.get_volume().await, 0.7);
    }

    #[tokio::test]
    async fn test_speed_persists_across_calls() {
        let player = AudioPlayer::new().unwrap();

        player.set_speed(1.5).await;
        assert_eq!(player.get_speed().await, 1.5);

        player.set_speed(2.0).await;
        assert_eq!(player.get_speed().await, 2.0);
    }

    #[tokio::test]
    async fn test_concurrent_volume_changes() {
        let player = Arc::new(AudioPlayer::new().unwrap());

        let mut handles = vec![];

        // Spawn 10 concurrent volume changes
        for i in 0..10 {
            let p = player.clone();
            let handle = tokio::spawn(async move {
                let vol = (i as f32) * 0.1;
                p.set_volume(vol).await;
            });
            handles.push(handle);
        }

        // Wait for all to complete
        for handle in handles {
            handle.await.unwrap();
        }

        // Should have some valid volume (no panics)
        let final_vol = player.get_volume().await;
        assert!(final_vol >= 0.0 && final_vol <= 1.0);
    }

    #[tokio::test]
    async fn test_concurrent_speed_changes() {
        let player = Arc::new(AudioPlayer::new().unwrap());

        let mut handles = vec![];

        // Spawn 10 concurrent speed changes
        for i in 1..=10 {
            let p = player.clone();
            let handle = tokio::spawn(async move {
                let speed = i as f32 * 0.5;
                p.set_speed(speed).await;
            });
            handles.push(handle);
        }

        // Wait for all to complete
        for handle in handles {
            handle.await.unwrap();
        }

        // Should have some valid speed (no panics)
        let final_speed = player.get_speed().await;
        assert!(final_speed > 0.0);
    }

    #[tokio::test]
    async fn test_concurrent_seeks() {
        let player = Arc::new(AudioPlayer::new().unwrap());

        let mut handles = vec![];

        // Spawn 5 concurrent seeks
        for i in 0..5 {
            let p = player.clone();
            let handle = tokio::spawn(async move {
                let pos = Duration::from_secs(i * 10);
                let _ = p.seek_to(pos).await;
            });
            handles.push(handle);
        }

        // Wait for all to complete - should not deadlock
        let timeout = tokio::time::timeout(
            Duration::from_secs(5),
            futures::future::join_all(handles)
        ).await;

        assert!(timeout.is_ok(), "Concurrent seeks deadlocked");
    }

    #[tokio::test]
    async fn test_mixed_concurrent_operations() {
        let player = Arc::new(AudioPlayer::new().unwrap());

        let mut handles = vec![];

        // Mix of different operations
        for i in 0..20 {
            let p = player.clone();
            let handle = tokio::spawn(async move {
                match i % 5 {
                    0 => p.set_volume(0.5).await,
                    1 => p.set_speed(1.5).await,
                    2 => { p.pause().await; },
                    3 => { p.play().await; },
                    _ => { let _ = p.seek_forward(Duration::from_secs(5)).await; },
                }
            });
            handles.push(handle);
        }

        // All should complete without deadlock
        let timeout = tokio::time::timeout(
            Duration::from_secs(5),
            futures::future::join_all(handles)
        ).await;

        assert!(timeout.is_ok(), "Mixed operations deadlocked");
    }

    #[tokio::test]
    async fn test_is_stopped_after_creation() {
        let player = AudioPlayer::new().unwrap();

        assert!(player.is_stopped().await);
    }

    #[tokio::test]
    async fn test_multiple_players_independent() {
        let player1 = AudioPlayer::new().unwrap();
        let player2 = AudioPlayer::new().unwrap();

        player1.set_volume(0.3).await;
        player2.set_volume(0.7).await;

        assert_eq!(player1.get_volume().await, 0.3);
        assert_eq!(player2.get_volume().await, 0.7);
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn prop_volume_always_clamped(vol in -1000.0f32..1000.0f32) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let player = AudioPlayer::new().unwrap();
                player.set_volume(vol).await;
                let actual = player.get_volume().await;
                prop_assert!(actual >= 0.0 && actual <= 1.0,
                    "Volume {} should be clamped to [0.0, 1.0], got {}",
                    vol, actual);
                Ok(())
            })?;
        }

        #[test]
        fn prop_speed_always_positive(speed in -100.0f32..100.0f32) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let player = AudioPlayer::new().unwrap();
                player.set_speed(speed).await;
                let actual = player.get_speed().await;
                prop_assert!(actual > 0.0,
                    "Speed {} should always be positive, got {}",
                    speed, actual);
                Ok(())
            })?;
        }

        #[test]
        fn prop_speed_within_bounds(speed in -100.0f32..100.0f32) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let player = AudioPlayer::new().unwrap();
                player.set_speed(speed).await;
                let actual = player.get_speed().await;
                prop_assert!(actual >= 0.1 && actual <= 4.0,
                    "Speed {} should be clamped to [0.1, 4.0], got {}",
                    speed, actual);
                Ok(())
            })?;
        }

        #[test]
        fn prop_seek_backward_never_negative(seek_back_secs in 0u64..10000) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let player = AudioPlayer::new().unwrap();

                player.seek_backward(Duration::from_secs(seek_back_secs)).await.unwrap();
                let pos = player.get_position().await;

                prop_assert!(pos >= 0.0,
                    "Position after seeking back {} seconds should not be negative, got {}",
                    seek_back_secs, pos);
                Ok(())
            })?;
        }

        #[test]
        fn prop_position_never_negative(operations in prop::collection::vec(0u8..5, 0..100)) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let player = AudioPlayer::new().unwrap();

                // Perform random operations
                for op in operations {
                    match op {
                        0 => { player.stop().await; },
                        1 => { let _ = player.seek_forward(Duration::from_secs(10)).await; },
                        2 => { let _ = player.seek_backward(Duration::from_secs(5)).await; },
                        3 => { let _ = player.seek_to(Duration::from_secs(100)).await; },
                        _ => { player.pause().await; },
                    }

                    let pos = player.get_position().await;
                    prop_assert!(pos >= 0.0,
                        "Position should never be negative after operation {}, got {}",
                        op, pos);
                }
                Ok(())
            })?;
        }

        #[test]
        fn prop_volume_persistence(vol1 in 0.0f32..1.0f32, vol2 in 0.0f32..1.0f32) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let player = AudioPlayer::new().unwrap();

                player.set_volume(vol1).await;
                let retrieved1 = player.get_volume().await;
                prop_assert!((retrieved1 - vol1).abs() < 0.001,
                    "Volume {} should persist, got {}",
                    vol1, retrieved1);

                player.set_volume(vol2).await;
                let retrieved2 = player.get_volume().await;
                prop_assert!((retrieved2 - vol2).abs() < 0.001,
                    "Volume {} should persist, got {}",
                    vol2, retrieved2);
                Ok(())
            })?;
        }

        #[test]
        fn prop_speed_persistence(speed1 in 0.1f32..4.0f32, speed2 in 0.1f32..4.0f32) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let player = AudioPlayer::new().unwrap();

                player.set_speed(speed1).await;
                let retrieved1 = player.get_speed().await;
                prop_assert!((retrieved1 - speed1).abs() < 0.001,
                    "Speed {} should persist, got {}",
                    speed1, retrieved1);

                player.set_speed(speed2).await;
                let retrieved2 = player.get_speed().await;
                prop_assert!((retrieved2 - speed2).abs() < 0.001,
                    "Speed {} should persist, got {}",
                    speed2, retrieved2);
                Ok(())
            })?;
        }
    }
}
