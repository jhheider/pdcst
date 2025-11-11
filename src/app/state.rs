use crate::artwork::ArtworkManager;
use crate::audio::{AudioPlayer, AudioStreamer};
use crate::download::DownloadManager;
use crate::feed::{FeedRefresher, PodcastSearch, SearchResult};
use crate::models::Config;
use crate::models::{Episode, Subscription};
use crate::queue::QueueManager;
use crate::storage::Database;
use anyhow::Result;
use std::path::Path;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum View {
    Subscriptions,
    Episodes,
    Queue,
    Search,
    Settings,
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

    // UI state
    pub current_view: View,
    pub selected_index: usize,
    pub scroll_offset: usize,
    pub modal: Modal,
    pub search_input: String,
    pub search_cursor: usize,
    pub status_message: Option<String>,
    pub show_help: bool,

    // Data
    pub subscriptions: Vec<Subscription>,
    pub episodes: Vec<Episode>,
    pub current_subscription: Option<Subscription>,
    pub search_results: Vec<SearchResult>,
    pub queue_items: Vec<Episode>,  // Cached queue items

    // Playback state
    pub is_playing: bool,
    pub current_episode: Option<Episode>,
    pub playback_position: f64,
    pub playback_speed: f32,
    pub volume: f32,
}

impl AppState {
    pub fn new(
        config: Config,
        db: Arc<Database>,
        audio_player: Arc<AudioPlayer>,
        audio_streamer: Arc<AudioStreamer>,
        queue_manager: Arc<QueueManager>,
        download_manager: Arc<DownloadManager>,
        feed_refresher: Arc<FeedRefresher>,
        podcast_search: Arc<PodcastSearch>,
        artwork_manager: Arc<ArtworkManager>,
    ) -> Self {
        // Set up queue auto-advance
        if let Some(mut completion_rx) = audio_player.take_completion_rx() {
            let queue_manager_clone = queue_manager.clone();
            let db_clone = db.clone();
            let audio_streamer_clone = audio_streamer.clone();
            let audio_player_clone = audio_player.clone();

            tokio::spawn(async move {
                while let Some(completed_episode_id) = completion_rx.recv().await {
                    tracing::info!("Episode {} completed, checking queue for next episode", completed_episode_id);

                    // Mark episode as played
                    if let Err(e) = db_clone.mark_episode_played(completed_episode_id, true).await {
                        tracing::error!("Failed to mark episode as played: {}", e);
                    }

                    // Remove from queue
                    if let Err(e) = queue_manager_clone.remove_episode(completed_episode_id).await {
                        tracing::error!("Failed to remove episode from queue: {}", e);
                    }

                    // Play next episode in queue
                    match queue_manager_clone.get_next().await {
                        Ok(Some(next_item)) => {
                            tracing::info!("Auto-advancing to next episode in queue");

                            // Load episode details
                            match db_clone.get_episode(next_item.episode_id).await {
                                Ok(Some(next_episode)) => {
                                    // Load audio data
                                    let audio_data_result = if next_episode.is_downloaded() {
                                        if let Some(path) = &next_episode.local_path {
                                            match audio_streamer_clone.load_from_file(next_episode.id, path.as_ref()).await {
                                                Ok(state) => state.get_buffer().await,
                                                Err(e) => {
                                                    tracing::error!("Failed to load from file: {}", e);
                                                    continue;
                                                }
                                            }
                                        } else {
                                            tracing::error!("Downloaded episode missing local_path");
                                            continue;
                                        }
                                    } else {
                                        match audio_streamer_clone.stream_episode(next_episode.id, &next_episode.url).await {
                                            Ok(state) => state.get_buffer().await,
                                            Err(e) => {
                                                tracing::error!("Failed to stream episode: {}", e);
                                                continue;
                                            }
                                        }
                                    };

                                    // Play the loaded audio
                                    if let Err(e) = audio_player_clone
                                        .play_from_memory(next_episode.id, &audio_data_result)
                                        .await
                                    {
                                        tracing::error!("Failed to play next episode: {}", e);
                                    } else {
                                        tracing::info!("Auto-playing: {}", next_episode.title);
                                    }
                                }
                                Ok(None) => {
                                    tracing::warn!("Episode {} not found in database", next_item.episode_id);
                                }
                                Err(e) => {
                                    tracing::error!("Failed to get next episode: {}", e);
                                }
                            }
                        }
                        Ok(None) => {
                            tracing::info!("Queue empty, no more episodes to play");
                        }
                        Err(e) => {
                            tracing::error!("Failed to get next from queue: {}", e);
                        }
                    }
                }
            });
        }

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
            current_view: View::Subscriptions,
            selected_index: 0,
            scroll_offset: 0,
            modal: Modal::None,
            search_input: String::new(),
            search_cursor: 0,
            status_message: None,
            show_help: false,
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

    pub async fn update(&mut self) -> Result<()> {
        // Update playback state
        self.is_playing = self.audio_player.is_playing().await;
        self.playback_speed = self.audio_player.get_speed().await;
        self.volume = self.audio_player.get_volume().await;

        // Load subscriptions if not loaded
        if self.subscriptions.is_empty() {
            self.load_subscriptions().await?;
        }

        Ok(())
    }

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
        } else if let Some(_episode) = &self.current_episode {
            self.audio_player.play().await;
        }
        Ok(())
    }

    pub fn set_view(&mut self, view: View) {
        self.current_view = view;
        self.selected_index = 0;
    }

    pub fn next_item(&mut self) {
        let max_index = match self.current_view {
            View::Subscriptions => self.subscriptions.len().saturating_sub(1),
            View::Episodes => self.episodes.len().saturating_sub(1),
            _ => 0,
        };

        if self.selected_index < max_index {
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

    async fn play_episode(&mut self, episode: Episode) -> Result<()> {
        tracing::info!("Playing episode: {}", episode.title);

        // 1. Load audio data (from file or stream)
        let audio_data = if episode.is_downloaded() {
            // Load from local file
            let path = episode
                .local_path
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Downloaded episode missing local_path"))?;

            tracing::debug!("Loading audio from file: {}", path.display());
            let state = self
                .audio_streamer
                .load_from_file(episode.id, Path::new(path))
                .await?;
            state.get_buffer().await
        } else {
            // Stream from URL
            tracing::debug!("Streaming audio from URL: {}", episode.url);
            let state = self
                .audio_streamer
                .stream_episode(episode.id, &episode.url)
                .await?;
            state.get_buffer().await
        };

        // 2. Play through audio player
        self.audio_player
            .play_from_memory(episode.id, &audio_data)
            .await?;

        // 3. Update app state
        self.current_episode = Some(episode.clone());
        self.is_playing = true;

        tracing::info!("Playback started successfully for: {}", episode.title);
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
        self.audio_player.seek_to(std::time::Duration::from_secs(0)).await?;
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
        self.current_view = match self.current_view {
            View::Subscriptions => View::Episodes,
            View::Episodes => View::Queue,
            View::Queue => View::Search,
            View::Search => View::Settings,
            View::Settings => View::Subscriptions,
        };
        self.selected_index = 0;
    }

    pub fn previous_view(&mut self) {
        self.current_view = match self.current_view {
            View::Subscriptions => View::Settings,
            View::Settings => View::Search,
            View::Search => View::Queue,
            View::Queue => View::Episodes,
            View::Episodes => View::Subscriptions,
        };
        self.selected_index = 0;
    }

    // List navigation methods

    pub fn goto_top(&mut self) {
        self.selected_index = 0;
    }

    pub fn goto_bottom(&mut self) {
        let max_index = match self.current_view {
            View::Subscriptions => self.subscriptions.len().saturating_sub(1),
            View::Episodes => self.episodes.len().saturating_sub(1),
            View::Queue => 0, // TODO: Get queue length
            _ => 0,
        };
        self.selected_index = max_index;
    }

    pub fn page_up(&mut self) {
        self.selected_index = self.selected_index.saturating_sub(10);
    }

    pub fn page_down(&mut self) {
        let max_index = match self.current_view {
            View::Subscriptions => self.subscriptions.len().saturating_sub(1),
            View::Episodes => self.episodes.len().saturating_sub(1),
            View::Queue => 0,
            _ => 0,
        };
        self.selected_index = (self.selected_index + 10).min(max_index);
    }

    // Item action methods

    pub async fn add_selected_to_queue(&mut self) -> Result<()> {
        if self.current_view == View::Episodes {
            if let Some(episode) = self.episodes.get(self.selected_index) {
                self.queue_manager.add_episode(episode.id).await?;
                tracing::info!("Added '{}' to queue", episode.title);
            }
        }
        Ok(())
    }

    pub async fn download_selected_episode(&mut self) -> Result<()> {
        if self.current_view == View::Episodes {
            if let Some(episode) = self.episodes.get(self.selected_index).cloned() {
                tracing::info!("Downloading episode: {}", episode.title);
                // Spawn download task to not block UI
                let download_manager = self.download_manager.clone();
                tokio::spawn(async move {
                    if let Err(e) = download_manager.download_episode(&episode).await {
                        tracing::error!("Download failed: {}", e);
                    }
                });
            }
        }
        Ok(())
    }

    pub async fn delete_selected_download(&mut self) -> Result<()> {
        if self.current_view == View::Episodes {
            if let Some(episode) = self.episodes.get(self.selected_index) {
                if episode.is_downloaded() {
                    self.delete_download(episode).await?;
                    // Reload episodes to update UI
                    if let Some(sub) = &self.current_subscription {
                        self.load_episodes_for_subscription(sub.id).await?;
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn refresh_selected_subscription(&mut self) -> Result<()> {
        if self.current_view == View::Subscriptions {
            if let Some(subscription) = self.subscriptions.get(self.selected_index).cloned() {
                tracing::info!("Refreshing subscription: {}", subscription.title);
                self.feed_refresher.refresh_one(subscription).await?;
            }
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
        if self.current_view == View::Episodes {
            if let Some(episode) = self.episodes.get(self.selected_index) {
                let new_status = !episode.played;
                self.db.mark_episode_played(episode.id, new_status).await?;
                tracing::info!("Marked episode as {}", if new_status { "played" } else { "unplayed" });

                // Reload episodes to update UI
                if let Some(sub) = &self.current_subscription {
                    self.load_episodes_for_subscription(sub.id).await?;
                }
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

    pub fn set_status(&mut self, message: String) {
        self.status_message = Some(message);
    }

    pub fn clear_status(&mut self) {
        self.status_message = None;
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
    pub async fn get_download_progress(&self, episode_id: uuid::Uuid) -> Option<Arc<crate::download::DownloadProgress>> {
        self.download_manager.get_download_progress(episode_id).await
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
