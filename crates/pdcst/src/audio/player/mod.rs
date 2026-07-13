//! Audio playback.
//!
//! A thread-safe [`AudioPlayer`] with seeking, speed, and volume. The engine
//! (rodio) runs on a dedicated std thread because its device sink is `!Send` and
//! must stay on one thread; async code talks to it over an `mpsc` command
//! channel and reads state through atomics. Playback state changes are published
//! on the [`EventBus`] so the UI updates event-driven, never by polling the
//! player.
//!
//! # Lock ordering
//!
//! Acquire `operation_lock` before any `state` lock; never the reverse.

use super::stream::{GrowingFile, StreamFailure};
use super::wsola_source::WsolaSource;
use crate::app::events::{EventBus, StateEvent};
use anyhow::{Context, Result};
use rodio::stream::DeviceSinkBuilder;
use rodio::{Decoder, MixerDeviceSink, Player};
use std::fs::File;
use std::io::{Read, Seek};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};
use tokio::sync::Mutex as TokioMutex;
use uuid::Uuid;

/// Where an episode's audio comes from. Both variants decode straight off disk
/// (a `Read + Seek`), so nothing is buffered wholesale in memory.
enum PlaySource {
    /// A fully-downloaded episode, played from its on-disk file.
    File(PathBuf),
    /// A remote episode streaming to disk, read as it grows.
    Stream(GrowingFile),
}

/// Commands sent to the audio thread.
enum AudioCommand {
    Play {
        episode_id: Uuid,
        source: PlaySource,
        /// Start position (for resume); `Duration::ZERO` to play from the top.
        start: Duration,
    },
    Pause,
    Resume,
    Stop,
    Seek(Duration),
    SetVolume(f32),
    Shutdown,
}

/// Shared audio state, read from async code without touching the audio thread.
struct AudioState {
    current_episode: Mutex<Option<Uuid>>,
    /// Source-time position (ms), shared with the WsolaSource, which writes it.
    position_ms: Arc<AtomicU64>,
    is_playing: AtomicBool,
    is_paused: AtomicBool,
    /// Tempo (`f32` bits), shared with the WsolaSource, which reads it live.
    /// Pitch-corrected speed is wsola's job, not the sink's.
    tempo: Arc<AtomicU32>,
    volume: Mutex<f32>,
}

impl AudioState {
    fn new() -> Self {
        Self {
            current_episode: Mutex::new(None),
            position_ms: Arc::new(AtomicU64::new(0)),
            is_playing: AtomicBool::new(false),
            is_paused: AtomicBool::new(false),
            tempo: Arc::new(AtomicU32::new(1.0f32.to_bits())),
            volume: Mutex::new(1.0),
        }
    }

    fn tempo(&self) -> f32 {
        f32::from_bits(self.tempo.load(Ordering::Relaxed))
    }

    fn set_tempo(&self, tempo: f32) {
        self.tempo.store(tempo.to_bits(), Ordering::Relaxed);
    }

    fn set_position(&self, position: Duration) {
        self.position_ms
            .store(position.as_millis() as u64, Ordering::Relaxed);
    }

    fn get_position(&self) -> Duration {
        Duration::from_millis(self.position_ms.load(Ordering::Relaxed))
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

/// Thread-safe audio player handle.
pub struct AudioPlayer {
    command_tx: mpsc::Sender<AudioCommand>,
    state: Arc<AudioState>,
    /// Serializes multi-step operations (e.g. seek) against each other.
    operation_lock: Arc<TokioMutex<()>>,
    event_bus: Arc<EventBus>,
}

impl AudioPlayer {
    /// Create a player, spawning the dedicated audio thread that owns the output
    /// device and publishes state changes to `event_bus`.
    pub fn new(event_bus: Arc<EventBus>) -> Result<Self> {
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(AudioState::new());

        let state_clone = state.clone();
        let event_bus_clone = event_bus.clone();
        std::thread::spawn(move || {
            audio_thread(rx, state_clone, event_bus_clone);
        });

        Ok(Self {
            command_tx: tx,
            state,
            operation_lock: Arc::new(TokioMutex::new(())),
            event_bus,
        })
    }

    /// Play a fully-downloaded episode from its on-disk `path`, starting at
    /// `start` ([`Duration::ZERO`] plays from the top; a non-zero value resumes).
    pub async fn play_from_file(
        &self,
        episode_id: Uuid,
        path: PathBuf,
        start: Duration,
    ) -> Result<()> {
        tracing::info!("Playing episode {episode_id} from file {path:?} at {start:?}");
        self.start_play(episode_id, PlaySource::File(path), start)
    }

    /// Play a remote episode as it streams to disk, starting at `start`. The
    /// `reader` yields bytes as the download progresses (see [`GrowingFile`]).
    pub async fn play_stream(
        &self,
        episode_id: Uuid,
        reader: GrowingFile,
        start: Duration,
    ) -> Result<()> {
        tracing::info!("Playing episode {episode_id} from stream at {start:?}");
        self.start_play(episode_id, PlaySource::Stream(reader), start)
    }

    /// Common tail for the play entry points: dispatch to the audio thread and
    /// announce the start.
    fn start_play(&self, episode_id: Uuid, source: PlaySource, start: Duration) -> Result<()> {
        self.command_tx
            .send(AudioCommand::Play {
                episode_id,
                source,
                start,
            })
            .context("audio thread is gone")?;

        *self.state.current_episode.lock().unwrap() = Some(episode_id);
        self.event_bus
            .publish(StateEvent::PlaybackStarted { episode_id });
        Ok(())
    }

    /// Pause playback.
    pub async fn pause(&self) {
        let _ = self.command_tx.send(AudioCommand::Pause);
        self.state.set_paused(true);
        self.event_bus.publish(StateEvent::PlaybackPaused);
    }

    /// Resume playback.
    pub async fn play(&self) {
        let _ = self.command_tx.send(AudioCommand::Resume);
        self.state.set_playing(true);
        self.event_bus.publish(StateEvent::PlaybackResumed);
    }

    /// Stop playback and clear the current episode.
    pub async fn stop(&self) {
        let _ = self.command_tx.send(AudioCommand::Stop);
        self.state.set_playing(false);
        self.state.set_paused(false);
        self.state.set_position(Duration::ZERO);
        *self.state.current_episode.lock().unwrap() = None;
        self.event_bus.publish(StateEvent::PlaybackStopped);
    }

    /// Whether audio is currently playing.
    pub async fn is_playing(&self) -> bool {
        self.state.is_playing.load(Ordering::Relaxed)
    }

    /// Whether playback is paused.
    pub async fn is_paused(&self) -> bool {
        self.state.is_paused.load(Ordering::Relaxed)
    }

    /// Whether playback is neither playing nor paused.
    pub async fn is_stopped(&self) -> bool {
        !self.is_playing().await && !self.is_paused().await
    }

    /// Current playback position in seconds.
    pub async fn get_position(&self) -> f64 {
        self.state.get_position().as_secs_f64()
    }

    /// Seek forward by `duration`.
    pub async fn seek_forward(&self, duration: Duration) -> Result<()> {
        let _guard = self.operation_lock.lock().await;
        let new_pos = self.state.get_position() + duration;
        self.command_tx
            .send(AudioCommand::Seek(new_pos))
            .context("audio thread is gone")
    }

    /// Seek backward by `duration`.
    pub async fn seek_backward(&self, duration: Duration) -> Result<()> {
        let _guard = self.operation_lock.lock().await;
        let new_pos = self.state.get_position().saturating_sub(duration);
        self.command_tx
            .send(AudioCommand::Seek(new_pos))
            .context("audio thread is gone")
    }

    /// Seek to an absolute `position`.
    pub async fn seek_to(&self, position: Duration) -> Result<()> {
        let _guard = self.operation_lock.lock().await;
        self.command_tx
            .send(AudioCommand::Seek(position))
            .context("audio thread is gone")
    }

    /// Set playback speed (`1.0` normal). Clamped to `[0.1, 4.0]`.
    pub async fn set_speed(&self, speed: f32) {
        let speed = speed.clamp(0.1, 4.0);
        // The WsolaSource reads this live from the audio thread; no command.
        self.state.set_tempo(speed);
        self.event_bus.publish(StateEvent::SpeedChanged { speed });
    }

    /// Current playback speed.
    pub async fn get_speed(&self) -> f32 {
        self.state.tempo()
    }

    /// Set volume, clamped to `[0.0, 1.0]`.
    pub async fn set_volume(&self, volume: f32) {
        let volume = volume.clamp(0.0, 1.0);
        *self.state.volume.lock().unwrap() = volume;
        let _ = self.command_tx.send(AudioCommand::SetVolume(volume));
        self.event_bus.publish(StateEvent::VolumeChanged { volume });
    }

    /// Current volume.
    pub async fn get_volume(&self) -> f32 {
        *self.state.volume.lock().unwrap()
    }

    /// The episode currently loaded, if any.
    pub async fn get_current_episode(&self) -> Option<Uuid> {
        *self.state.current_episode.lock().unwrap()
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        let _ = self.command_tx.send(AudioCommand::Shutdown);
    }
}

/// The audio thread: owns the `!Send` output device and the current [`Player`],
/// services commands, and drives position/completion events.
fn audio_thread(
    rx: mpsc::Receiver<AudioCommand>,
    state: Arc<AudioState>,
    event_bus: Arc<EventBus>,
) {
    // No output device (a headless box, CI, or an audio-less machine) must not
    // kill the thread: stay alive in silent mode so the handle and command
    // channel keep working and the app never deadlocks - it just makes no sound.
    let device = match DeviceSinkBuilder::open_default_sink() {
        Ok(mut d) => {
            // The sink is dropped exactly once, on every clean quit, when this
            // thread returns after a `Shutdown`. rodio warns on that drop
            // ("Dropping DeviceSink...") straight to stderr - which lands as
            // stray text in the shell after our alt-screen is already torn down.
            // Here the drop is always the intended shutdown, so the warning is a
            // false positive every time; silence it.
            d.log_on_drop(false);
            Some(d)
        }
        Err(e) => {
            tracing::error!("no audio output device: {e}; running silent");
            None
        }
    };

    let mut player: Option<Player> = None;
    let mut current_episode: Option<Uuid> = None;
    // Kept alongside the player so a source running dry can be told apart from a
    // mid-stream download failure (which must not count as "finished"). `None`
    // for a fully-downloaded file, which cannot fail this way.
    let mut current_failure: Option<StreamFailure> = None;
    let mut last_position_event = Instant::now();

    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(AudioCommand::Shutdown) => break,
            Ok(cmd) => {
                if let AudioCommand::Play { episode_id, .. } = &cmd {
                    current_episode = Some(*episode_id);
                }
                handle_command(
                    cmd,
                    &mut player,
                    &mut current_failure,
                    device.as_ref(),
                    &state,
                    &event_bus,
                );
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Some(p) = &player {
                    // Position comes straight from the player (accounts for speed
                    // and seeks); no manual clock to drift.
                    if state.is_playing.load(Ordering::Relaxed) && !p.is_paused() {
                        // The WsolaSource keeps this current in source time.
                        let position = state.get_position();
                        if last_position_event.elapsed() >= Duration::from_secs(1) {
                            event_bus.publish(StateEvent::PlaybackPosition {
                                position_secs: position.as_secs_f64(),
                            });
                            last_position_event = Instant::now();
                        }
                    }
                    if p.empty() && state.is_playing.load(Ordering::Relaxed) {
                        state.set_playing(false);
                        state.set_paused(false);
                        // A mid-stream download failure makes the source run dry
                        // early; that is an error (keep position, do not mark
                        // played), not a completion.
                        let failed = current_failure.take().is_some_and(|f| f.failed());
                        if let Some(episode_id) = current_episode.take() {
                            event_bus.publish(run_dry_event(episode_id, failed));
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    tracing::debug!("audio thread exited");
}

fn handle_command(
    cmd: AudioCommand,
    player: &mut Option<Player>,
    current_failure: &mut Option<StreamFailure>,
    device: Option<&MixerDeviceSink>,
    state: &Arc<AudioState>,
    event_bus: &Arc<EventBus>,
) {
    match cmd {
        AudioCommand::Play {
            episode_id,
            source,
            start,
        } => {
            tracing::debug!("audio thread: play episode {episode_id} at {start:?}");
            if let Some(p) = player.take() {
                p.stop();
            }
            // A new play supersedes any prior stream's failure handle; set it from
            // this source (a stream can fail mid-download; a local file cannot).
            *current_failure = None;
            if let Some(device) = device {
                // Each source is a distinct `Read + Seek` concrete type, so open
                // it and hand it to the generic `start_playback`.
                match source {
                    PlaySource::File(path) => match File::open(&path) {
                        Ok(file) => start_playback(file, device, player, state, event_bus, start),
                        Err(e) => {
                            let error = format!("failed to open {}: {e}", path.display());
                            tracing::error!("{error}");
                            event_bus.publish(StateEvent::PlaybackError { error });
                            return;
                        }
                    },
                    PlaySource::Stream(reader) => {
                        *current_failure = Some(reader.failure());
                        start_playback(reader, device, player, state, event_bus, start)
                    }
                }
            }
            // State stays consistent even in silent mode (no device).
            state.set_playing(true);
            state.set_position(start);
        }

        AudioCommand::Pause => {
            if let Some(p) = player {
                p.pause();
                state.set_paused(true);
            }
        }

        AudioCommand::Resume => {
            if let Some(p) = player {
                p.play();
                state.set_playing(true);
            }
        }

        AudioCommand::Stop => {
            if let Some(p) = player.take() {
                p.stop();
            }
            *current_failure = None;
            state.set_playing(false);
            state.set_paused(false);
            state.set_position(Duration::ZERO);
        }

        AudioCommand::Seek(position) => {
            // Real seek: the decoder seeks within its buffer, O(1)-ish, no
            // re-decode from zero. In silent mode we still track the position.
            match player {
                Some(p) => match p.try_seek(position) {
                    Ok(()) => state.set_position(position),
                    Err(e) => tracing::error!("seek to {position:?} failed: {e}"),
                },
                None => state.set_position(position),
            }
        }

        AudioCommand::SetVolume(volume) => {
            if let Some(p) = player {
                p.set_volume(volume);
            }
        }

        AudioCommand::Shutdown => {
            if let Some(p) = player.take() {
                p.stop();
            }
            *current_failure = None;
        }
    }
}

/// The terminal event when a source runs dry: a mid-stream download failure is a
/// `PlaybackError` (the app keeps the saved position and does not mark the
/// episode played), anything else is a natural `PlaybackCompleted`.
fn run_dry_event(episode_id: Uuid, failed: bool) -> StateEvent {
    if failed {
        tracing::warn!("episode {episode_id} stream failed mid-playback; not marking played");
        StateEvent::PlaybackError {
            error: "episode download failed mid-stream; your position was kept".to_string(),
        }
    } else {
        StateEvent::PlaybackCompleted { episode_id }
    }
}

/// Decode `reader`, wrap it in the pitch-corrected time-stretcher, and start it
/// on a fresh [`Player`] connected to `device`, seeking to `start` if resuming.
///
/// Generic over the reader so a downloaded [`File`] and a streaming
/// [`GrowingFile`] share one code path.
fn start_playback<R: Read + Seek + Send + Sync + 'static>(
    reader: R,
    device: &MixerDeviceSink,
    player: &mut Option<Player>,
    state: &Arc<AudioState>,
    event_bus: &Arc<EventBus>,
    start: Duration,
) {
    let decoder = match Decoder::new(reader) {
        Ok(s) => s,
        Err(e) => {
            let error = format!("failed to decode audio: {e}");
            tracing::error!("{error}");
            event_bus.publish(StateEvent::PlaybackError { error });
            return;
        }
    };
    // Tempo and position are shared with AudioState, so speed changes apply live
    // and position is reported in source time.
    let source = WsolaSource::new(decoder, state.tempo.clone(), state.position_ms.clone());
    let p = Player::connect_new(device.mixer());
    p.append(source);
    p.set_volume(*state.volume.lock().unwrap());
    if start > Duration::ZERO
        && let Err(e) = p.try_seek(start)
    {
        tracing::warn!("resume seek to {start:?} failed: {e}");
    }
    *player = Some(p);
}

// AudioPlayer must be shareable across async tasks.
fn _assert_send_sync() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    assert_send::<AudioPlayer>();
    assert_sync::<AudioPlayer>();
}

#[cfg(test)]
mod tests;
