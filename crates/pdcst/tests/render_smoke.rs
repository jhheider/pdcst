//! Render smoke tests: every view must draw into a TestBackend without
//! panicking, with realistic data (a subscription, episodes with each listen
//! state, a non-empty queue, a now-playing episode). CI has no terminal, so this
//! is the only automated guard on the rendering layer - exactly where the app's
//! showstoppers historically lived.

mod common;

use common::{build_state, sample_episode};
use pdcst::app::state::{AppState, View};
use pdcst::models::Subscription;
use pdcst::ui::Ui;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

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
}

#[tokio::test]
async fn renders_search_view() {
    let (mut state, _dir) = build_state().await;
    let ui = Ui::new();

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
