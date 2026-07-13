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
                // A failure ends playback (e.g. a mid-stream download drop); clear
                // the playing indicator. The saved position is untouched, and the
                // episode is not marked played, so it resumes where it left off.
                self.state.is_playing = false;
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
                // Do not pop a modal per failure: a bulk refresh of a fixture with
                // several dead URLs would bury the user in dialogs. The error is
                // now persisted on the row (set by the refresher) and shown inline
                // with a `!` marker; just reload so it appears, plus a brief status.
                let _ = self.state.load_subscriptions().await;
                self.state
                    .set_status("A feed failed to refresh (see the ! marker)".to_string());
            }

            // Feed recovery: a title search found a different feed URL. Stash the
            // re-point as a pending action and ask before changing anything.
            FeedFixFound {
                subscription_id,
                podcast_title,
                artist,
                new_url,
            } => {
                self.state.pending_action = Some(crate::app::state::PendingAction::RepointFeed {
                    subscription_id,
                    new_url: new_url.clone(),
                });
                self.state.modal = crate::app::state::Modal::Confirm {
                    message: format!(
                        "Found '{podcast_title}' by {artist}.\nSwitch this feed to:\n{new_url}?"
                    ),
                    action: "repoint-feed".to_string(),
                };
            }
            FeedFixNotFound { .. } => {
                self.state
                    .set_status("No updated feed found for that title.".to_string());
            }
            // No confident match: drop into the Search view as a picker over the
            // candidates (choosing one re-points this feed instead of subscribing).
            FeedFixCandidates {
                subscription_id,
                results,
            } => {
                // Seed the query box with the title so a re-search is one edit away.
                let title = self
                    .state
                    .subscriptions
                    .iter()
                    .find(|s| s.id == subscription_id)
                    .map(|s| s.title.clone())
                    .unwrap_or_default();

                self.state.set_view(crate::app::state::View::Search);
                self.state.feed_fix_target = Some(subscription_id);
                self.state.search_input = title;
                self.state.search_cursor = self.state.search_input.chars().count();
                self.state.search_results = results;
                self.state.focus_search_results();
                self.state.set_status(
                    "No exact match - pick a feed to re-point, or edit the search.".to_string(),
                );
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
