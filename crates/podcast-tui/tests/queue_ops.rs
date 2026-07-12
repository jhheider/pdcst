//! Queue operability behavior: skip semantics and remove-from-queue.

mod common;

use common::{build_state, sample_episode};
use podcast_tui::app::state::View;
use podcast_tui::models::Subscription;

/// Skipping must drop the currently-playing episode (the queue head) and advance
/// to the next one, rather than replaying the same episode.
#[tokio::test]
async fn skip_removes_current_and_advances() {
    let (mut state, _dir) = build_state().await;

    let sub = Subscription::new("Show".to_string(), "https://example.com/f.xml".to_string());
    state.db.insert_subscription(&sub).await.unwrap();
    let a = sample_episode(sub.id, "A", false, 0);
    let b = sample_episode(sub.id, "B", false, 0);
    state.db.insert_episode(&a).await.unwrap();
    state.db.insert_episode(&b).await.unwrap();
    state.queue_manager.add_episode(a.id).await.unwrap();
    state.queue_manager.add_episode(b.id).await.unwrap();

    // A is the current (head) episode.
    state.current_episode = Some(a.clone());

    state.play_next_in_queue().await.unwrap();

    let queue = state.db.get_queue().await.unwrap();
    assert!(
        !queue.iter().any(|item| item.episode_id == a.id),
        "the skipped (current) episode is removed from the queue"
    );
    assert!(
        queue.iter().any(|item| item.episode_id == b.id),
        "the next episode remains queued"
    );
}

/// `x` in the Queue view removes the selected item.
#[tokio::test]
async fn remove_selected_drops_the_queue_item() {
    let (mut state, _dir) = build_state().await;

    let sub = Subscription::new("Show".to_string(), "https://example.com/f.xml".to_string());
    state.db.insert_subscription(&sub).await.unwrap();
    let a = sample_episode(sub.id, "A", false, 0);
    let b = sample_episode(sub.id, "B", false, 0);
    state.db.insert_episode(&a).await.unwrap();
    state.db.insert_episode(&b).await.unwrap();
    state.queue_manager.add_episode(a.id).await.unwrap();
    state.queue_manager.add_episode(b.id).await.unwrap();
    state.load_queue().await.unwrap();

    // Select the second item in the Queue view and remove it.
    state.set_view(View::Queue);
    state.selected_index = 1;
    let removed_id = state.queue_items[1].id;
    state.remove_selected_from_queue().await.unwrap();

    let queue = state.db.get_queue().await.unwrap();
    assert!(!queue.iter().any(|item| item.episode_id == removed_id));
    assert_eq!(queue.len(), 1);
}
