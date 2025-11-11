use crate::audio::{AudioPlayer, AudioStreamer};
use crate::download::DownloadManager;
use crate::feed::FeedRefresher;
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

    // UI state
    pub current_view: View,
    pub selected_index: usize,

    // Data
    pub subscriptions: Vec<Subscription>,
    pub episodes: Vec<Episode>,
    pub current_subscription: Option<Subscription>,

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
    ) -> Self {
        Self {
            config,
            db,
            audio_player,
            audio_streamer,
            queue_manager,
            download_manager,
            feed_refresher,
            current_view: View::Subscriptions,
            selected_index: 0,
            subscriptions: Vec::new(),
            episodes: Vec::new(),
            current_subscription: None,
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
}
