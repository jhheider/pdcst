use crate::app::events::{EventBus, StateEvent};
use crate::audio::{AudioPlayer, AudioStreamer};
use crate::download::DownloadManager;
use crate::feed::{FeedRefresher, PodcastSearch, SearchResult};
use crate::models::Config;
use crate::models::{Episode, PlaybackStatus, Subscription};
use crate::queue::QueueManager;
use crate::storage::Database;
use crate::storage::db::PlaybackState;
use anyhow::Result;
use ratatui::widgets::ListState;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long a transient status message stays on screen before it auto-clears.
const STATUS_TTL: Duration = Duration::from_secs(2);

/// A one-line status message shown at the bottom of the screen.
///
/// Transient messages carry an expiry so they clear themselves at render time
/// (replacing the old pattern of blocking the event loop with `sleep(2s)`).
/// Persistent messages (`expires_at == None`) stay until explicitly cleared -
/// used for "Loading..." while an episode fetch is in flight.
pub struct StatusMessage {
    pub text: String,
    expires_at: Option<Instant>,
}

impl StatusMessage {
    /// Whether this message's TTL has elapsed as of `now`. Persistent messages
    /// (no expiry) never expire.
    fn is_expired(&self, now: Instant) -> bool {
        self.expires_at.is_some_and(|expires_at| now >= expires_at)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum View {
    Subscriptions,
    Episodes,
    Queue,
    Search,
    Settings,
}

/// Where keystrokes go in the Search view: into the query box, or into the
/// results list (where Enter subscribes).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SearchFocus {
    Input,
    Results,
}

/// The four top-level views the number keys and Tab cycle through. Episodes is a
/// drill-down of Subscriptions (reached with Enter, left with Esc), not a tab of
/// its own, so number/Tab navigation and the help screen all share one model.
const TOP_VIEWS: [View; 4] = [
    View::Subscriptions,
    View::Queue,
    View::Search,
    View::Settings,
];

/// The next top-level view when cycling with Tab. Episodes cycles as if it were
/// Subscriptions (its parent), so Tab out of a drill-down is predictable.
fn next_top_view(view: View) -> View {
    let current = if view == View::Episodes {
        View::Subscriptions
    } else {
        view
    };
    let idx = TOP_VIEWS.iter().position(|&v| v == current).unwrap_or(0);
    TOP_VIEWS[(idx + 1) % TOP_VIEWS.len()]
}

/// The previous top-level view (Shift-Tab), with Episodes treated as its parent.
fn prev_top_view(view: View) -> View {
    let current = if view == View::Episodes {
        View::Subscriptions
    } else {
        view
    };
    let idx = TOP_VIEWS.iter().position(|&v| v == current).unwrap_or(0);
    TOP_VIEWS[(idx + TOP_VIEWS.len() - 1) % TOP_VIEWS.len()]
}

#[derive(Debug, Clone, PartialEq)]
pub enum Modal {
    None,
    Help,
    Error(String),
    Confirm { message: String, action: String },
}

/// An action awaiting the user's confirmation in a [`Modal::Confirm`]. Carried
/// separately from the modal's display text so the Enter handler has the typed
/// payload it needs to execute (the modal itself only renders a message).
#[derive(Debug, Clone, PartialEq)]
pub enum PendingAction {
    /// Re-point a failing/moved subscription at a feed URL found by title search.
    RepointFeed {
        subscription_id: uuid::Uuid,
        new_url: String,
    },
}

pub struct AppState {
    pub config: Config,
    pub db: Arc<Database>,
    pub audio_player: Arc<AudioPlayer>,
    pub audio_streamer: Arc<AudioStreamer>,
    pub queue_manager: Arc<QueueManager>,
    pub download_manager: Arc<DownloadManager>,
    pub feed_refresher: Arc<FeedRefresher>,
    pub podcast_search: Arc<PodcastSearch>,
    pub event_bus: Arc<EventBus>,

    // UI state
    pub current_view: View,
    /// Cursor for the Queue and Search lists (the single-list views). The
    /// two-pane library keeps its own per-pane cursors below, because both panes
    /// are on screen at once and each scrolls independently.
    pub selected_index: usize,
    /// Scroll/selection state for the current single-list view (Queue/Search).
    /// Reused across frames so ratatui keeps the selected row on screen; reset
    /// when the view changes.
    pub list_state: ListState,
    /// Left-pane (Subscriptions) cursor in the two-pane library. Persists while
    /// the right pane is focused, so drilling into a feed and backing out returns
    /// to the same subscription.
    pub subscription_index: usize,
    /// Right-pane (Episodes) cursor in the two-pane library.
    pub episode_index: usize,
    pub subscription_list_state: ListState,
    pub episode_list_state: ListState,
    pub modal: Modal,
    /// The action a `Modal::Confirm` will run on Enter (cleared on Esc/close).
    pub pending_action: Option<PendingAction>,
    pub search_input: String,
    pub search_cursor: usize,
    /// Whether keystrokes go to the query box or the results list.
    pub search_focus: SearchFocus,
    /// When set, the Search view is a feed-recovery *picker* for this
    /// subscription: choosing a result re-points that feed rather than adding a
    /// new subscription. `None` is an ordinary search.
    pub feed_fix_target: Option<uuid::Uuid>,
    pub status_message: Option<StatusMessage>,
    pub show_help: bool,
    /// Set by the quit key; the run loop checks it and exits.
    pub should_quit: bool,

    // Data
    pub subscriptions: Vec<Subscription>,
    pub episodes: Vec<Episode>,
    pub current_subscription: Option<Subscription>,
    pub search_results: Vec<SearchResult>,
    pub queue_items: Vec<Episode>, // Cached queue items

    // Playback state
    pub is_playing: bool,
    pub current_episode: Option<Episode>,
    pub playback_position: f64,
    pub playback_speed: f32,
    pub volume: f32,
    /// The volume to restore when unmuting (the level held before mute).
    pub pre_mute_volume: f32,
}

/// The shared services AppState is built from, grouped so AppState::new does not
/// take a dozen positional Arc args (and so the call site reads by name).
pub struct Services {
    pub audio_player: Arc<AudioPlayer>,
    pub audio_streamer: Arc<AudioStreamer>,
    pub queue_manager: Arc<QueueManager>,
    pub download_manager: Arc<DownloadManager>,
    pub feed_refresher: Arc<FeedRefresher>,
    pub podcast_search: Arc<PodcastSearch>,
}

mod library;
mod navigation;
mod playback;

pub(crate) use playback::load_and_play;

impl AppState {
    pub fn new(
        config: Config,
        db: Arc<Database>,
        services: Services,
        event_bus: Arc<EventBus>,
    ) -> Self {
        let Services {
            audio_player,
            audio_streamer,
            queue_manager,
            download_manager,
            feed_refresher,
            podcast_search,
        } = services;

        // Note: Auto-advance logic has been moved to App to use event-driven architecture
        // instead of the old completion channel. This eliminates the zombie task issue.

        // Clone audio_player to query initial state (sync before events start flowing)
        let audio_player_for_init = audio_player.clone();
        let event_bus_for_init = event_bus.clone();

        // Spawn task to initialize playback state from AudioPlayer
        // This prevents startup race condition where UI shows wrong initial values
        tokio::spawn(async move {
            // Small delay to ensure audio player is fully initialized
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;

            // Query current state and emit initial events to sync UI
            let volume = audio_player_for_init.get_volume().await;
            let speed = audio_player_for_init.get_speed().await;

            event_bus_for_init.publish(crate::app::events::StateEvent::VolumeChanged { volume });
            event_bus_for_init.publish(crate::app::events::StateEvent::SpeedChanged { speed });
        });

        Self {
            config,
            db,
            audio_player,
            audio_streamer,
            queue_manager,
            download_manager,
            feed_refresher,
            podcast_search,
            event_bus,
            current_view: View::Subscriptions,
            selected_index: 0,
            list_state: ListState::default(),
            subscription_index: 0,
            episode_index: 0,
            subscription_list_state: ListState::default(),
            episode_list_state: ListState::default(),
            modal: Modal::None,
            pending_action: None,
            search_input: String::new(),
            search_cursor: 0,
            search_focus: SearchFocus::Input,
            feed_fix_target: None,
            status_message: None,
            show_help: false,
            should_quit: false,
            subscriptions: Vec::new(),
            episodes: Vec::new(),
            current_subscription: None,
            search_results: Vec::new(),
            queue_items: Vec::new(),
            is_playing: false,
            current_episode: None,
            playback_position: 0.0,
            playback_speed: 1.0,
            volume: 1.0,
            pre_mute_volume: 1.0,
        }
    }

    /// Move focus to the results list after a search returns hits, so j/k browse
    /// and Enter subscribes. Called by the input handler when a query runs.
    pub fn focus_search_results(&mut self) {
        if !self.search_results.is_empty() {
            self.search_focus = SearchFocus::Results;
            self.selected_index = 0;
            self.list_state = ListState::default();
        }
    }

    /// Return focus to the query box (e.g. Esc from the results list).
    pub fn focus_search_input(&mut self) {
        self.search_focus = SearchFocus::Input;
    }

    // Search mode methods (placeholders for future UI state)

    pub fn enter_search_mode(&mut self) {
        // A fresh search is a normal search, not a feed-recovery picker.
        self.feed_fix_target = None;
        self.set_view(View::Search);
        tracing::debug!("Entered search mode");
    }

    pub fn exit_search_mode(&mut self) {
        if self.current_view == View::Search {
            self.set_view(View::Subscriptions);
        }
        // Leaving Search abandons any in-progress feed re-point.
        self.feed_fix_target = None;
        tracing::debug!("Exited search mode");
    }

    // Modal and notification methods

    pub fn show_help_modal(&mut self) {
        self.modal = Modal::Help;
    }

    pub fn show_error(&mut self, message: String) {
        self.modal = Modal::Error(message);
    }

    pub fn close_modal(&mut self) {
        self.modal = Modal::None;
        // Dismissing a confirm dialog abandons its pending action.
        self.pending_action = None;
    }

    /// Show a transient status message that auto-clears after [`STATUS_TTL`].
    /// Does not block the event loop; expiry happens at render time.
    pub fn set_status(&mut self, message: String) {
        self.status_message = Some(StatusMessage {
            text: message,
            expires_at: Some(Instant::now() + STATUS_TTL),
        });
    }

    /// Show a status message that stays until explicitly cleared (e.g.
    /// "Loading..." while an episode fetch is in flight, which can outlast the
    /// transient TTL).
    pub fn set_status_persistent(&mut self, message: String) {
        self.status_message = Some(StatusMessage {
            text: message,
            expires_at: None,
        });
    }

    pub fn clear_status(&mut self) {
        self.status_message = None;
    }

    /// The status text to display, or `None` if there is no live message.
    pub fn current_status(&self) -> Option<&str> {
        self.status_message.as_ref().map(|s| s.text.as_str())
    }

    /// Clear the status message if its TTL has elapsed. Returns `true` when a
    /// message was cleared, signalling the caller to redraw.
    pub fn expire_status(&mut self) -> bool {
        if let Some(status) = &self.status_message
            && status.is_expired(Instant::now())
        {
            self.status_message = None;
            return true;
        }
        false
    }

    // Search input methods

    pub fn append_search_char(&mut self, c: char) {
        self.search_input.insert(self.search_cursor, c);
        self.search_cursor += 1;
    }

    pub fn delete_search_char(&mut self) {
        if self.search_cursor > 0 && !self.search_input.is_empty() {
            self.search_cursor -= 1;
            self.search_input.remove(self.search_cursor);
        }
    }

    pub fn clear_search_input(&mut self) {
        self.search_input.clear();
        self.search_cursor = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_status_expires_after_ttl() {
        let now = Instant::now();
        let msg = StatusMessage {
            text: "Added to queue".to_string(),
            expires_at: Some(now + STATUS_TTL),
        };

        assert!(!msg.is_expired(now), "should be live immediately");
        assert!(
            !msg.is_expired(now + STATUS_TTL - Duration::from_millis(1)),
            "should still be live just before the TTL"
        );
        assert!(
            msg.is_expired(now + STATUS_TTL),
            "should be expired once the TTL elapses"
        );
    }

    #[test]
    fn persistent_status_never_expires() {
        let now = Instant::now();
        let msg = StatusMessage {
            text: "Loading...".to_string(),
            expires_at: None,
        };

        assert!(!msg.is_expired(now));
        assert!(!msg.is_expired(now + Duration::from_secs(3600)));
    }

    #[test]
    fn tab_cycles_the_four_top_level_views() {
        assert_eq!(next_top_view(View::Subscriptions), View::Queue);
        assert_eq!(next_top_view(View::Queue), View::Search);
        assert_eq!(next_top_view(View::Search), View::Settings);
        assert_eq!(next_top_view(View::Settings), View::Subscriptions);

        assert_eq!(prev_top_view(View::Subscriptions), View::Settings);
        assert_eq!(prev_top_view(View::Settings), View::Search);
        assert_eq!(prev_top_view(View::Search), View::Queue);
        assert_eq!(prev_top_view(View::Queue), View::Subscriptions);
    }

    #[test]
    fn episodes_cycles_as_its_parent_subscriptions() {
        // Episodes is a drill-down, not a tab: Tab out of it behaves like being
        // in Subscriptions, so it never dead-ends.
        assert_eq!(next_top_view(View::Episodes), View::Queue);
        assert_eq!(prev_top_view(View::Episodes), View::Settings);
    }
}
