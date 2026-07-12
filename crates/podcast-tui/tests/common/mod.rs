//! Shared test scaffolding: build a fully-wired `AppState` over a temp DB and
//! fabricate episodes. Used by the render-smoke and queue-ops integration tests.

use std::sync::Arc;

use chrono::Utc;
use podcast_tui::app::events::EventBus;
use podcast_tui::app::state::{AppState, Services};
use podcast_tui::artwork::ArtworkManager;
use podcast_tui::audio::{AudioPlayer, AudioStreamer};
use podcast_tui::download::DownloadManager;
use podcast_tui::feed::{FeedRefresher, PodcastSearch};
use podcast_tui::models::{Config, Episode};
use podcast_tui::queue::QueueManager;
use podcast_tui::storage::Database;
use tempfile::TempDir;

/// A ready-to-use `AppState` over a temp database. The returned `TempDir` must
/// be kept alive for the duration of the test (it owns the DB file).
pub async fn build_state() -> (AppState, TempDir) {
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
        feed_refresher: Arc::new(FeedRefresher::new(
            5,
            db.clone(),
            event_bus.clone(),
            Arc::new(QueueManager::new(db.clone(), event_bus.clone())),
            podcast_tui::feed::AutoQueuePolicy {
                max_depth: 20,
                interleave: true,
            },
        )),
        podcast_search: Arc::new(PodcastSearch::new()),
        artwork_manager: Arc::new(ArtworkManager::new(config.artwork_dir.clone())),
    };

    (AppState::new(config, db, services, event_bus), dir)
}

/// A fabricated episode for `sub_id`, with a listen state and a duration.
pub fn sample_episode(sub_id: uuid::Uuid, title: &str, played: bool, position: i64) -> Episode {
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
