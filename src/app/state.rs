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

pub struct AppState {
    pub config: Config,
    pub db: Arc<Database>,
    pub audio_player: Arc<AudioPlayer>,
    pub audio_streamer: Arc<AudioStreamer>,
    pub queue_manager: Arc<QueueManager>,
    pub download_manager: Arc<DownloadManager>,
    pub feed_refresher: Arc<FeedRefresher>,
    pub podcast_search: Arc<PodcastSearch>,

    // UI state
    pub current_view: View,
    pub selected_index: usize,

    // Data
    pub subscriptions: Vec<Subscription>,
    pub episodes: Vec<Episode>,
    pub current_subscription: Option<Subscription>,
    pub search_results: Vec<SearchResult>,

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
            current_view: View::Subscriptions,
            selected_index: 0,
            subscriptions: Vec::new(),
            episodes: Vec::new(),
            current_subscription: None,
            search_results: Vec::new(),
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
}
