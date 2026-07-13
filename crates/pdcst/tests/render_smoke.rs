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

/// Draw a view and return the full rendered buffer as one string, so tests can
/// assert on what actually reaches the terminal cells.
fn render_to_string(ui: &Ui, state: &mut AppState) -> String {
    let backend = TestBackend::new(80, 24);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal
        .draw(|f| ui.render(f, state))
        .expect("render must not panic");
    terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| cell.symbol())
        .collect()
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
    // renders its markers. Set every pane's cursor so whichever view is drawn is
    // scrolled (the library panes and the single-list views track separately).
    for view in [View::Subscriptions, View::Episodes, View::Queue] {
        state.set_view(view);
        state.subscription_index = 2;
        state.episode_index = 2;
        state.selected_index = 2;
        draw(&ui, &mut state);
    }
}

#[tokio::test]
async fn episode_card_snippet_is_tag_and_joiner_free() {
    // The description snippet must strip HTML markup and must not carry a raw ZWJ
    // (feed text is normalized at ingest; this guards the render path too). A wide
    // multibyte title must also draw without panicking or leaking markup.
    let (mut state, _dir) = build_state().await;
    let ui = Ui::new();

    let sub = Subscription::new(
        "Wide \u{65E5}\u{672C}\u{8A9E} Show".to_string(),
        "https://example.com/feed.xml".to_string(),
    );
    let mut ep = sample_episode(sub.id, "Curly \u{2019}quotes\u{2019} here", false, 0);
    ep.description = Some("<p>Clean <b>preview</b> text \u{2014} no markup</p>".to_string());

    state.current_subscription = Some(sub.clone());
    state.subscriptions = vec![sub];
    state.episodes = vec![ep];
    state.set_view(View::Episodes);

    let rendered = render_to_string(&ui, &mut state);
    assert!(
        rendered.contains("Clean preview text"),
        "snippet should be tag-stripped: {rendered:?}"
    );
    assert!(
        !rendered.contains("<p>") && !rendered.contains("<b>"),
        "no raw HTML tags should reach the buffer"
    );
    assert!(
        !rendered.contains('\u{200D}'),
        "no zero-width joiner should reach the buffer"
    );
}

#[tokio::test]
async fn stream_drop_notice_shows_in_the_playback_bar() {
    // The sticky stream-drop notice must take over the status line (non-blocking,
    // no modal), replacing the normal now-playing text while it is set.
    let (mut state, _dir) = build_state().await;
    let ui = Ui::new();

    let sub = Subscription::new("Show".to_string(), "https://example.com/f.xml".to_string());
    let ep = sample_episode(sub.id, "Episode", false, 600);
    state.current_episode = Some(ep);
    state.playback_notice = Some("Reconnecting (1/2)...".to_string());
    state.set_view(View::Queue);

    let rendered = render_to_string(&ui, &mut state);
    assert!(
        rendered.contains("Reconnecting (1/2)"),
        "the sticky notice should reach the playback bar: {rendered:?}"
    );
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
