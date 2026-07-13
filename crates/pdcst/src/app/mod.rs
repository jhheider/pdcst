mod auto_advance;
pub mod events;
mod input;
mod opml;
pub mod state;
mod update;

pub use events::{EventBus, StateEvent};
pub use state::AppState;

use crate::audio::{AudioPlayer, AudioStreamer};
use crate::download::DownloadManager;
use crate::feed::{FeedRefresher, PodcastSearch};
use crate::models::Config;
use crate::queue::QueueManager;
use crate::retention::RetentionManager;
use crate::storage::Database;
use crate::ui::Ui;
use anyhow::Result;
use crossterm::{
    cursor::Show,
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;
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
        let audio_streamer = Arc::new(AudioStreamer::new(
            config.download_dir.clone(),
            db.clone(),
            event_bus.clone(),
        ));

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
            queue_manager.clone(),
            crate::feed::AutoQueuePolicy {
                max_depth: config.queue_max_depth,
                interleave: config.smart_interleave,
            },
        ));
        let podcast_search = Arc::new(PodcastSearch::new());

        // Clone for auto-advance task before moving into AppState
        let audio_player_clone = audio_player.clone();
        let audio_streamer_clone = audio_streamer.clone();
        let queue_manager_clone = queue_manager.clone();
        let db_clone = db.clone();
        let download_manager_finish = download_manager.clone();
        let delete_on_finish = config.delete_on_finish;

        // Clones + setting for the background refresh scheduler.
        let refresh_scheduler_feed = feed_refresher.clone();
        let refresh_scheduler_db = db.clone();
        let auto_refresh_interval = config.auto_refresh_interval_minutes;

        // Clones + settings for the retention manager (config is moved below).
        let retention = Arc::new(RetentionManager::new(
            db.clone(),
            download_manager.clone(),
            audio_player.clone(),
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
            },
            event_bus.clone(),
        );

        // Create UI
        let ui = Ui::new();

        // Auto-advance: play the next queued episode when one finishes.
        auto_advance::spawn(auto_advance::AutoAdvance {
            event_rx: event_bus.subscribe(),
            db: db_clone,
            event_bus: event_bus.clone(),
            queue_manager: queue_manager_clone,
            audio_player: audio_player_clone,
            audio_streamer: audio_streamer_clone,
            download_manager: download_manager_finish,
            delete_on_finish,
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

        // Background refresh: refresh all feeds at launch and on the configured
        // interval, feeding new episodes into the auto-queue.
        crate::feed::spawn_auto_refresh(
            refresh_scheduler_feed,
            refresh_scheduler_db,
            auto_refresh_interval,
        );

        tracing::info!("Application initialized successfully");

        Ok(Self { state, ui })
    }

    pub async fn run(&mut self) -> Result<()> {
        // Setup terminal. No mouse capture: pdcst handles no mouse events, and
        // capturing it would steal the terminal's own text selection/scroll.
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;

        // Restore the terminal on every exit path, including a panic in the run
        // loop, so a crash never leaves the shell wedged in raw/alt-screen mode.
        let _guard = TerminalGuard;

        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend)?;

        tracing::info!("Entering main loop");
        self.run_loop(&mut terminal).await
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
        // Populate the library's right (Episodes) pane against the first feed, so
        // it is not blank before the first keypress.
        if let Err(e) = self.state.preview_selected_subscription().await {
            tracing::warn!("Failed to preview first subscription: {}", e);
        }

        // Restore the last session's episode (shown, not auto-played).
        if let Err(e) = self.state.restore_playback_state().await {
            tracing::warn!("Failed to restore playback state: {}", e);
        }

        // Load the queue so "Up Next: N" is correct from the first frame.
        if let Err(e) = self.state.load_queue().await {
            tracing::warn!("Failed to load queue: {}", e);
        }

        // Initial draw
        terminal.draw(|f| {
            self.ui.render(f, &mut self.state);
        })?;

        // Create ticker for polling keyboard input (non-blocking)
        let mut tick_interval = tokio::time::interval(Duration::from_millis(50));

        // Termination signals (Unix): a `kill`, a closed terminal/tab (SIGHUP),
        // or an SSH drop ends the process without going through our quit key.
        // Register them once and route them through the same save-then-break path
        // so an abrupt exit still checkpoints where we left off. Raw mode swallows
        // Ctrl-C as a key event (not SIGINT), so SIGINT is handled there already.
        #[cfg(unix)]
        let (mut sigterm, mut sighup) = {
            use tokio::signal::unix::{SignalKind, signal};
            (
                signal(SignalKind::terminate())?,
                signal(SignalKind::hangup())?,
            )
        };

        loop {
            // A future that resolves when a termination signal arrives (never, on
            // non-Unix). Recreated each iteration over the persistent handles;
            // `recv` is cancel-safe, so losing this branch to another arm is fine.
            let termination = async {
                #[cfg(unix)]
                tokio::select! {
                    _ = sigterm.recv() => {}
                    _ = sighup.recv() => {}
                }
                #[cfg(not(unix))]
                std::future::pending::<()>().await;
            };

            tokio::select! {
                // A termination signal: save and exit gracefully, like the quit key.
                _ = termination => {
                    tracing::info!("Received termination signal; saving and exiting");
                    self.state.save_progress().await;
                    break;
                }

                // Handle state events
                event_result = event_rx.recv() => {
                    match event_result {
                        Ok(state_event) => {
                            // Update state based on event
                            self.handle_state_event(state_event).await?;

                            // Redraw UI
                            terminal.draw(|f| {
                                self.ui.render(f, &mut self.state);
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
                                self.ui.render(f, &mut self.state);
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
                    let mut quit = false;

                    // Drain every pending terminal event this tick: key bursts
                    // (held j/k) all get processed, and a Resize repaints even
                    // when no key was pressed (otherwise the screen stays garbled
                    // after a window drag until the next keypress).
                    while event::poll(Duration::from_millis(0))? {
                        match event::read()? {
                            Event::Key(key) => {
                                // Ctrl-C is an always-available hard quit, even
                                // while typing (where plain 'q' is a character).
                                if key.code == KeyCode::Char('c')
                                    && key.modifiers.contains(KeyModifiers::CONTROL)
                                {
                                    tracing::info!("Hard quit (Ctrl-C)");
                                    quit = true;
                                    break;
                                }

                                self.handle_key_event(key.code).await?;

                                // The quit key sets a flag (routed through the key
                                // handler) so it never fires while typing.
                                if self.state.should_quit {
                                    quit = true;
                                    break;
                                }
                                needs_redraw = true;
                            }
                            Event::Resize(_, _) => needs_redraw = true,
                            _ => {}
                        }
                    }

                    if quit {
                        tracing::info!("Quit requested");
                        self.state.save_progress().await;
                        break;
                    }

                    // Expire any transient status message (replaces the old
                    // blocking sleep-then-clear pattern in the action handlers).
                    if self.state.expire_status() {
                        needs_redraw = true;
                    }

                    if needs_redraw {
                        terminal.draw(|f| {
                            self.ui.render(f, &mut self.state);
                        })?;
                    }
                }
            }
        }

        Ok(())
    }
}

/// Restores the terminal (cooked mode, main screen, cursor shown) when dropped.
/// Held for the lifetime of the run loop so a panic unwinding through it still
/// leaves the shell usable.
struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
    }
}
