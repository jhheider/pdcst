//! Render smoke tests: every view must draw into a TestBackend without
//! panicking, with realistic data (a subscription, episodes with each listen
//! state, a non-empty queue, a now-playing episode). CI has no terminal, so this
//! is the only automated guard on the rendering layer - exactly where the app's
//! showstoppers historically lived.

use std::sync::Arc;

use chrono::Utc;
use podcast_tui::app::events::EventBus;
use podcast_tui::app::state::{AppState, Services, View};
use podcast_tui::artwork::ArtworkManager;
use podcast_tui::audio::{AudioPlayer, AudioStreamer};
use podcast_tui::download::DownloadManager;
use podcast_tui::feed::{FeedRefresher, PodcastSearch};
use podcast_tui::models::{Config, Episode, Subscription};
use podcast_tui::queue::QueueManager;
use podcast_tui::storage::Database;
use podcast_tui::ui::Ui;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use tempfile::TempDir;

async fn build_state() -> (AppState, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        data_dir: dir.path().to_path_buf(),
        download_dir: dir.path().join("downloads"),
        artwork_dir: dir.path().join("artwork"),
        ..Config::default()
    };

    let db = Arc::new(Database::new(&dir.path().join("test.db")).await.unwrap());
    let event_bus = Arc::new(EventBus::new());
    let services = Services {
        audio_player: Arc::new(AudioPlayer::new(event_bus.clone()).unwrap()),
        audio_streamer: Arc::new(AudioStreamer::new(config.stream_cache_dir())),
        queue_manager: Arc::new(QueueManager::new(db.clone(), event_bus.clone())),
        download_manager: Arc::new(DownloadManager::new(
            config.download_dir.clone(),
            3,
            db.clone(),
            event_bus.clone(),
        )),
        feed_refresher: Arc::new(FeedRefresher::new(5, db.clone(), event_bus.clone())),
        podcast_search: Arc::new(PodcastSearch::new()),
        artwork_manager: Arc::new(ArtworkManager::new(config.artwork_dir.clone())),
    };

    (AppState::new(config, db, services, event_bus), dir)
}

fn sample_episode(sub_id: uuid::Uuid, title: &str, played: bool, position: i64) -> Episode {
    let mut ep = Episode::new(
        sub_id,
        title.to_string(),
        format!("https://example.com/{title}.mp3"),
        format!("guid-{title}"),
        Utc::now(),
    );
    ep.played = played;
    ep.playback_position_seconds = position;
    ep.duration_seconds = Some(2700);
    ep
}

fn draw(ui: &Ui, state: &mut AppState) {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| ui.render(f, state))
        .expect("render must not panic");
}

#[tokio::test]
async fn renders_every_view_empty() {
    let (mut state, _dir) = build_state().await;
    let ui = Ui::new();
    for view in [
        View::Subscriptions,
        View::Episodes,
        View::Queue,
        View::Search,
        View::Settings,
    ] {
        state.set_view(view);
        draw(&ui, &mut state);
    }
}

#[tokio::test]
async fn renders_populated_lists_with_markers() {
    let (mut state, _dir) = build_state().await;
    let ui = Ui::new();

    let sub = Subscription::new(
        "Test Show".to_string(),
        "https://example.com/feed.xml".to_string(),
    );
    let unplayed = sample_episode(sub.id, "Unplayed", false, 0);
    let in_progress = sample_episode(sub.id, "In Progress", false, 600);
    let played = sample_episode(sub.id, "Played", true, 2700);

    state.current_subscription = Some(sub.clone());
    state.subscriptions = vec![sub];
    state.episodes = vec![unplayed, in_progress.clone(), played];
    state.queue_items = state.episodes.clone();
    // A now-playing episode drives the ">" marker and the playback bar.
    state.current_episode = Some(in_progress);
    state.is_playing = true;
    state.playback_position = 600.0;

    // Selection near the bottom exercises the scroll offset, and each list view
    // renders its markers.
    for view in [View::Subscriptions, View::Episodes, View::Queue] {
        state.set_view(view);
        state.selected_index = 2;
        draw(&ui, &mut state);
    }

    // Search with results.
    state.set_view(View::Search);
    state.search_input = "test".to_string();
    draw(&ui, &mut state);
}

#[tokio::test]
async fn renders_modals() {
    let (mut state, _dir) = build_state().await;
    let ui = Ui::new();

    state.show_help_modal();
    draw(&ui, &mut state);

    state.show_error("something broke".to_string());
    draw(&ui, &mut state);
}
