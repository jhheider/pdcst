//! The App's reaction to `StateEvent`s from the event bus - the UI-state side of
//! the event-driven loop (the render happens in the run loop after this runs).

use super::App;
use crate::app::events::StateEvent;
use anyhow::Result;

impl App {
    pub(crate) async fn handle_state_event(&mut self, event: StateEvent) -> Result<()> {
        use StateEvent::*;

        match event {
            PlaybackStarted { episode_id } => {
                self.state.is_playing = true;
                // The load finished: drop the "Loading..." status.
                self.state.clear_status();
                // Keep current_episode in sync. The manual play path already set
                // it, but auto-advance plays directly, so load it if it differs.
                if self.state.current_episode.as_ref().map(|e| e.id) != Some(episode_id)
                    && let Ok(Some(ep)) = self.state.db.get_episode(episode_id).await
                {
                    self.state.playback_position = ep.playback_position_seconds as f64;
                    self.state.current_episode = Some(ep);
                }
                tracing::debug!("Playback started: {}", episode_id);
            }
            PlaybackPaused => {
                self.state.is_playing = false;
            }
            PlaybackResumed => {
                self.state.is_playing = true;
            }
            PlaybackStopped => {
                self.state.is_playing = false;
                self.state.playback_position = 0.0;
            }
            PlaybackCompleted { .. } => {
                self.state.is_playing = false;
            }
            PlaybackPosition { position_secs } => {
                self.state.playback_position = position_secs;
                // Checkpoint resume position on the 1s tick (fires only while
                // playing), so a crash or quit loses at most a second.
                self.state.save_progress().await;
            }
            PlaybackError { error } => {
                tracing::error!("Playback error: {}", error);
                self.state.show_error(format!("Playback error: {}", error));
            }
            VolumeChanged { volume } => {
                self.state.volume = volume;
            }
            SpeedChanged { speed } => {
                self.state.playback_speed = speed;
            }
            QueueUpdated => {
                // Reload regardless of view: the footer shows "Up Next: N" from
                // everywhere, so the cached queue must stay current.
                let _ = self.state.load_queue().await;
            }
            DownloadProgress {
                episode_id,
                percent,
            } => {
                // Update download progress for the episode
                tracing::debug!("Download progress for {}: {:.1}%", episode_id, percent);
            }
            DownloadCompleted { episode_id } => {
                tracing::info!("Download completed for episode {}", episode_id);
                self.state.set_status("Download completed".to_string());
            }
            DownloadFailed { episode_id, error } => {
                tracing::error!("Download failed for {}: {}", episode_id, error);
                self.state.show_error(format!("Download failed: {}", error));
            }

            // A background feed refresh finished: reload subscriptions (new
            // episode counts) and, if the refreshed feed is the one being viewed,
            // its episode list too.
            FeedRefreshCompleted {
                subscription_id,
                new_episodes,
            } => {
                let _ = self.state.load_subscriptions().await;
                if self
                    .state
                    .current_subscription
                    .as_ref()
                    .is_some_and(|s| s.id == subscription_id)
                {
                    let _ = self
                        .state
                        .load_episodes_for_subscription(subscription_id)
                        .await;
                }
                if new_episodes > 0 {
                    self.state
                        .set_status(format!("Refreshed: {new_episodes} new"));
                } else {
                    self.state.set_status("Feed refreshed".to_string());
                }
            }
            FeedRefreshFailed {
                subscription_id,
                error,
            } => {
                tracing::error!("Refresh failed for {}: {}", subscription_id, error);
                self.state.show_error(format!("Refresh failed: {}", error));
            }

            // Background search finished: show the results and move focus to them.
            SearchCompleted { results } => {
                self.state.search_results = results;
                self.state.clear_status();
                self.state.focus_search_results();
            }
            SearchFailed { error } => {
                self.state.show_error(format!("Search failed: {}", error));
            }

            _ => {
                // Other events don't need immediate UI state updates
            }
        }

        Ok(())
    }
}
