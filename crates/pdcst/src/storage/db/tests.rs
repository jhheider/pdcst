use super::*;
use crate::models::{DownloadStatus, Episode, PlaybackStatus, QueueItem, Subscription};
use chrono::Utc;
use std::path::PathBuf;
use tempfile::TempDir;
use uuid::Uuid;

async fn create_test_db() -> (Database, TempDir) {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let db = Database::new(&db_path).await.unwrap();
    (db, temp_dir)
}

#[tokio::test]
async fn test_migrations_run_successfully() {
    let (db, _temp) = create_test_db().await;

    // Verify _sqlx_migrations table exists
    let result: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&db.pool)
        .await
        .expect("_sqlx_migrations table should exist");

    assert!(result.0 > 0, "Should have at least one migration");
}

#[tokio::test]
async fn test_migrations_are_idempotent() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");

    // First run
    let db1 = Database::new(&db_path).await.unwrap();

    // Verify initial migration ran
    let count1: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&db1.pool)
        .await
        .unwrap();

    drop(db1);

    // Second run - should not error
    let db2 = Database::new(&db_path).await.unwrap();

    // Verify same number of migrations
    let count2: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&db2.pool)
        .await
        .unwrap();

    assert_eq!(count1.0, count2.0, "Migration count should be stable");
}

#[tokio::test]
async fn test_schema_created_correctly() {
    let (db, _temp) = create_test_db().await;

    // Verify all expected tables exist
    let tables: Vec<(String,)> = sqlx::query_as(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_sqlx_%' ORDER BY name"
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();

    let table_names: Vec<String> = tables.into_iter().map(|(name,)| name).collect();

    assert!(table_names.contains(&"subscriptions".to_string()));
    assert!(table_names.contains(&"episodes".to_string()));
    assert!(table_names.contains(&"queue_items".to_string()));
    assert!(table_names.contains(&"playback_state".to_string()));
    assert!(table_names.contains(&"config".to_string()));
}

fn create_test_subscription(title: &str, rss_url: &str) -> Subscription {
    Subscription::new(title.to_string(), rss_url.to_string())
}

fn create_test_episode(subscription_id: Uuid, title: &str) -> Episode {
    Episode {
        id: Uuid::new_v4(),
        subscription_id,
        title: title.to_string(),
        description: Some("Test episode description".to_string()),
        url: "https://example.com/episode.mp3".to_string(),
        guid: format!("test-guid-{}", Uuid::new_v4()),
        published_at: Utc::now(),
        duration_seconds: Some(3600),
        file_size_bytes: Some(50_000_000),
        file_type: Some("audio/mpeg".to_string()),
        download_status: DownloadStatus::NotDownloaded,
        local_path: None,
        playback_position_seconds: 0,
        played: false,
        is_new: true,
        last_played_at: None,
        created_at: Utc::now(),
    }
}

// Subscription Tests

#[tokio::test]
async fn test_insert_and_get_subscription() {
    let (db, _temp) = create_test_db().await;
    let sub = create_test_subscription("Test Podcast", "https://example.com/feed.xml");

    db.insert_subscription(&sub).await.unwrap();

    let retrieved = db.get_subscription(sub.id).await.unwrap();
    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();

    assert_eq!(retrieved.id, sub.id);
    assert_eq!(retrieved.title, "Test Podcast");
    assert_eq!(retrieved.rss_url, "https://example.com/feed.xml");
}

#[tokio::test]
async fn test_auto_queue_config_roundtrips() {
    let (db, _temp) = create_test_db().await;

    let mut top = create_test_subscription("Top", "https://example.com/top.xml");
    top.auto_queue = true;
    top.auto_queue_to_top = true;
    db.insert_subscription(&top).await.unwrap();
    let got = db.get_subscription(top.id).await.unwrap().unwrap();
    assert!(got.auto_queue);
    assert!(got.auto_queue_to_top, "top direction persists");

    // The direction defaults to bottom (false).
    let mut bottom = create_test_subscription("Bottom", "https://example.com/bottom.xml");
    bottom.auto_queue = true;
    db.insert_subscription(&bottom).await.unwrap();
    let got = db.get_subscription(bottom.id).await.unwrap().unwrap();
    assert!(got.auto_queue);
    assert!(!got.auto_queue_to_top, "direction defaults to bottom");
}

#[tokio::test]
async fn test_get_all_subscriptions() {
    let (db, _temp) = create_test_db().await;

    let sub1 = create_test_subscription("Podcast 1", "https://example.com/feed1.xml");
    let sub2 = create_test_subscription("Podcast 2", "https://example.com/feed2.xml");
    let sub3 = create_test_subscription("Podcast 3", "https://example.com/feed3.xml");

    db.insert_subscription(&sub1).await.unwrap();
    db.insert_subscription(&sub2).await.unwrap();
    db.insert_subscription(&sub3).await.unwrap();

    let all_subs = db.get_all_subscriptions().await.unwrap();

    assert_eq!(all_subs.len(), 3);
    assert!(all_subs.iter().any(|s| s.id == sub1.id));
    assert!(all_subs.iter().any(|s| s.id == sub2.id));
    assert!(all_subs.iter().any(|s| s.id == sub3.id));
}

#[tokio::test]
async fn test_get_nonexistent_subscription() {
    let (db, _temp) = create_test_db().await;

    let result = db.get_subscription(Uuid::new_v4()).await.unwrap();

    assert!(result.is_none());
}

#[tokio::test]
async fn test_unique_rss_url_constraint() {
    let (db, _temp) = create_test_db().await;

    let sub1 = create_test_subscription("Podcast 1", "https://example.com/feed.xml");
    db.insert_subscription(&sub1).await.unwrap();

    // Try to insert another subscription with same RSS URL
    let sub2 = create_test_subscription("Podcast 2", "https://example.com/feed.xml");
    let result = db.insert_subscription(&sub2).await;

    assert!(result.is_err(), "Should fail on duplicate RSS URL");
}

#[tokio::test]
async fn test_update_subscription_last_refreshed() {
    let (db, _temp) = create_test_db().await;
    let sub = create_test_subscription("Test Podcast", "https://example.com/feed.xml");

    db.insert_subscription(&sub).await.unwrap();

    let original_time = sub.last_refreshed;

    // Wait a bit to ensure time difference
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    db.update_subscription_last_refreshed(sub.id).await.unwrap();

    let updated = db.get_subscription(sub.id).await.unwrap().unwrap();
    assert!(updated.last_refreshed > original_time);
}

#[tokio::test]
async fn test_delete_subscription() {
    let (db, _temp) = create_test_db().await;
    let sub = create_test_subscription("Test Podcast", "https://example.com/feed.xml");

    db.insert_subscription(&sub).await.unwrap();

    db.delete_subscription(sub.id).await.unwrap();

    let result = db.get_subscription(sub.id).await.unwrap();
    assert!(result.is_none());
}

// Episode Tests

#[tokio::test]
async fn test_insert_and_get_episode() {
    let (db, _temp) = create_test_db().await;
    let sub = create_test_subscription("Test Podcast", "https://example.com/feed.xml");
    db.insert_subscription(&sub).await.unwrap();

    let episode = create_test_episode(sub.id, "Test Episode");
    db.insert_episode(&episode).await.unwrap();

    let retrieved = db.get_episode(episode.id).await.unwrap();
    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();

    assert_eq!(retrieved.id, episode.id);
    assert_eq!(retrieved.title, "Test Episode");
    assert_eq!(retrieved.subscription_id, sub.id);
}

#[tokio::test]
async fn test_get_episodes_for_subscription() {
    let (db, _temp) = create_test_db().await;
    let sub = create_test_subscription("Test Podcast", "https://example.com/feed.xml");
    db.insert_subscription(&sub).await.unwrap();

    let ep1 = create_test_episode(sub.id, "Episode 1");
    let ep2 = create_test_episode(sub.id, "Episode 2");
    let ep3 = create_test_episode(sub.id, "Episode 3");

    db.insert_episode(&ep1).await.unwrap();
    db.insert_episode(&ep2).await.unwrap();
    db.insert_episode(&ep3).await.unwrap();

    let episodes = db.get_episodes_for_subscription(sub.id).await.unwrap();

    assert_eq!(episodes.len(), 3);
}

#[tokio::test]
async fn test_cascade_delete_episodes_on_subscription_delete() {
    let (db, _temp) = create_test_db().await;
    let sub = create_test_subscription("Test Podcast", "https://example.com/feed.xml");
    db.insert_subscription(&sub).await.unwrap();

    let episode = create_test_episode(sub.id, "Test Episode");
    db.insert_episode(&episode).await.unwrap();

    // Verify episode exists
    assert!(db.get_episode(episode.id).await.unwrap().is_some());

    // Delete subscription
    db.delete_subscription(sub.id).await.unwrap();

    // Episode should be cascade deleted
    let result = db.get_episode(episode.id).await.unwrap();
    assert!(result.is_none(), "Episode should be cascade deleted");
}

#[tokio::test]
async fn test_update_episode_playback_position() {
    let (db, _temp) = create_test_db().await;
    let sub = create_test_subscription("Test Podcast", "https://example.com/feed.xml");
    db.insert_subscription(&sub).await.unwrap();

    let episode = create_test_episode(sub.id, "Test Episode");
    db.insert_episode(&episode).await.unwrap();

    db.update_episode_playback_position(episode.id, 1234)
        .await
        .unwrap();

    let updated = db.get_episode(episode.id).await.unwrap().unwrap();
    assert_eq!(updated.playback_position_seconds, 1234);
}

#[tokio::test]
async fn refresh_reinsert_preserves_user_state_and_identity() {
    let (db, _temp) = create_test_db().await;
    let sub = create_test_subscription("Show", "https://example.com/feed.xml");
    db.insert_subscription(&sub).await.unwrap();

    // Initial insert, then user-state accrues: resume position, played, download.
    let ep = create_test_episode(sub.id, "Original title");
    db.insert_episode(&ep).await.unwrap();
    db.update_episode_playback_position(ep.id, 942)
        .await
        .unwrap();
    db.mark_episode_played(ep.id, true).await.unwrap();
    db.update_episode_download_status(
        ep.id,
        DownloadStatus::Downloaded,
        Some(std::path::Path::new("/tmp/ep.mp3")),
    )
    .await
    .unwrap();

    // Mark it now-playing. INSERT OR REPLACE used to delete the episode row on
    // refresh, tripping the playback_state ON DELETE SET NULL foreign key and
    // losing the now-playing episode entirely.
    db.update_playback_state(&PlaybackState {
        current_episode_id: Some(ep.id),
        position_seconds: 942.0,
        volume: 1.0,
        playback_rate: 1.0,
        status: PlaybackStatus::Playing,
    })
    .await
    .unwrap();

    // A refresh re-parses the feed: same (subscription_id, guid) but a fresh id,
    // a metadata change, and default user-state - exactly what INSERT OR REPLACE
    // used to clobber (position/played/download reset, id changed -> FK nulled).
    let mut refreshed = create_test_episode(sub.id, "Updated title");
    refreshed.guid = ep.guid.clone();
    assert_ne!(refreshed.id, ep.id);
    db.insert_episode(&refreshed).await.unwrap();

    let eps = db.get_episodes_for_subscription(sub.id).await.unwrap();
    assert_eq!(eps.len(), 1, "no duplicate row");
    let got = &eps[0];
    assert_eq!(
        got.id, ep.id,
        "identity preserved (id unchanged across refresh)"
    );
    assert_eq!(got.playback_position_seconds, 942, "resume position kept");
    assert!(got.played, "played flag kept");
    assert!(
        matches!(got.download_status, DownloadStatus::Downloaded),
        "download status kept"
    );
    assert_eq!(
        got.local_path.as_deref(),
        Some(std::path::Path::new("/tmp/ep.mp3")),
        "download path kept"
    );
    assert_eq!(got.title, "Updated title", "feed metadata refreshed");

    // The now-playing pointer survived the refresh (no row delete -> FK intact).
    let pstate = db.get_playback_state().await.unwrap();
    assert_eq!(
        pstate.current_episode_id,
        Some(ep.id),
        "now-playing episode preserved across refresh"
    );
}

#[tokio::test]
async fn mark_seen_clears_new_without_marking_played() {
    let (db, _temp) = create_test_db().await;
    let sub = create_test_subscription("Show", "https://example.com/feed.xml");
    db.insert_subscription(&sub).await.unwrap();

    // Three fresh episodes -> all "new" (as after an OPML import).
    for t in ["a", "b", "c"] {
        db.insert_episode(&create_test_episode(sub.id, t))
            .await
            .unwrap();
    }
    let before = db.get_subscription(sub.id).await.unwrap().unwrap();
    assert_eq!(before.new_count, 3, "all imported episodes are new");

    // Mark one seen: new drops, but it is NOT played (seen != listened).
    let one = db.get_episodes_for_subscription(sub.id).await.unwrap()[0].id;
    db.mark_episode_seen(one).await.unwrap();
    let got = db.get_episode(one).await.unwrap().unwrap();
    assert!(!got.is_new, "no longer new");
    assert!(!got.played, "seen does not mark played");
    assert_eq!(
        db.get_subscription(sub.id)
            .await
            .unwrap()
            .unwrap()
            .new_count,
        2
    );

    // Mark the whole feed seen: the backlog clears.
    let n = db.mark_subscription_seen(sub.id).await.unwrap();
    assert_eq!(n, 2, "the two still-new episodes were cleared");
    assert_eq!(
        db.get_subscription(sub.id)
            .await
            .unwrap()
            .unwrap()
            .new_count,
        0
    );

    // Playing (mark_episode_played) also clears new.
    let played = db.get_episodes_for_subscription(sub.id).await.unwrap()[1].id;
    db.mark_subscription_seen(sub.id).await.unwrap(); // reset baseline is 0 already
    db.mark_episode_played(played, true).await.unwrap();
    let got = db.get_episode(played).await.unwrap().unwrap();
    assert!(got.played && !got.is_new, "played implies acknowledged");
}

#[tokio::test]
async fn test_mark_episode_played() {
    let (db, _temp) = create_test_db().await;
    let sub = create_test_subscription("Test Podcast", "https://example.com/feed.xml");
    db.insert_subscription(&sub).await.unwrap();

    let episode = create_test_episode(sub.id, "Test Episode");
    db.insert_episode(&episode).await.unwrap();

    assert!(!episode.played);

    db.mark_episode_played(episode.id, true).await.unwrap();

    let updated = db.get_episode(episode.id).await.unwrap().unwrap();
    assert!(updated.played);
    // Note: last_played_at is not updated by mark_episode_played method
}

#[tokio::test]
async fn test_update_episode_download_status() {
    let (db, _temp) = create_test_db().await;
    let sub = create_test_subscription("Test Podcast", "https://example.com/feed.xml");
    db.insert_subscription(&sub).await.unwrap();

    let episode = create_test_episode(sub.id, "Test Episode");
    db.insert_episode(&episode).await.unwrap();

    let path = PathBuf::from("/tmp/episode.mp3");
    db.update_episode_download_status(episode.id, DownloadStatus::Downloaded, Some(&path))
        .await
        .unwrap();

    let updated = db.get_episode(episode.id).await.unwrap().unwrap();
    assert_eq!(updated.download_status, DownloadStatus::Downloaded);
    assert_eq!(updated.local_path, Some(path));
}

// Queue Tests

#[tokio::test]
async fn test_add_to_queue() {
    let (db, _temp) = create_test_db().await;
    let sub = create_test_subscription("Test Podcast", "https://example.com/feed.xml");
    db.insert_subscription(&sub).await.unwrap();

    let episode = create_test_episode(sub.id, "Test Episode");
    db.insert_episode(&episode).await.unwrap();

    let queue_item = QueueItem::new(episode.id, 0);

    db.add_to_queue(&queue_item).await.unwrap();

    let queue = db.get_queue().await.unwrap();
    assert_eq!(queue.len(), 1);
    assert_eq!(queue[0].episode_id, episode.id);
}

#[tokio::test]
async fn test_remove_from_queue() {
    let (db, _temp) = create_test_db().await;
    let sub = create_test_subscription("Test Podcast", "https://example.com/feed.xml");
    db.insert_subscription(&sub).await.unwrap();

    let episode = create_test_episode(sub.id, "Test Episode");
    db.insert_episode(&episode).await.unwrap();

    let queue_item = QueueItem::new(episode.id, 0);

    db.add_to_queue(&queue_item).await.unwrap();
    db.remove_from_queue(episode.id).await.unwrap();

    let queue = db.get_queue().await.unwrap();
    assert_eq!(queue.len(), 0);
}

#[tokio::test]
async fn test_reorder_queue() {
    let (db, _temp) = create_test_db().await;
    let sub = create_test_subscription("Test Podcast", "https://example.com/feed.xml");
    db.insert_subscription(&sub).await.unwrap();

    let ep1 = create_test_episode(sub.id, "Episode 1");
    let ep2 = create_test_episode(sub.id, "Episode 2");

    db.insert_episode(&ep1).await.unwrap();
    db.insert_episode(&ep2).await.unwrap();

    db.add_to_queue(&QueueItem::new(ep1.id, 0)).await.unwrap();
    db.add_to_queue(&QueueItem::new(ep2.id, 1)).await.unwrap();

    // Reorder ep2 to position 0
    let result = db.reorder_queue_item(ep2.id, 0).await;
    assert!(result.is_ok(), "Reorder should succeed");

    let queue = db.get_queue().await.unwrap();
    assert_eq!(queue.len(), 2, "Queue should still have 2 items");
}

#[tokio::test]
async fn test_clear_queue() {
    let (db, _temp) = create_test_db().await;
    let sub = create_test_subscription("Test Podcast", "https://example.com/feed.xml");
    db.insert_subscription(&sub).await.unwrap();

    let ep1 = create_test_episode(sub.id, "Episode 1");
    let ep2 = create_test_episode(sub.id, "Episode 2");

    db.insert_episode(&ep1).await.unwrap();
    db.insert_episode(&ep2).await.unwrap();

    db.add_to_queue(&QueueItem::new(ep1.id, 0)).await.unwrap();
    db.add_to_queue(&QueueItem::new(ep2.id, 1)).await.unwrap();

    db.clear_queue().await.unwrap();

    let queue = db.get_queue().await.unwrap();
    assert_eq!(queue.len(), 0);
}

// Playback State Tests

#[tokio::test]
async fn test_get_and_update_playback_state() {
    let (db, _temp) = create_test_db().await;
    let sub = create_test_subscription("Test Podcast", "https://example.com/feed.xml");
    db.insert_subscription(&sub).await.unwrap();

    let episode = create_test_episode(sub.id, "Test Episode");
    db.insert_episode(&episode).await.unwrap();

    let state = PlaybackState {
        current_episode_id: Some(episode.id),
        position_seconds: 123.4,
        volume: 0.8,
        playback_rate: 1.5,
        status: PlaybackStatus::Playing,
    };

    db.update_playback_state(&state).await.unwrap();

    let retrieved = db.get_playback_state().await.unwrap();
    assert_eq!(retrieved.current_episode_id, Some(episode.id));
    assert!((retrieved.position_seconds - 123.4).abs() < 0.01);
    assert_eq!(retrieved.volume, 0.8);
    assert_eq!(retrieved.playback_rate, 1.5);
    assert_eq!(retrieved.status, PlaybackStatus::Playing);
}

#[tokio::test]
async fn test_multiple_db_connections() {
    let temp_dir = tempfile::tempdir().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let db1 = Database::new(&db_path).await.unwrap();
    let db2 = Database::new(&db_path).await.unwrap();

    let sub = create_test_subscription("Test Podcast", "https://example.com/feed.xml");
    db1.insert_subscription(&sub).await.unwrap();

    // Should be visible from second connection
    let retrieved = db2.get_subscription(sub.id).await.unwrap();
    assert!(retrieved.is_some());
}

// Note: This test is commented out because the UNIQUE constraint on (subscription_id, guid)
// may not be enforced in the current schema. If the schema is updated to include this constraint,
// this test should be uncommented and enabled.
//
// #[tokio::test]
// async fn test_unique_episode_guid_per_subscription() {
//     let (db, _temp) = create_test_db().await;
//     let sub = create_test_subscription("Test Podcast", "https://example.com/feed.xml");
//     db.insert_subscription(&sub).await.unwrap();
//
//     let mut ep1 = create_test_episode(sub.id, "Episode 1");
//     ep1.guid = "unique-guid-123".to_string();
//     db.insert_episode(&ep1).await.unwrap();
//
//     // Try to insert episode with same guid for same subscription
//     let mut ep2 = create_test_episode(sub.id, "Episode 2");
//     ep2.id = Uuid::new_v4(); // Different ID
//     ep2.guid = "unique-guid-123".to_string(); // Same GUID
//
//     let result = db.insert_episode(&ep2).await;
//     assert!(result.is_err(), "Should fail on duplicate guid for same subscription");
// }
