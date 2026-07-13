//! AppState: playback control (play/pause, seek, speed, volume) and the
//! resume persistence, plus the shared `load_and_play` fetch-and-play helper.

#[allow(unused_imports)]
use super::*;

/// Delay before an automatic reconnect attempt after a stream drops - long
/// enough to ride out a brief blip, short enough to feel responsive.
const STREAM_RETRY_DELAY: Duration = Duration::from_secs(2);

impl AppState {
    pub async fn toggle_playback(&mut self) -> Result<()> {
        if self.is_playing {
            self.audio_player.pause().await;
            // Checkpoint on pause so a mid-episode pause survives a quit.
            self.save_progress().await;
        } else if self.stream_interrupted {
            // A stream dropped and auto-retry gave up; pressing play re-opens the
            // episode from where it left off (the player has already run dry, so a
            // plain resume would do nothing).
            if let Some(episode) = self.current_episode.clone() {
                self.clear_stream_interruption();
                let start = Duration::from_secs_f64(self.playback_position.max(0.0));
                self.play_episode_at(episode, start).await?;
            }
        } else if self.audio_player.get_current_episode().await.is_some() {
            // Episode already loaded (paused mid-listen) - just resume.
            self.audio_player.play().await;
        } else if let Some(episode) = self.current_episode.clone() {
            // Nothing loaded yet (e.g. a restored last-session episode) - load
            // and resume from the saved position.
            self.play_episode(episode).await?;
        }
        Ok(())
    }

    /// Clear the stream-drop self-heal state (sticky notice, retry counter, and
    /// the interrupted flag). Called on a successful start or a manual resume.
    pub(crate) fn clear_stream_interruption(&mut self) {
        self.playback_notice = None;
        self.stream_retry_attempts = 0;
        self.stream_interrupted = false;
    }

    /// Re-open the current episode from where it left off, in the background,
    /// after a short delay. A further failure republishes `StreamInterrupted` so
    /// the retry counter advances; success publishes `PlaybackStarted` as usual.
    pub(crate) fn spawn_stream_retry(&self) {
        let Some(episode) = self.current_episode.clone() else {
            return;
        };
        let start = Duration::from_secs_f64(self.playback_position.max(0.0));
        let audio_player = self.audio_player.clone();
        let audio_streamer = self.audio_streamer.clone();
        let event_bus = self.event_bus.clone();
        let episode_id = episode.id;
        tokio::spawn(async move {
            tokio::time::sleep(STREAM_RETRY_DELAY).await;
            if let Err(e) = load_and_play(&audio_player, &audio_streamer, &episode, start).await {
                tracing::warn!("stream reconnect for '{}' failed: {}", episode.title, e);
                event_bus.publish(StateEvent::StreamInterrupted { episode_id });
            }
        });
    }

    /// Start playing an episode, resuming from its saved position if any.
    ///
    /// The audio fetch (a whole-body HTTP download or a file read) runs in a
    /// spawned task so it never blocks the event loop - the UI stays responsive
    /// while "Loading..." shows. When the fetch completes, the audio player
    /// publishes `PlaybackStarted`, which clears the loading status; a failure
    /// publishes `PlaybackError`, which surfaces as an error modal.
    pub(crate) async fn play_episode(&mut self, episode: Episode) -> Result<()> {
        // Resume where we left off (0 for a fresh episode).
        let start = Duration::from_secs(episode.playback_position_seconds.max(0) as u64);
        self.play_episode_at(episode, start).await
    }

    /// Start playing `episode` from an explicit `start` position. Used by
    /// `play_episode` (resume from the saved position) and by a stream-drop
    /// resume (re-open from the live position).
    pub(crate) async fn play_episode_at(
        &mut self,
        episode: Episode,
        start: Duration,
    ) -> Result<()> {
        tracing::info!("Playing episode: {}", episode.title);

        // Reflect the selection immediately; the load happens off the loop.
        self.current_episode = Some(episode.clone());
        self.playback_position = start.as_secs_f64();
        self.set_status_persistent(format!("Loading {}...", episode.title));

        let audio_player = self.audio_player.clone();
        let audio_streamer = self.audio_streamer.clone();
        let event_bus = self.event_bus.clone();

        tokio::spawn(async move {
            if let Err(e) = load_and_play(&audio_player, &audio_streamer, &episode, start).await {
                tracing::error!("Failed to play '{}': {}", episode.title, e);
                event_bus.publish(StateEvent::PlaybackError {
                    error: e.to_string(),
                });
            }
        });

        Ok(())
    }

    // Playback control methods

    /// Skip to the next queued episode. Uses the shared `QueueManager::advance`
    /// (mark the current played, drop it from the queue, take the next), so `n`
    /// and natural completion behave identically. Marking played also keeps the
    /// skipped episode from being auto-re-queued.
    pub async fn play_next_in_queue(&mut self) -> Result<()> {
        tracing::info!("Skipping to next episode in queue");

        let next = if let Some(current) = self.current_episode.clone() {
            self.queue_manager.advance(current.id, true).await?
        } else {
            // Nothing playing: just take the queue head without marking anything.
            match self.queue_manager.get_next().await? {
                Some(item) => self.db.get_episode(item.episode_id).await?,
                None => None,
            }
        };

        match next {
            Some(episode) => self.play_episode(episode).await,
            None => {
                tracing::info!("Queue is empty");
                Ok(())
            }
        }
    }

    pub async fn restart_current_episode(&mut self) -> Result<()> {
        tracing::info!("Restarting current episode");
        self.audio_player
            .seek_to(std::time::Duration::from_secs(0))
            .await?;
        Ok(())
    }

    pub async fn increase_volume(&mut self, amount: f32) -> Result<()> {
        let new_volume = (self.volume + amount).clamp(0.0, 1.0);
        self.audio_player.set_volume(new_volume).await;
        self.volume = new_volume;
        tracing::debug!("Volume increased to {}", new_volume);
        Ok(())
    }

    pub async fn decrease_volume(&mut self, amount: f32) -> Result<()> {
        let new_volume = (self.volume - amount).clamp(0.0, 1.0);
        self.audio_player.set_volume(new_volume).await;
        self.volume = new_volume;
        tracing::debug!("Volume decreased to {}", new_volume);
        Ok(())
    }

    pub async fn toggle_mute(&mut self) -> Result<()> {
        if self.volume > 0.0 {
            // Remember the level so unmute restores it, not a hardcoded default.
            self.pre_mute_volume = self.volume;
            self.audio_player.set_volume(0.0).await;
            self.volume = 0.0;
            tracing::info!("Muted");
        } else {
            let restore = if self.pre_mute_volume > 0.0 {
                self.pre_mute_volume
            } else {
                1.0
            };
            self.audio_player.set_volume(restore).await;
            self.volume = restore;
            tracing::info!("Unmuted to {}", restore);
        }
        Ok(())
    }

    pub async fn increase_speed(&mut self, amount: f32) -> Result<()> {
        let new_speed = (self.playback_speed + amount).clamp(0.5, 3.0);
        self.audio_player.set_speed(new_speed).await;
        self.playback_speed = new_speed;
        tracing::debug!("Speed increased to {}x", new_speed);
        Ok(())
    }

    pub async fn decrease_speed(&mut self, amount: f32) -> Result<()> {
        let new_speed = (self.playback_speed - amount).clamp(0.5, 3.0);
        self.audio_player.set_speed(new_speed).await;
        self.playback_speed = new_speed;
        tracing::debug!("Speed decreased to {}x", new_speed);
        Ok(())
    }

    pub async fn seek_forward(&mut self, seconds: f64) -> Result<()> {
        let duration = std::time::Duration::from_secs_f64(seconds);
        self.audio_player.seek_forward(duration).await?;
        tracing::debug!("Seeked forward {}s", seconds);
        Ok(())
    }

    pub async fn seek_backward(&mut self, seconds: f64) -> Result<()> {
        let duration = std::time::Duration::from_secs_f64(seconds);
        self.audio_player.seek_backward(duration).await?;
        tracing::debug!("Seeked backward {}s", seconds);
        Ok(())
    }

    // Playback persistence (resume)

    /// Checkpoint the resume position, but at most once per
    /// `save_position_interval_seconds`, so the 1s position tick does not churn
    /// the DB. Pause/stop/quit call `save_progress` directly (unthrottled), so
    /// this only bounds how much progress a crash between checkpoints can lose.
    pub(crate) async fn maybe_checkpoint_progress(&mut self) {
        let interval =
            std::time::Duration::from_secs(self.config.save_position_interval_seconds.max(1));
        let now = std::time::Instant::now();
        let due = self
            .last_position_save
            .is_none_or(|last| now.duration_since(last) >= interval);
        if due {
            self.save_progress().await;
            self.last_position_save = Some(now);
        }
    }

    /// Persist the current playback position so it survives a pause or quit.
    ///
    /// Writes both the per-episode position (what a later replay resumes from)
    /// and the singleton `playback_state` (which episode was last playing, plus
    /// rate/volume, for restore-on-launch). A no-op when nothing is loaded.
    pub async fn save_progress(&self) {
        let Some(episode) = &self.current_episode else {
            return;
        };
        let position = self.audio_player.get_position().await;

        if let Err(e) = self
            .db
            .update_episode_playback_position(episode.id, position as i64)
            .await
        {
            tracing::warn!("Failed to persist episode position: {}", e);
        }

        let status = if self.audio_player.is_playing().await {
            PlaybackStatus::Playing
        } else {
            PlaybackStatus::Paused
        };
        let state = PlaybackState {
            current_episode_id: Some(episode.id),
            position_seconds: position,
            playback_rate: self.audio_player.get_speed().await,
            volume: self.audio_player.get_volume().await,
            status,
        };
        if let Err(e) = self.db.update_playback_state(&state).await {
            tracing::warn!("Failed to persist playback state: {}", e);
        }
    }

    /// Restore the last session's episode (without auto-playing) so the app
    /// reopens showing where you left off; press play to resume from there.
    pub async fn restore_playback_state(&mut self) -> Result<()> {
        let saved = self.db.get_playback_state().await?;
        if let Some(episode_id) = saved.current_episode_id
            && let Some(episode) = self.db.get_episode(episode_id).await?
        {
            tracing::info!(
                "Restoring last episode '{}' at {:.0}s",
                episode.title,
                saved.position_seconds
            );
            self.current_episode = Some(episode);
            self.playback_position = saved.position_seconds;
            self.playback_speed = saved.playback_rate;
            self.volume = saved.volume;
            // Push restored settings to the player so a resume uses them.
            self.audio_player.set_speed(saved.playback_rate).await;
            self.audio_player.set_volume(saved.volume).await;
        }
        Ok(())
    }
}

/// Start an episode playing from `start`, choosing the source: a downloaded
/// episode plays straight from its on-disk file; a remote one streams to disk
/// and starts as soon as a prebuffer lands. Both decode off disk - nothing is
/// buffered wholesale in memory. Runs off the event loop (a spawned task or the
/// auto-advance task), so the prebuffer wait never freezes the UI.
pub(crate) async fn load_and_play(
    audio_player: &AudioPlayer,
    audio_streamer: &AudioStreamer,
    episode: &Episode,
    start: Duration,
) -> Result<()> {
    if episode.is_downloaded() {
        let path = episode
            .local_path
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Downloaded episode missing local_path"))?;
        tracing::debug!("Playing from downloaded file: {}", path.display());
        audio_player
            .play_from_file(episode.id, path.clone(), start)
            .await
    } else {
        tracing::debug!("Streaming to disk from URL: {}", episode.url);
        let reader = audio_streamer.open_stream(episode.id, &episode.url).await?;
        audio_player.play_stream(episode.id, reader, start).await
    }
}
