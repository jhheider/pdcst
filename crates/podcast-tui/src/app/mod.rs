pub mod events;
pub mod state;

pub use events::{EventBus, StateEvent};
pub use state::AppState;

use crate::artwork::ArtworkManager;
use crate::audio::{AudioPlayer, AudioStreamer};
use crate::download::DownloadManager;
use crate::feed::{FeedRefresher, OpmlExporter, OpmlImporter, PodcastSearch};
use crate::models::Config;
use crate::queue::QueueManager;
use crate::retention::RetentionManager;
use crate::storage::Database;
use crate::ui::Ui;
use anyhow::{Context, Result};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

pub struct App {
    state: AppState,
    ui: Ui,
}

impl App {
    pub async fn new(config: Config) -> Result<Self> {
        tracing::info!("Initializing application");

        // Initialize database
        let db_path = config.database_path();
        let db = Arc::new(Database::new(&db_path).await?);

        // Create event bus first (needed by audio player)
        let event_bus = Arc::new(EventBus::new());

        // Initialize audio player and streamer
        let audio_player = Arc::new(AudioPlayer::new(event_bus.clone())?);
        let audio_streamer = Arc::new(AudioStreamer::new(config.stream_cache_dir()));

        // Initialize managers
        let queue_manager = Arc::new(QueueManager::new(db.clone(), event_bus.clone()));
        let download_manager = Arc::new(DownloadManager::new(
            config.download_dir.clone(),
            config.max_concurrent_downloads,
            db.clone(),
            event_bus.clone(),
        ));
        let feed_refresher = Arc::new(FeedRefresher::new(
            config.max_concurrent_refreshes,
            db.clone(),
            event_bus.clone(),
        ));
        let podcast_search = Arc::new(PodcastSearch::new());

        // Initialize artwork manager
        let artwork_manager = Arc::new(ArtworkManager::new(config.artwork_dir.clone()));

        // Load artwork cache from disk
        artwork_manager.load_cache_from_disk().await?;

        // Clone for auto-advance task before moving into AppState
        let audio_player_clone = audio_player.clone();
        let audio_streamer_clone = audio_streamer.clone();
        let queue_manager_clone = queue_manager.clone();
        let db_clone = db.clone();
        let download_manager_finish = download_manager.clone();
        let delete_on_finish = config.delete_on_finish;

        // Clones + settings for the retention manager (config is moved below).
        let retention = Arc::new(RetentionManager::new(
            db.clone(),
            download_manager.clone(),
            audio_player.clone(),
            config.stream_cache_dir(),
            config.max_cache_episodes,
            config.max_cache_megabytes,
        ));

        // Create application state
        let state = AppState::new(
            config,
            db,
            state::Services {
                audio_player,
                audio_streamer,
                queue_manager,
                download_manager,
                feed_refresher,
                podcast_search,
                artwork_manager,
            },
            event_bus.clone(),
        );

        // Create UI
        let ui = Ui::new();

        // Set up auto-advance: subscribe to PlaybackCompleted events
        let mut event_rx_auto_advance = event_bus.subscribe();
        let event_bus_clone = event_bus.clone();

        tokio::spawn(async move {
            loop {
                match event_rx_auto_advance.recv().await {
                    Ok(StateEvent::PlaybackCompleted {
                        episode_id: completed_episode_id,
                    }) => {
                        tracing::info!(
                            "Episode {} completed, checking queue for next episode",
                            completed_episode_id
                        );

                        // Mark episode as played
                        match db_clone
                            .mark_episode_played(completed_episode_id, true)
                            .await
                        {
                            Err(e) => {
                                tracing::error!("Failed to mark episode as played: {}", e);
                            }
                            _ => {
                                // Emit episode marked played event
                                event_bus_clone.publish(StateEvent::EpisodeMarkedPlayed {
                                    episode_id: completed_episode_id,
                                });
                            }
                        }

                        // Delete-on-finish: reclaim a finished episode's download.
                        if delete_on_finish
                            && let Ok(Some(finished)) =
                                db_clone.get_episode(completed_episode_id).await
                            && finished.is_downloaded()
                        {
                            match download_manager_finish.delete_download(&finished).await {
                                Ok(()) => tracing::info!(
                                    "delete-on-finish: removed download for '{}'",
                                    finished.title
                                ),
                                Err(e) => tracing::warn!(
                                    "delete-on-finish failed for '{}': {}",
                                    finished.title,
                                    e
                                ),
                            }
                        }

                        // Remove from queue
                        if let Err(e) = queue_manager_clone
                            .remove_episode(completed_episode_id)
                            .await
                        {
                            tracing::error!("Failed to remove episode from queue: {}", e);
                        }

                        // Play next episode in queue (with retry on failure)
                        loop {
                            match queue_manager_clone.get_next().await {
                                Ok(Some(next_item)) => {
                                    let next_episode_id = next_item.episode_id;
                                    tracing::info!("Auto-advancing to next episode in queue");

                                    // Load the episode and play it (from the top),
                                    // reusing the shared play path so auto-advance
                                    // streams to disk exactly like a manual play.
                                    let load_result =
                                        match db_clone.get_episode(next_episode_id).await {
                                            Ok(Some(next_episode)) => {
                                                match state::load_and_play(
                                                    &audio_player_clone,
                                                    &audio_streamer_clone,
                                                    &next_episode,
                                                    std::time::Duration::ZERO,
                                                )
                                                .await
                                                {
                                                    Ok(()) => {
                                                        tracing::info!(
                                                            "Auto-playing: {}",
                                                            next_episode.title
                                                        );
                                                        event_bus_clone.publish(
                                                            StateEvent::QueueAdvanced {
                                                                next_episode_id,
                                                            },
                                                        );
                                                        Ok(())
                                                    }
                                                    Err(e) => Err(format!("Failed to play: {}", e)),
                                                }
                                            }
                                            Ok(None) => Err(format!(
                                                "Episode {} not found in database",
                                                next_episode_id
                                            )),
                                            Err(e) => Err(format!("Database error: {}", e)),
                                        };

                                    match load_result {
                                        Ok(_) => {
                                            // Successfully loaded and started playing
                                            break;
                                        }
                                        Err(error_msg) => {
                                            // Failed to play this episode - emit error and try next
                                            tracing::error!(
                                                "Failed to auto-play episode {}: {}",
                                                next_episode_id,
                                                error_msg
                                            );
                                            event_bus_clone.publish(StateEvent::PlaybackError {
                                                error: format!("Auto-play failed: {}", error_msg),
                                            });

                                            // Remove failed episode from queue and try next
                                            if let Err(e) = queue_manager_clone
                                                .remove_episode(next_episode_id)
                                                .await
                                            {
                                                tracing::error!(
                                                    "Failed to remove failed episode from queue: {}",
                                                    e
                                                );
                                                break;
                                            }

                                            // Continue loop to try next episode
                                        }
                                    }
                                }
                                Ok(None) => {
                                    tracing::info!("Queue empty, no more episodes to play");
                                    break;
                                }
                                Err(e) => {
                                    tracing::error!("Failed to get next from queue: {}", e);
                                    event_bus_clone.publish(StateEvent::PlaybackError {
                                        error: format!("Queue error: {}", e),
                                    });
                                    break;
                                }
                            }
                        }
                    }
                    Ok(_) => {
                        // Ignore other events
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!("Auto-advance task lagged, skipped {} events", skipped);
                        // Continue processing - we'll catch the next PlaybackCompleted
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        tracing::info!("Event bus closed, stopping auto-advance");
                        break;
                    }
                }
            }
        });

        // Disk retention: sweep the cache at startup (off the constructor's
        // path) and periodically thereafter so it never grows unbounded.
        {
            let retention = retention.clone();
            tokio::spawn(async move {
                retention.run_startup().await;
            });
        }
        retention.spawn_periodic();

        tracing::info!("Application initialized successfully");

        Ok(Self { state, ui })
    }

    pub async fn run(&mut self) -> Result<()> {
        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        tracing::info!("Entering main loop");

        // Run the main loop
        let result = self.run_loop(&mut terminal).await;

        // Restore terminal
        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        result
    }

    async fn run_loop(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<()> {
        // Subscribe to state events
        let mut event_rx = self.state.event_bus.subscribe();

        // Load initial data
        if self.state.subscriptions.is_empty() {
            self.state.load_subscriptions().await?;
        }

        // Restore the last session's episode (shown, not auto-played).
        if let Err(e) = self.state.restore_playback_state().await {
            tracing::warn!("Failed to restore playback state: {}", e);
        }

        // Initial draw
        terminal.draw(|f| {
            self.ui.render(f, &self.state);
        })?;

        // Create ticker for polling keyboard input (non-blocking)
        let mut tick_interval = tokio::time::interval(Duration::from_millis(50));

        loop {
            tokio::select! {
                // Handle state events
                event_result = event_rx.recv() => {
                    match event_result {
                        Ok(state_event) => {
                            // Update state based on event
                            self.handle_state_event(state_event).await?;

                            // Redraw UI
                            terminal.draw(|f| {
                                self.ui.render(f, &self.state);
                            })?;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                            tracing::warn!("UI event receiver lagged, skipped {} events - requesting full state refresh", skipped);

                            // When events are dropped, sync state from source of truth
                            self.state.is_playing = self.state.audio_player.is_playing().await;
                            self.state.volume = self.state.audio_player.get_volume().await;
                            self.state.playback_speed = self.state.audio_player.get_speed().await;
                            self.state.playback_position = self.state.audio_player.get_position().await;

                            // Redraw with refreshed state
                            terminal.draw(|f| {
                                self.ui.render(f, &self.state);
                            })?;
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            tracing::error!("Event bus closed unexpectedly");
                            break;
                        }
                    }
                }

                // Poll for keyboard input
                _ = tick_interval.tick() => {
                    let mut needs_redraw = false;

                    if event::poll(Duration::from_millis(0))?
                        && let Event::Key(key) = event::read()? {
                            // Ctrl-C is an always-available hard quit, even while
                            // typing (where plain 'q' is a literal character).
                            if key.code == KeyCode::Char('c')
                                && key.modifiers.contains(KeyModifiers::CONTROL)
                            {
                                tracing::info!("Hard quit (Ctrl-C)");
                                self.state.save_progress().await;
                                break;
                            }

                            self.handle_key_event(key.code).await?;

                            // The quit key sets a flag rather than breaking the
                            // loop directly, so it routes through handle_key_event
                            // and never fires while typing into the search box.
                            if self.state.should_quit {
                                tracing::info!("Quit requested");
                                self.state.save_progress().await;
                                break;
                            }
                            needs_redraw = true;
                        }

                    // Expire any transient status message (replaces the old
                    // blocking sleep-then-clear pattern in the action handlers).
                    if self.state.expire_status() {
                        needs_redraw = true;
                    }

                    if needs_redraw {
                        terminal.draw(|f| {
                            self.ui.render(f, &self.state);
                        })?;
                    }
                }
            }
        }

        Ok(())
    }

    async fn handle_state_event(&mut self, event: StateEvent) -> Result<()> {
        use StateEvent::*;
        use state::View;

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
                // Reload queue if we're on the queue view
                if self.state.current_view == View::Queue {
                    let _ = self.state.load_queue().await;
                }
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
            _ => {
                // Other events don't need immediate UI state updates
            }
        }

        Ok(())
    }

    async fn handle_key_event(&mut self, key: KeyCode) -> Result<()> {
        use state::{Modal, View};

        // Handle modal-specific keys first
        match &self.state.modal {
            Modal::Help | Modal::Error(_) => {
                match key {
                    KeyCode::Esc | KeyCode::Char('q') => {
                        self.state.close_modal();
                        return Ok(());
                    }
                    _ => return Ok(()), // Ignore other keys when modal is open
                }
            }
            Modal::Confirm { .. } => {
                match key {
                    KeyCode::Enter => {
                        // TODO: Execute confirmed action
                        self.state.close_modal();
                        return Ok(());
                    }
                    KeyCode::Esc => {
                        self.state.close_modal();
                        return Ok(());
                    }
                    _ => return Ok(()),
                }
            }
            Modal::None => {}
        }

        // Handle search input. While the box has focus, every printable key
        // (digits and 'q' included) types into it; only Esc (exit) and Enter
        // (run) escape to the global handlers, so typing can never switch views
        // or quit the app. Arrow/Page keys fall through to navigate results.
        if self.state.current_view == View::Search && !matches!(key, KeyCode::Esc) {
            match key {
                KeyCode::Char(c) if !c.is_control() => {
                    self.state.append_search_char(c);
                    return Ok(());
                }
                KeyCode::Backspace => {
                    self.state.delete_search_char();
                    return Ok(());
                }
                KeyCode::Enter => {
                    // Trigger search
                    if !self.state.search_input.is_empty() {
                        self.state.set_status("Searching...".to_string());
                        match self
                            .state
                            .search_podcasts(&self.state.search_input.clone())
                            .await
                        {
                            Ok(_) => {
                                self.state.selected_index = 0;
                                self.state.clear_status();
                            }
                            Err(e) => {
                                self.state.show_error(format!("Search failed: {}", e));
                            }
                        }
                    }
                    return Ok(());
                }
                _ => {}
            }
        }

        // Global shortcuts
        match key {
            // Help modal
            KeyCode::Char('?') => {
                self.state.show_help_modal();
                return Ok(());
            }

            // Quit (a literal 'q' while typing is handled by the search gate above).
            KeyCode::Char('q') => {
                self.state.should_quit = true;
                return Ok(());
            }

            // Esc - close a modal, leave search, or drill back out of Episodes.
            KeyCode::Esc => {
                if self.state.modal != Modal::None {
                    self.state.close_modal();
                } else if self.state.current_view == View::Search {
                    self.state.exit_search_mode();
                } else if self.state.current_view == View::Episodes {
                    self.state.set_view(View::Subscriptions);
                }
                return Ok(());
            }

            // Playback controls
            KeyCode::Char(' ') => {
                if let Err(e) = self.state.toggle_playback().await {
                    self.state.show_error(format!("Playback error: {}", e));
                }
            }
            KeyCode::Char('n') => {
                if let Err(e) = self.state.play_next_in_queue().await {
                    self.state.show_error(format!("Failed to play next: {}", e));
                }
            }
            KeyCode::Char('p') | KeyCode::Char('P') => {
                if let Err(e) = self.state.restart_current_episode().await {
                    self.state.show_error(format!("Failed to restart: {}", e));
                }
            }

            // Volume controls
            KeyCode::Char('+') | KeyCode::Char('=') => {
                if let Err(e) = self.state.increase_volume(0.1).await {
                    tracing::error!("Volume error: {}", e);
                }
            }
            KeyCode::Char('-') | KeyCode::Char('_') => {
                if let Err(e) = self.state.decrease_volume(0.1).await {
                    tracing::error!("Volume error: {}", e);
                }
            }
            KeyCode::Char('m') => {
                if let Err(e) = self.state.toggle_mute().await {
                    tracing::error!("Mute error: {}", e);
                }
            }

            // Playback speed controls
            KeyCode::Char('[') => {
                if let Err(e) = self.state.decrease_speed(0.1).await {
                    tracing::error!("Speed error: {}", e);
                }
            }
            KeyCode::Char(']') => {
                if let Err(e) = self.state.increase_speed(0.1).await {
                    tracing::error!("Speed error: {}", e);
                }
            }

            // Seeking
            KeyCode::Left => {
                if let Err(e) = self.state.seek_backward(10.0).await {
                    tracing::error!("Seek error: {}", e);
                }
            }
            KeyCode::Right => {
                if let Err(e) = self.state.seek_forward(10.0).await {
                    tracing::error!("Seek error: {}", e);
                }
            }
            KeyCode::Char('<') => {
                if let Err(e) = self.state.seek_backward(30.0).await {
                    tracing::error!("Seek error: {}", e);
                }
            }
            KeyCode::Char('>') => {
                if let Err(e) = self.state.seek_forward(30.0).await {
                    tracing::error!("Seek error: {}", e);
                }
            }

            // View navigation
            KeyCode::Char('1') => {
                self.state.set_view(View::Subscriptions);
                self.state.selected_index = 0;
            }
            KeyCode::Char('2') => {
                self.state.set_view(View::Queue);
                self.state.selected_index = 0;
                // Load queue items
                if let Err(e) = self.state.load_queue().await {
                    self.state
                        .show_error(format!("Failed to load queue: {}", e));
                }
            }
            KeyCode::Char('3') => {
                self.state.set_view(View::Search);
                self.state.selected_index = 0;
                self.state.clear_search_input();
            }
            KeyCode::Char('4') => {
                self.state.set_view(View::Settings);
                self.state.selected_index = 0;
            }
            KeyCode::Tab => {
                self.state.next_view();
                self.state.selected_index = 0;
                // Load data for new view
                if self.state.current_view == View::Queue {
                    let _ = self.state.load_queue().await;
                }
            }
            KeyCode::BackTab => {
                self.state.previous_view();
                self.state.selected_index = 0;
                // Load data for new view
                if self.state.current_view == View::Queue {
                    let _ = self.state.load_queue().await;
                }
            }

            // List navigation
            KeyCode::Up | KeyCode::Char('k') => {
                self.state.previous_item();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.state.next_item();
            }
            KeyCode::Char('g') => {
                self.state.goto_top();
            }
            KeyCode::Char('G') => {
                self.state.goto_bottom();
            }
            KeyCode::PageUp => {
                self.state.page_up();
            }
            KeyCode::PageDown => {
                self.state.page_down();
            }

            // Item selection and actions
            KeyCode::Enter => {
                if let Err(e) = self.state.select_item().await {
                    self.state.show_error(format!("Selection failed: {}", e));
                }
            }
            KeyCode::Char('a') => {
                match self.state.add_selected_to_queue().await {
                    Err(e) => {
                        self.state
                            .show_error(format!("Failed to add to queue: {}", e));
                    }
                    _ => {
                        // Transient status auto-clears at render time (no block).
                        self.state.set_status("Added to queue".to_string());
                    }
                }
            }
            KeyCode::Char('d') => {
                self.state.set_status("Starting download...".to_string());
                match self.state.download_selected_episode().await {
                    Err(e) => {
                        self.state.show_error(format!("Download failed: {}", e));
                    }
                    _ => {
                        self.state.set_status("Download started".to_string());
                    }
                }
            }
            KeyCode::Char('x') => match self.state.delete_selected_download().await {
                Err(e) => {
                    self.state.show_error(format!("Failed to delete: {}", e));
                }
                _ => {
                    self.state.set_status("Download deleted".to_string());
                }
            },
            KeyCode::Char('r') => {
                self.state.set_status("Refreshing feed...".to_string());
                match self.state.refresh_selected_subscription().await {
                    Err(e) => {
                        self.state.show_error(format!("Refresh failed: {}", e));
                    }
                    _ => {
                        self.state.set_status("Feed refreshed".to_string());
                    }
                }
            }
            KeyCode::Char('R') => {
                self.state.set_status("Refreshing all feeds...".to_string());
                match self.state.refresh_all_subscriptions().await {
                    Err(e) => {
                        self.state.show_error(format!("Refresh all failed: {}", e));
                    }
                    _ => {
                        self.state.set_status("All feeds refreshed".to_string());
                    }
                }
            }
            KeyCode::Char('s') => match self.state.toggle_played_status().await {
                Err(e) => {
                    self.state
                        .show_error(format!("Failed to toggle played: {}", e));
                }
                _ => {
                    self.state.set_status("Toggled played status".to_string());
                }
            },

            // Search
            KeyCode::Char('/') => {
                self.state.enter_search_mode();
            }

            _ => {}
        }

        Ok(())
    }

    /// Import subscriptions from an OPML file
    ///
    /// Reads an OPML file and subscribes to all podcast feeds found within.
    /// Existing subscriptions with duplicate RSS URLs will be skipped.
    ///
    /// Returns the number of subscriptions successfully imported.
    pub async fn import_opml(&mut self, path: &Path) -> Result<usize> {
        tracing::info!("Importing OPML from: {}", path.display());

        // Parse OPML file (sync I/O in tokio::task::spawn_blocking)
        let path_buf = path.to_path_buf();
        let subscriptions =
            tokio::task::spawn_blocking(move || OpmlImporter::import_from_file(&path_buf))
                .await
                .context("OPML import task panicked")??;

        let total = subscriptions.len();
        let mut imported = 0;

        // Insert each subscription
        for sub in subscriptions {
            let subscription_id = sub.id;
            match self.state.db.insert_subscription(&sub).await {
                Ok(_) => {
                    tracing::debug!("Imported subscription: {}", sub.title);
                    imported += 1;

                    // Emit event
                    self.state
                        .event_bus
                        .publish(StateEvent::SubscriptionAdded { subscription_id });
                }
                Err(e) => {
                    // Log but continue - might be duplicate RSS URL
                    tracing::warn!("Failed to import {}: {}", sub.title, e);
                }
            }
        }

        // Reload subscriptions in UI
        self.state.load_subscriptions().await?;

        tracing::info!("Imported {}/{} subscriptions", imported, total);
        Ok(imported)
    }

    /// Export all subscriptions to an OPML file
    ///
    /// Writes all current subscriptions to an OPML file that can be imported
    /// by other podcast clients or re-imported later.
    pub async fn export_opml(&self, path: &Path) -> Result<()> {
        tracing::info!("Exporting OPML to: {}", path.display());

        // Get all subscriptions from database
        let subscriptions = self.state.db.get_all_subscriptions().await?;
        let count = subscriptions.len();

        // Write OPML file (sync I/O in tokio::task::spawn_blocking)
        let path_buf = path.to_path_buf();
        tokio::task::spawn_blocking(move || {
            OpmlExporter::export_to_file(&subscriptions, &path_buf)
        })
        .await
        .context("OPML export task panicked")??;

        tracing::info!("Exported {} subscriptions", count);
        Ok(())
    }
}
