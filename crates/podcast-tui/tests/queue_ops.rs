//! Queue operability behavior: skip semantics and remove-from-queue.

mod common;

use common::{build_state, sample_episode};
use podcast_tui::app::state::View;
use podcast_tui::models::{PlaybackStatus, Subscription};
use podcast_tui::storage::db::PlaybackState;

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

// --- Phase C: auto-enqueue (the publish-time hook) ---

async fn two_subs(state: &podcast_tui::app::state::AppState) -> (Subscription, Subscription) {
    let a = Subscription::new("A".to_string(), "https://example.com/a.xml".to_string());
    let b = Subscription::new("B".to_string(), "https://example.com/b.xml".to_string());
    state.db.insert_subscription(&a).await.unwrap();
    state.db.insert_subscription(&b).await.unwrap();
    (a, b)
}

/// Push appends to the tail; unshift prepends to the front.
#[tokio::test]
async fn auto_enqueue_push_and_unshift() {
    let (state, _dir) = build_state().await;
    let (sub_a, sub_b) = two_subs(&state).await;
    let a = sample_episode(sub_a.id, "A1", false, 0);
    let b = sample_episode(sub_b.id, "B1", false, 0);
    state.db.insert_episode(&a).await.unwrap();
    state.db.insert_episode(&b).await.unwrap();

    assert!(
        state
            .queue_manager
            .auto_enqueue(&a, false, 20, false)
            .await
            .unwrap(),
        "push enqueues"
    );
    assert!(
        state
            .queue_manager
            .auto_enqueue(&b, true, 20, false)
            .await
            .unwrap(),
        "unshift enqueues"
    );

    let q = state.db.get_queue().await.unwrap();
    assert_eq!(q.len(), 2);
    assert_eq!(q[0].episode_id, b.id, "unshift put B on top");
    assert_eq!(q[1].episode_id, a.id);
}

/// A full queue (at max_depth) does not grow.
#[tokio::test]
async fn auto_enqueue_respects_max_depth() {
    let (state, _dir) = build_state().await;
    let (sub, _) = two_subs(&state).await;
    let a = sample_episode(sub.id, "A", false, 0);
    let b = sample_episode(sub.id, "B", false, 0);
    state.db.insert_episode(&a).await.unwrap();
    state.db.insert_episode(&b).await.unwrap();

    assert!(
        state
            .queue_manager
            .auto_enqueue(&a, false, 1, false)
            .await
            .unwrap()
    );
    assert!(
        !state
            .queue_manager
            .auto_enqueue(&b, false, 1, false)
            .await
            .unwrap(),
        "at the cap, the second episode is skipped"
    );
    assert_eq!(state.db.get_queue().await.unwrap().len(), 1);
}

/// The currently-playing head is never displaced: an unshift lands after it.
#[tokio::test]
async fn auto_enqueue_never_displaces_current() {
    let (state, _dir) = build_state().await;
    let (sub_a, sub_b) = two_subs(&state).await;
    let a = sample_episode(sub_a.id, "A1", false, 0);
    let b = sample_episode(sub_b.id, "B1", false, 0);
    state.db.insert_episode(&a).await.unwrap();
    state.db.insert_episode(&b).await.unwrap();

    state
        .queue_manager
        .auto_enqueue(&a, false, 20, false)
        .await
        .unwrap();
    // A is the loaded/playing episode (the protected head).
    state
        .db
        .update_playback_state(&PlaybackState {
            current_episode_id: Some(a.id),
            position_seconds: 0.0,
            playback_rate: 1.0,
            volume: 1.0,
            status: PlaybackStatus::Playing,
        })
        .await
        .unwrap();

    state
        .queue_manager
        .auto_enqueue(&b, true, 20, false)
        .await
        .unwrap();

    let q = state.db.get_queue().await.unwrap();
    assert_eq!(q[0].episode_id, a.id, "protected head stays at the top");
    assert_eq!(
        q[1].episode_id, b.id,
        "unshift landed after the current item"
    );
}

/// Interleave keeps two episodes of the same podcast from sitting adjacent.
#[tokio::test]
async fn auto_enqueue_interleave_avoids_adjacency() {
    let (state, _dir) = build_state().await;
    let (sub_a, sub_b) = two_subs(&state).await;
    let a1 = sample_episode(sub_a.id, "A1", false, 0);
    let a2 = sample_episode(sub_a.id, "A2", false, 0);
    let b1 = sample_episode(sub_b.id, "B1", false, 0);
    for ep in [&b1, &a1, &a2] {
        state.db.insert_episode(ep).await.unwrap();
    }

    // Queue becomes [b1, a1]; pushing a2 with interleave must not sit next to a1.
    state
        .queue_manager
        .auto_enqueue(&b1, false, 20, true)
        .await
        .unwrap();
    state
        .queue_manager
        .auto_enqueue(&a1, false, 20, true)
        .await
        .unwrap();
    state
        .queue_manager
        .auto_enqueue(&a2, false, 20, true)
        .await
        .unwrap();

    let entries = state.queue_manager.get_queue_with_episodes().await.unwrap();
    let subs: Vec<_> = entries.iter().map(|(_, ep)| ep.subscription_id).collect();
    assert_eq!(subs.len(), 3);
    for pair in subs.windows(2) {
        assert_ne!(pair[0], pair[1], "no two adjacent episodes share a podcast");
    }
}
