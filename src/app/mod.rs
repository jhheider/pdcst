pub mod events;
pub mod state;

use crate::artwork::ArtworkManager;
use crate::audio::{AudioPlayer, AudioStreamer};
use crate::download::DownloadManager;
use crate::feed::{FeedRefresher, OpmlExporter, OpmlImporter, PodcastSearch};
use crate::models::Config;
use crate::queue::QueueManager;
use crate::storage::Database;
use crate::ui::Ui;
use anyhow::{Context, Result};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

pub use state::AppState;

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

        // Initialize audio player and streamer
        let audio_player = Arc::new(AudioPlayer::new()?);
        let audio_streamer = Arc::new(AudioStreamer::new());

        // Initialize managers
        let queue_manager = Arc::new(QueueManager::new(db.clone()));
        let download_manager = Arc::new(DownloadManager::new(
            config.download_dir.clone(),
            config.max_concurrent_downloads,
            db.clone(),
        ));
        let feed_refresher = Arc::new(FeedRefresher::new(
            config.max_concurrent_refreshes,
            db.clone(),
        ));
        let podcast_search = Arc::new(PodcastSearch::new());

        // Initialize artwork manager
        let artwork_manager = Arc::new(ArtworkManager::new(config.artwork_dir.clone()));

        // Load artwork cache from disk
        artwork_manager.load_cache_from_disk().await?;

        // Create application state
        let state = AppState::new(
            config,
            db,
            audio_player,
            audio_streamer,
            queue_manager,
            download_manager,
            feed_refresher,
            podcast_search,
            artwork_manager,
        );

        // Create UI
        let ui = Ui::new();

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
        loop {
            // Draw UI
            terminal.draw(|f| {
                self.ui.render(f, &self.state);
            })?;

            // Handle events with timeout
            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    match key.code {
                        KeyCode::Char('q') => {
                            tracing::info!("Quit requested");
                            break;
                        }
                        _ => {
                            self.handle_key_event(key.code).await?;
                        }
                    }
                }
            }

            // Update state
            self.state.update().await?;
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

        // Handle search input
        if self.state.current_view == View::Search && !matches!(key, KeyCode::Esc | KeyCode::Char('1'..='4')) {
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
                        match self.state.search_podcasts(&self.state.search_input.clone()).await {
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

            // Esc - close modal or exit search
            KeyCode::Esc => {
                if self.state.modal != Modal::None {
                    self.state.close_modal();
                } else if self.state.current_view == View::Search {
                    self.state.exit_search_mode();
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
                    self.state.show_error(format!("Failed to load queue: {}", e));
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
                if let Err(e) = self.state.add_selected_to_queue().await {
                    self.state.show_error(format!("Failed to add to queue: {}", e));
                } else {
                    self.state.set_status("Added to queue".to_string());
                    // Auto-clear status after showing
                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                    self.state.clear_status();
                }
            }
            KeyCode::Char('d') => {
                self.state.set_status("Starting download...".to_string());
                if let Err(e) = self.state.download_selected_episode().await {
                    self.state.show_error(format!("Download failed: {}", e));
                } else {
                    self.state.set_status("Download started".to_string());
                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                    self.state.clear_status();
                }
            }
            KeyCode::Char('x') => {
                if let Err(e) = self.state.delete_selected_download().await {
                    self.state.show_error(format!("Failed to delete: {}", e));
                } else {
                    self.state.set_status("Download deleted".to_string());
                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                    self.state.clear_status();
                }
            }
            KeyCode::Char('r') => {
                self.state.set_status("Refreshing feed...".to_string());
                if let Err(e) = self.state.refresh_selected_subscription().await {
                    self.state.show_error(format!("Refresh failed: {}", e));
                } else {
                    self.state.set_status("Feed refreshed".to_string());
                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                    self.state.clear_status();
                }
            }
            KeyCode::Char('R') => {
                self.state.set_status("Refreshing all feeds...".to_string());
                if let Err(e) = self.state.refresh_all_subscriptions().await {
                    self.state.show_error(format!("Refresh all failed: {}", e));
                } else {
                    self.state.set_status("All feeds refreshed".to_string());
                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                    self.state.clear_status();
                }
            }
            KeyCode::Char('s') => {
                if let Err(e) = self.state.toggle_played_status().await {
                    self.state.show_error(format!("Failed to toggle played: {}", e));
                } else {
                    self.state.set_status("Toggled played status".to_string());
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    self.state.clear_status();
                }
            }

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
        let subscriptions = tokio::task::spawn_blocking(move || {
            OpmlImporter::import_from_file(&path_buf)
        })
        .await
        .context("OPML import task panicked")??;

        let total = subscriptions.len();
        let mut imported = 0;

        // Insert each subscription
        for sub in subscriptions {
            match self.state.db.insert_subscription(&sub).await {
                Ok(_) => {
                    tracing::debug!("Imported subscription: {}", sub.title);
                    imported += 1;
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
