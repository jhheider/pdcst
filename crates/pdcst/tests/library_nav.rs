//! The two-pane library: independent per-pane cursors, live preview of the
//! highlighted feed, and the joined per-subscription stats.

mod common;

use common::{build_state, sample_episode};
use pdcst::app::state::{Modal, PendingAction, SearchFocus, View};
use pdcst::feed::SearchResult;
use pdcst::models::Subscription;

/// An oldest-first feed reverses the (newest-first) query so it reads and queues
/// top-to-bottom in publication order.
#[tokio::test]
async fn oldest_first_reverses_episode_order() {
    let (mut state, _dir) = build_state().await;
    let mut sub = Subscription::new("Serial".into(), "https://example.com/s.xml".into());
    sub.queue_oldest_first = true;
    state.db.insert_subscription(&sub).await.unwrap();
    for (i, t) in ["ep1", "ep2", "ep3"].iter().enumerate() {
        let mut ep = sample_episode(sub.id, t, false, 0);
        ep.published_at = chrono::Utc::now() - chrono::Duration::days(3 - i as i64);
        state.db.insert_episode(&ep).await.unwrap();
    }
    state.load_subscriptions().await.unwrap();
    state.load_episodes_for_subscription(sub.id).await.unwrap();
    assert_eq!(
        state.episodes.first().unwrap().title,
        "ep1",
        "oldest on top"
    );
    assert_eq!(
        state.episodes.last().unwrap().title,
        "ep3",
        "newest at bottom"
    );

    // Flipping back to newest-first restores descending order.
    state
        .db
        .update_subscription_queue_order(sub.id, false)
        .await
        .unwrap();
    state.load_subscriptions().await.unwrap();
    state.load_episodes_for_subscription(sub.id).await.unwrap();
    assert_eq!(
        state.episodes.first().unwrap().title,
        "ep3",
        "newest on top"
    );
}

/// The left (Subscriptions) and right (Episodes) panes keep separate cursors, so
/// drilling into a feed and backing out returns to the same subscription.
#[tokio::test]
async fn panes_keep_independent_cursors() {
    let (mut state, _dir) = build_state().await;

    // Three feeds (A, B, C by title order); C has several episodes to move over.
    for title in ["A", "B", "C"] {
        let sub = Subscription::new(title.into(), format!("https://example.com/{title}.xml"));
        state.db.insert_subscription(&sub).await.unwrap();
        if title == "C" {
            for ep in ["c1", "c2", "c3"] {
                state
                    .db
                    .insert_episode(&sample_episode(sub.id, ep, false, 0))
                    .await
                    .unwrap();
            }
        }
    }
    state.load_subscriptions().await.unwrap();
    state.set_view(View::Subscriptions);

    // Move the left cursor to C and preview it into the right pane.
    state.next_item(); // -> B
    state.next_item(); // -> C
    state.preview_selected_subscription().await.unwrap();
    assert_eq!(state.subscription_index, 2);
    assert_eq!(state.episode_index, 0);
    assert_eq!(state.episodes.len(), 3, "right pane previews C's episodes");

    // Focus the right pane and move within it: the left cursor is untouched.
    state.focus_episodes();
    assert_eq!(state.current_view, View::Episodes);
    state.next_item(); // episode 0 -> 1
    assert_eq!(state.episode_index, 1);
    assert_eq!(
        state.subscription_index, 2,
        "left cursor preserved across focus"
    );

    // Back out: the subscription cursor is exactly where it was.
    state.focus_subscriptions();
    assert_eq!(state.current_view, View::Subscriptions);
    assert_eq!(state.subscription_index, 2);
}

/// Moving the left cursor swaps the previewed feed's episodes into the right pane
/// and resets the episode cursor to the top.
#[tokio::test]
async fn preview_follows_the_left_cursor() {
    let (mut state, _dir) = build_state().await;

    let a = Subscription::new("A".into(), "https://example.com/a.xml".into());
    let b = Subscription::new("B".into(), "https://example.com/b.xml".into());
    state.db.insert_subscription(&a).await.unwrap();
    state.db.insert_subscription(&b).await.unwrap();
    state
        .db
        .insert_episode(&sample_episode(a.id, "a1", false, 0))
        .await
        .unwrap();
    state
        .db
        .insert_episode(&sample_episode(b.id, "b1", false, 0))
        .await
        .unwrap();
    state
        .db
        .insert_episode(&sample_episode(b.id, "b2", false, 0))
        .await
        .unwrap();
    state.load_subscriptions().await.unwrap();
    state.set_view(View::Subscriptions);

    state.preview_selected_subscription().await.unwrap();
    assert_eq!(state.current_subscription.as_ref().unwrap().title, "A");
    assert_eq!(state.episodes.len(), 1);

    state.next_item(); // -> B
    state.preview_selected_subscription().await.unwrap();
    assert_eq!(state.current_subscription.as_ref().unwrap().title, "B");
    assert_eq!(state.episodes.len(), 2, "right pane now shows B's episodes");
    assert_eq!(
        state.episode_index, 0,
        "episode cursor resets on a feed change"
    );
}

/// `get_all_subscriptions` carries the joined counts and newest-episode date.
#[tokio::test]
async fn subscription_row_has_counts() {
    let (mut state, _dir) = build_state().await;

    let sub = Subscription::new("Show".into(), "https://example.com/s.xml".into());
    state.db.insert_subscription(&sub).await.unwrap();
    // Two unplayed, one played -> total 3, unplayed 2.
    state
        .db
        .insert_episode(&sample_episode(sub.id, "e1", false, 0))
        .await
        .unwrap();
    state
        .db
        .insert_episode(&sample_episode(sub.id, "e2", false, 300))
        .await
        .unwrap();
    state
        .db
        .insert_episode(&sample_episode(sub.id, "e3", true, 0))
        .await
        .unwrap();

    state.load_subscriptions().await.unwrap();
    let row = &state.subscriptions[0];
    assert_eq!(row.episode_count, 3);
    assert_eq!(row.new_count, 2);
    assert!(row.latest_episode_at.is_some());

    // A feed with no episodes reports zeros, not NULLs.
    let empty = Subscription::new("Empty".into(), "https://example.com/e.xml".into());
    state.db.insert_subscription(&empty).await.unwrap();
    state.load_subscriptions().await.unwrap();
    let empty_row = state
        .subscriptions
        .iter()
        .find(|s| s.title == "Empty")
        .unwrap();
    assert_eq!(empty_row.episode_count, 0);
    assert_eq!(empty_row.new_count, 0);
    assert!(empty_row.latest_episode_at.is_none());
}

/// A recorded refresh error round-trips onto the row and clears on success.
#[tokio::test]
async fn subscription_error_round_trips() {
    let (mut state, _dir) = build_state().await;
    let sub = Subscription::new("Show".into(), "https://example.com/s.xml".into());
    state.db.insert_subscription(&sub).await.unwrap();

    state
        .db
        .set_subscription_error(sub.id, Some("Failed to fetch feed"))
        .await
        .unwrap();
    state.load_subscriptions().await.unwrap();
    assert_eq!(
        state.subscriptions[0].last_error.as_deref(),
        Some("Failed to fetch feed")
    );

    state.db.set_subscription_error(sub.id, None).await.unwrap();
    state.load_subscriptions().await.unwrap();
    assert!(state.subscriptions[0].last_error.is_none());
}

/// Confirming a feed re-point swaps the URL, clears the error, and dismisses the
/// modal - the recovery path for a moved feed.
#[tokio::test]
async fn confirming_repoint_switches_the_feed_url() {
    let (mut state, _dir) = build_state().await;
    let sub = Subscription::new("Show".into(), "https://old.example.com/dead.xml".into());
    state.db.insert_subscription(&sub).await.unwrap();
    state
        .db
        .set_subscription_error(sub.id, Some("Failed to fetch feed"))
        .await
        .unwrap();
    state.load_subscriptions().await.unwrap();

    // Simulate the confirm dialog a FeedFixFound event would raise.
    state.pending_action = Some(PendingAction::RepointFeed {
        subscription_id: sub.id,
        new_url: "https://new.example.com/live.xml".into(),
    });
    state.modal = Modal::Confirm {
        message: "switch?".into(),
        action: "repoint-feed".into(),
    };

    state.apply_pending_action().await.unwrap();

    let updated = state.db.get_subscription(sub.id).await.unwrap().unwrap();
    assert_eq!(updated.rss_url, "https://new.example.com/live.xml");
    assert!(updated.last_error.is_none(), "error cleared on re-point");
    assert_eq!(state.modal, Modal::None, "modal dismissed");
    assert!(state.pending_action.is_none(), "pending action consumed");
}

/// In the feed-recovery picker, choosing a result prompts to re-point the target
/// feed (not subscribe a new one).
#[tokio::test]
async fn picking_in_fix_mode_prompts_repoint_not_subscribe() {
    let (mut state, _dir) = build_state().await;
    let sub = Subscription::new("Show".into(), "https://old.example.com/dead.xml".into());
    state.db.insert_subscription(&sub).await.unwrap();
    state.load_subscriptions().await.unwrap();

    // Enter the picker: Search view, fix target set, one candidate, result focus.
    state.set_view(View::Search);
    state.feed_fix_target = Some(sub.id);
    state.search_results = vec![SearchResult {
        title: "Show (relocated)".into(),
        artist: "Host".into(),
        feed_url: "https://new.example.com/live.xml".into(),
        artwork_url: None,
        description: None,
        genre: Some("News".into()),
        track_count: Some(120),
    }];
    state.search_focus = SearchFocus::Results;
    state.selected_index = 0;

    state.select_item().await.unwrap();

    // It prompts a re-point rather than creating a subscription.
    assert!(matches!(
        state.pending_action,
        Some(PendingAction::RepointFeed { subscription_id, ref new_url })
            if subscription_id == sub.id && new_url == "https://new.example.com/live.xml"
    ));
    assert!(matches!(state.modal, Modal::Confirm { .. }));
    assert_eq!(
        state.db.get_all_subscriptions().await.unwrap().len(),
        1,
        "no new subscription was added"
    );
}

/// Cancelling a confirm dialog abandons the pending action and touches nothing.
#[tokio::test]
async fn cancelling_confirm_abandons_the_action() {
    let (mut state, _dir) = build_state().await;
    let sub = Subscription::new("Show".into(), "https://old.example.com/feed.xml".into());
    state.db.insert_subscription(&sub).await.unwrap();

    state.pending_action = Some(PendingAction::RepointFeed {
        subscription_id: sub.id,
        new_url: "https://new.example.com/live.xml".into(),
    });
    state.modal = Modal::Confirm {
        message: "switch?".into(),
        action: "repoint-feed".into(),
    };

    state.close_modal();

    assert!(state.pending_action.is_none());
    let unchanged = state.db.get_subscription(sub.id).await.unwrap().unwrap();
    assert_eq!(unchanged.rss_url, "https://old.example.com/feed.xml");
}
