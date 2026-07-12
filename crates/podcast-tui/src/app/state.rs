use crate::app::events::{EventBus, StateEvent};
use crate::artwork::ArtworkManager;
use crate::audio::{AudioPlayer, AudioStreamer};
use crate::download::DownloadManager;
use crate::feed::{FeedRefresher, PodcastSearch, SearchResult};
use crate::models::Config;
use crate::models::{Episode, PlaybackStatus, Subscription};
use crate::queue::QueueManager;
use crate::storage::Database;
use crate::storage::db::PlaybackState;
use anyhow::Result;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long a transient status message stays on screen before it auto-clears.
const STATUS_TTL: Duration = Duration::from_secs(2);

/// A one-line status message shown at the bottom of the screen.
///
/// Transient messages carry an expiry so they clear themselves at render time
/// (replacing the old pattern of blocking the event loop with `sleep(2s)`).
/// Persistent messages (`expires_at == None`) stay until explicitly cleared -
/// used for "Loading..." while an episode fetch is in flight.
pub struct StatusMessage {
    pub text: String,
    expires_at: Option<Instant>,
}

impl StatusMessage {
    /// Whether this message's TTL has elapsed as of `now`. Persistent messages
    /// (no expiry) never expire.
    fn is_expired(&self, now: Instant) -> bool {
        self.expires_at.is_some_and(|expires_at| now >= expires_at)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum View {
    Subscriptions,
    Episodes,
    Queue,
    Search,
    Settings,
}

/// The four top-level views the number keys and Tab cycle through. Episodes is a
/// drill-down of Subscriptions (reached with Enter, left with Esc), not a tab of
/// its own, so number/Tab navigation and the help screen all share one model.
const TOP_VIEWS: [View; 4] = [
    View::Subscriptions,
    View::Queue,
    View::Search,
    View::Settings,
];

/// The next top-level view when cycling with Tab. Episodes cycles as if it were
/// Subscriptions (its parent), so Tab out of a drill-down is predictable.
fn next_top_view(view: View) -> View {
    let current = if view == View::Episodes {
        View::Subscriptions
    } else {
        view
    };
    let idx = TOP_VIEWS.iter().position(|&v| v == current).unwrap_or(0);
    TOP_VIEWS[(idx + 1) % TOP_VIEWS.len()]
}

/// The previous top-level view (Shift-Tab), with Episodes treated as its parent.
fn prev_top_view(view: View) -> View {
    let current = if view == View::Episodes {
        View::Subscriptions
    } else {
        view
    };
    let idx = TOP_VIEWS.iter().position(|&v| v == current).unwrap_or(0);
    TOP_VIEWS[(idx + TOP_VIEWS.len() - 1) % TOP_VIEWS.len()]
}

#[derive(Debug, Clone, PartialEq)]
pub enum Modal {
    None,
    Help,
    Error(String),
    Confirm { message: String, action: String },
}

pub struct AppState {
    pub config: Config,
    pub db: Arc<Database>,
    pub audio_player: Arc<AudioPlayer>,
    pub audio_streamer: Arc<AudioStreamer>,
    pub queue_manager: Arc<QueueManager>,
    pub download_manager: Arc<DownloadManager>,
    pub feed_refresher: Arc<FeedRefresher>,
    pub podcast_search: Arc<PodcastSearch>,
    pub artwork_manager: Arc<ArtworkManager>,
    pub event_bus: Arc<EventBus>,

    // UI state
    pub current_view: View,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub modal: Modal,
    pub search_input: String,
    pub search_cursor: usize,
    pub status_message: Option<StatusMessage>,
    pub show_help: bool,
    /// Set by the quit key; the run loop checks it and exits.
    pub should_quit: bool,

    // Data
    pub subscriptions: Vec<Subscription>,
    pub episodes: Vec<Episode>,
    pub current_subscription: Option<Subscription>,
    pub search_results: Vec<SearchResult>,
    pub queue_items: Vec<Episode>, // Cached queue items

    // Playback state
    pub is_playing: bool,
    pub current_episode: Option<Episode>,
    pub playback_position: f64,
    pub playback_speed: f32,
    pub volume: f32,
}

/// The shared services AppState is built from, grouped so AppState::new does not
/// take a dozen positional Arc args (and so the call site reads by name).
pub struct Services {
    pub audio_player: Arc<AudioPlayer>,
    pub audio_streamer: Arc<AudioStreamer>,
    pub queue_manager: Arc<QueueManager>,
    pub download_manager: Arc<DownloadManager>,
    pub feed_refresher: Arc<FeedRefresher>,
    pub podcast_search: Arc<PodcastSearch>,
    pub artwork_manager: Arc<ArtworkManager>,
}

impl AppState {
    pub fn new(
        config: Config,
        db: Arc<Database>,
        services: Services,
        event_bus: Arc<EventBus>,
    ) -> Self {
        let Services {
            audio_player,
            audio_streamer,
            queue_manager,
            download_manager,
            feed_refresher,
            podcast_search,
            artwork_manager,
        } = services;

        // Note: Auto-advance logic has been moved to App to use event-driven architecture
        // instead of the old completion channel. This eliminates the zombie task issue.

        // Clone audio_player to query initial state (sync before events start flowing)
        let audio_player_for_init = audio_player.clone();
        let event_bus_for_init = event_bus.clone();

        // Spawn task to initialize playback state from AudioPlayer
        // This prevents startup race condition where UI shows wrong initial values
        tokio::spawn(async move {
            // Small delay to ensure audio player is fully initialized
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;

            // Query current state and emit initial events to sync UI
            let volume = audio_player_for_init.get_volume().await;
            let speed = audio_player_for_init.get_speed().await;

            event_bus_for_init.publish(crate::app::events::StateEvent::VolumeChanged { volume });
            event_bus_for_init.publish(crate::app::events::StateEvent::SpeedChanged { speed });
        });

        Self {
            config,
            db,
            audio_player,
            audio_streamer,
            queue_manager,
            download_manager,
            feed_refresher,
            podcast_search,
            artwork_manager,
            event_bus,
            current_view: View::Subscriptions,
            selected_index: 0,
            scroll_offset: 0,
            modal: Modal::None,
            search_input: String::new(),
            search_cursor: 0,
            status_message: None,
            show_help: false,
            should_quit: false,
            subscriptions: Vec::new(),
            episodes: Vec::new(),
            current_subscription: None,
            search_results: Vec::new(),
            queue_items: Vec::new(),
            is_playing: false,
            current_episode: None,
            playback_position: 0.0,
            playback_speed: 1.0,
            volume: 1.0,
        }
    }

    // Note: update() method removed - state is now event-driven
    // State fields are updated via events published by AudioPlayer, DownloadManager, etc.

    pub async fn load_subscriptions(&mut self) -> Result<()> {
        self.subscriptions = self.db.get_all_subscriptions().await?;
        tracing::debug!("Loaded {} subscriptions", self.subscriptions.len());
        Ok(())
    }

    pub async fn load_episodes_for_subscription(
        &mut self,
        subscription_id: uuid::Uuid,
    ) -> Result<()> {
        self.episodes = self
            .db
            .get_episodes_for_subscription(subscription_id)
            .await?;
        tracing::debug!("Loaded {} episodes", self.episodes.len());
        Ok(())
    }

    pub async fn toggle_playback(&mut self) -> Result<()> {
        if self.is_playing {
            self.audio_player.pause().await;
            // Checkpoint on pause so a mid-episode pause survives a quit.
            self.save_progress().await;
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

    pub fn set_view(&mut self, view: View) {
        self.current_view = view;
        self.selected_index = 0;
    }

    /// Number of items in the current view's list (0 for non-list views).
    fn current_list_len(&self) -> usize {
        match self.current_view {
            View::Subscriptions => self.subscriptions.len(),
            View::Episodes => self.episodes.len(),
            View::Queue => self.queue_items.len(),
            View::Search => self.search_results.len(),
            View::Settings => 0,
        }
    }

    /// Highest selectable index in the current view (0 when empty).
    fn max_index(&self) -> usize {
        self.current_list_len().saturating_sub(1)
    }

    pub fn next_item(&mut self) {
        if self.selected_index < self.max_index() {
            self.selected_index += 1;
        }
    }

    pub fn previous_item(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        }
    }

    pub async fn select_item(&mut self) -> Result<()> {
        match self.current_view {
            View::Subscriptions => {
                if let Some(subscription) = self.subscriptions.get(self.selected_index) {
                    self.current_subscription = Some(subscription.clone());
                    self.load_episodes_for_subscription(subscription.id).await?;
                    self.set_view(View::Episodes);
                }
            }
            View::Episodes => {
                if let Some(episode) = self.episodes.get(self.selected_index) {
                    // Play episode
                    self.play_episode(episode.clone()).await?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Start playing an episode, resuming from its saved position if any.
    ///
    /// The audio fetch (a whole-body HTTP download or a file read) runs in a
    /// spawned task so it never blocks the event loop - the UI stays responsive
    /// while "Loading..." shows. When the fetch completes, the audio player
    /// publishes `PlaybackStarted`, which clears the loading status; a failure
    /// publishes `PlaybackError`, which surfaces as an error modal.
    async fn play_episode(&mut self, episode: Episode) -> Result<()> {
        tracing::info!("Playing episode: {}", episode.title);

        // Resume where we left off (0 for a fresh episode).
        let start = Duration::from_secs(episode.playback_position_seconds.max(0) as u64);

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

    /// Search for podcasts using the iTunes Search API
    ///
    /// Updates `self.search_results` with the results.
    pub async fn search_podcasts(&mut self, query: &str) -> Result<()> {
        tracing::info!("Searching for podcasts: {}", query);

        let results = self.podcast_search.search(query).await?;

        tracing::info!("Found {} results", results.len());
        self.search_results = results;

        Ok(())
    }

    /// Subscribe to a podcast from a search result
    ///
    /// Creates a new subscription from the search result and adds it to the database.
    pub async fn subscribe_from_search_result(&mut self, result: &SearchResult) -> Result<()> {
        tracing::info!("Subscribing to: {}", result.title);

        // Create subscription from search result
        let mut subscription = Subscription::new(result.title.clone(), result.feed_url.clone());
        subscription.author = Some(result.artist.clone());
        subscription.artwork_url = result.artwork_url.clone();
        subscription.description = result.description.clone();

        // Insert into database
        self.db.insert_subscription(&subscription).await?;

        // Emit event
        self.event_bus
            .publish(crate::app::events::StateEvent::SubscriptionAdded {
                subscription_id: subscription.id,
            });

        // Reload subscriptions
        self.load_subscriptions().await?;

        tracing::info!("Successfully subscribed to: {}", result.title);
        Ok(())
    }

    // Playback control methods

    pub async fn play_next_in_queue(&mut self) -> Result<()> {
        tracing::info!("Playing next episode in queue");

        match self.queue_manager.get_next().await? {
            Some(next_item) => {
                if let Some(episode) = self.db.get_episode(next_item.episode_id).await? {
                    self.play_episode(episode).await?;
                }
                Ok(())
            }
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
            self.audio_player.set_volume(0.0).await;
            tracing::info!("Muted");
        } else {
            self.audio_player.set_volume(0.5).await;
            self.volume = 0.5;
            tracing::info!("Unmuted");
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

    // View navigation methods

    pub fn next_view(&mut self) {
        self.set_view(next_top_view(self.current_view));
    }

    pub fn previous_view(&mut self) {
        self.set_view(prev_top_view(self.current_view));
    }

    // List navigation methods

    pub fn goto_top(&mut self) {
        self.selected_index = 0;
    }

    pub fn goto_bottom(&mut self) {
        self.selected_index = self.max_index();
    }

    pub fn page_up(&mut self) {
        self.selected_index = self.selected_index.saturating_sub(10);
    }

    pub fn page_down(&mut self) {
        self.selected_index = (self.selected_index + 10).min(self.max_index());
    }

    // Item action methods

    pub async fn add_selected_to_queue(&mut self) -> Result<()> {
        if self.current_view == View::Episodes
            && let Some(episode) = self.episodes.get(self.selected_index)
        {
            self.queue_manager.add_episode(episode.id).await?;
            tracing::info!("Added '{}' to queue", episode.title);
        }
        Ok(())
    }

    pub async fn download_selected_episode(&mut self) -> Result<()> {
        if self.current_view == View::Episodes
            && let Some(episode) = self.episodes.get(self.selected_index).cloned()
        {
            tracing::info!("Downloading episode: {}", episode.title);
            // Spawn download task to not block UI
            let download_manager = self.download_manager.clone();
            tokio::spawn(async move {
                if let Err(e) = download_manager.download_episode(&episode).await {
                    tracing::error!("Download failed: {}", e);
                }
            });
        }
        Ok(())
    }

    pub async fn delete_selected_download(&mut self) -> Result<()> {
        if self.current_view == View::Episodes
            && let Some(episode) = self.episodes.get(self.selected_index)
            && episode.is_downloaded()
        {
            self.delete_download(episode).await?;
            // Reload episodes to update UI
            if let Some(sub) = &self.current_subscription {
                self.load_episodes_for_subscription(sub.id).await?;
            }
        }
        Ok(())
    }

    pub async fn refresh_selected_subscription(&mut self) -> Result<()> {
        if self.current_view == View::Subscriptions
            && let Some(subscription) = self.subscriptions.get(self.selected_index).cloned()
        {
            tracing::info!("Refreshing subscription: {}", subscription.title);
            self.feed_refresher.refresh_one(subscription).await?;
        }
        Ok(())
    }

    pub async fn refresh_all_subscriptions(&mut self) -> Result<()> {
        tracing::info!("Refreshing all subscriptions");
        let subscriptions = self.subscriptions.clone();
        self.feed_refresher.refresh_all(subscriptions).await?;
        self.load_subscriptions().await?;
        Ok(())
    }

    pub async fn toggle_played_status(&mut self) -> Result<()> {
        if self.current_view == View::Episodes
            && let Some(episode) = self.episodes.get(self.selected_index)
        {
            let episode_id = episode.id;
            let new_status = !episode.played;
            self.db.mark_episode_played(episode_id, new_status).await?;
            tracing::info!(
                "Marked episode as {}",
                if new_status { "played" } else { "unplayed" }
            );

            // Emit event
            if new_status {
                self.event_bus
                    .publish(crate::app::events::StateEvent::EpisodeMarkedPlayed { episode_id });
            } else {
                self.event_bus
                    .publish(crate::app::events::StateEvent::EpisodeMarkedUnplayed { episode_id });
            }

            // Reload episodes to update UI
            if let Some(sub) = &self.current_subscription {
                self.load_episodes_for_subscription(sub.id).await?;
            }
        }
        Ok(())
    }

    // Search mode methods (placeholders for future UI state)

    pub fn enter_search_mode(&mut self) {
        self.set_view(View::Search);
        tracing::debug!("Entered search mode");
    }

    pub fn exit_search_mode(&mut self) {
        if self.current_view == View::Search {
            self.set_view(View::Subscriptions);
        }
        tracing::debug!("Exited search mode");
    }

    // Modal and notification methods

    pub fn show_help_modal(&mut self) {
        self.modal = Modal::Help;
    }

    pub fn show_error(&mut self, message: String) {
        self.modal = Modal::Error(message);
    }

    pub fn close_modal(&mut self) {
        self.modal = Modal::None;
    }

    /// Show a transient status message that auto-clears after [`STATUS_TTL`].
    /// Does not block the event loop; expiry happens at render time.
    pub fn set_status(&mut self, message: String) {
        self.status_message = Some(StatusMessage {
            text: message,
            expires_at: Some(Instant::now() + STATUS_TTL),
        });
    }

    /// Show a status message that stays until explicitly cleared (e.g.
    /// "Loading..." while an episode fetch is in flight, which can outlast the
    /// transient TTL).
    pub fn set_status_persistent(&mut self, message: String) {
        self.status_message = Some(StatusMessage {
            text: message,
            expires_at: None,
        });
    }

    pub fn clear_status(&mut self) {
        self.status_message = None;
    }

    /// The status text to display, or `None` if there is no live message.
    pub fn current_status(&self) -> Option<&str> {
        self.status_message.as_ref().map(|s| s.text.as_str())
    }

    /// Clear the status message if its TTL has elapsed. Returns `true` when a
    /// message was cleared, signalling the caller to redraw.
    pub fn expire_status(&mut self) -> bool {
        if let Some(status) = &self.status_message
            && status.is_expired(Instant::now())
        {
            self.status_message = None;
            return true;
        }
        false
    }

    // Search input methods

    pub fn append_search_char(&mut self, c: char) {
        self.search_input.insert(self.search_cursor, c);
        self.search_cursor += 1;
    }

    pub fn delete_search_char(&mut self) {
        if self.search_cursor > 0 && !self.search_input.is_empty() {
            self.search_cursor -= 1;
            self.search_input.remove(self.search_cursor);
        }
    }

    pub fn clear_search_input(&mut self) {
        self.search_input.clear();
        self.search_cursor = 0;
    }

    // Queue management

    pub async fn load_queue(&mut self) -> Result<()> {
        // Load queue items from database
        let queue_data = self.db.get_queue().await?;
        let mut episodes = Vec::new();

        for item in queue_data {
            if let Some(episode) = self.db.get_episode(item.episode_id).await? {
                episodes.push(episode);
            }
        }

        self.queue_items = episodes;
        Ok(())
    }

    // Playback persistence (resume)

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

    /// Download an episode
    ///
    /// Downloads the episode audio to the configured download directory.
    /// Progress can be tracked via `get_download_progress()`.
    pub async fn download_episode(&self, episode: &Episode) -> Result<()> {
        tracing::info!("Starting download for: {}", episode.title);
        self.download_manager.download_episode(episode).await?;
        Ok(())
    }

    /// Get the download progress for a specific episode
    ///
    /// Returns `None` if the episode is not currently downloading.
    pub async fn get_download_progress(
        &self,
        episode_id: uuid::Uuid,
    ) -> Option<Arc<crate::download::DownloadProgress>> {
        self.download_manager
            .get_download_progress(episode_id)
            .await
    }

    /// Get all active downloads
    ///
    /// Returns a list of all episodes currently being downloaded with their progress.
    pub async fn get_active_downloads(&self) -> Vec<Arc<crate::download::DownloadProgress>> {
        self.download_manager.get_active_downloads().await
    }

    /// Cancel an active download
    ///
    /// Stops the download and cleans up any partial files.
    pub async fn cancel_download(&self, episode_id: uuid::Uuid) -> Result<()> {
        tracing::info!("Cancelling download for episode: {}", episode_id);
        self.download_manager.cancel_download(episode_id).await?;
        Ok(())
    }

    /// Delete a downloaded episode
    ///
    /// Removes the downloaded file from disk and updates the database.
    pub async fn delete_download(&self, episode: &Episode) -> Result<()> {
        tracing::info!("Deleting download for: {}", episode.title);
        self.download_manager.delete_download(episode).await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_status_expires_after_ttl() {
        let now = Instant::now();
        let msg = StatusMessage {
            text: "Added to queue".to_string(),
            expires_at: Some(now + STATUS_TTL),
        };

        assert!(!msg.is_expired(now), "should be live immediately");
        assert!(
            !msg.is_expired(now + STATUS_TTL - Duration::from_millis(1)),
            "should still be live just before the TTL"
        );
        assert!(
            msg.is_expired(now + STATUS_TTL),
            "should be expired once the TTL elapses"
        );
    }

    #[test]
    fn persistent_status_never_expires() {
        let now = Instant::now();
        let msg = StatusMessage {
            text: "Loading...".to_string(),
            expires_at: None,
        };

        assert!(!msg.is_expired(now));
        assert!(!msg.is_expired(now + Duration::from_secs(3600)));
    }

    #[test]
    fn tab_cycles_the_four_top_level_views() {
        assert_eq!(next_top_view(View::Subscriptions), View::Queue);
        assert_eq!(next_top_view(View::Queue), View::Search);
        assert_eq!(next_top_view(View::Search), View::Settings);
        assert_eq!(next_top_view(View::Settings), View::Subscriptions);

        assert_eq!(prev_top_view(View::Subscriptions), View::Settings);
        assert_eq!(prev_top_view(View::Settings), View::Search);
        assert_eq!(prev_top_view(View::Search), View::Queue);
        assert_eq!(prev_top_view(View::Queue), View::Subscriptions);
    }

    #[test]
    fn episodes_cycles_as_its_parent_subscriptions() {
        // Episodes is a drill-down, not a tab: Tab out of it behaves like being
        // in Subscriptions, so it never dead-ends.
        assert_eq!(next_top_view(View::Episodes), View::Queue);
        assert_eq!(prev_top_view(View::Episodes), View::Settings);
    }
}
