pub mod events;
pub mod state;

use crate::audio::{AudioPlayer, AudioStreamer};
use crate::download::DownloadManager;
use crate::feed::{FeedRefresher, OpmlExporter, OpmlImporter};
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

        // Create application state
        let state = AppState::new(
            config,
            db,
            audio_player,
            audio_streamer,
            queue_manager,
            download_manager,
            feed_refresher,
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
        use state::View;

        match key {
            KeyCode::Char(' ') => {
                // Play/pause
                self.state.toggle_playback().await?;
            }
            KeyCode::Char('1') => {
                self.state.set_view(View::Subscriptions);
            }
            KeyCode::Char('2') => {
                self.state.set_view(View::Queue);
            }
            KeyCode::Char('3') => {
                self.state.set_view(View::Search);
            }
            KeyCode::Up => {
                self.state.previous_item();
            }
            KeyCode::Down => {
                self.state.next_item();
            }
            KeyCode::Enter => {
                self.state.select_item().await?;
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
