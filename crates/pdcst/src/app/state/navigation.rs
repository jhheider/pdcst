//! AppState: view and list navigation (which view/pane, which item, scroll).
//!
//! The Subscriptions and Episodes views are the two panes of the library and are
//! on screen together, so each keeps its own cursor + scroll state
//! (`subscription_*` / `episode_*`). Queue and Search are single-list views and
//! share `selected_index` / `list_state`. The navigation keys always act on the
//! currently focused list, dispatched by `current_view`.

#[allow(unused_imports)]
use super::*;

impl AppState {
    pub fn set_view(&mut self, view: View) {
        self.current_view = view;
        // Reset the target view's own cursor + scroll. Focus moves *between* the
        // two library panes go through `focus_subscriptions`/`focus_episodes`
        // instead, which preserve the other pane's cursor.
        match view {
            View::Subscriptions => {
                self.subscription_index = 0;
                self.subscription_list_state = ListState::default();
            }
            View::Episodes => {
                self.episode_index = 0;
                self.episode_list_state = ListState::default();
            }
            _ => {
                self.selected_index = 0;
                self.list_state = ListState::default();
            }
        }
        // Entering Search always starts in the query box.
        if view == View::Search {
            self.search_focus = SearchFocus::Input;
        }
    }

    /// Focus the left (Subscriptions) pane without moving its cursor; the Esc/h
    /// path back out of the episode list, which must land on the same feed you
    /// drilled into rather than jumping to the top.
    pub fn focus_subscriptions(&mut self) {
        self.current_view = View::Subscriptions;
    }

    /// Focus the right (Episodes) pane, starting at the newest episode (top).
    pub fn focus_episodes(&mut self) {
        self.current_view = View::Episodes;
        self.episode_index = 0;
        self.episode_list_state = ListState::default();
    }

    /// Number of items in the currently focused list (0 for Settings).
    pub fn current_list_len(&self) -> usize {
        match self.current_view {
            View::Subscriptions => self.subscriptions.len(),
            View::Episodes => self.episodes.len(),
            View::Queue => self.queue_items.len(),
            View::Search => self.search_results.len(),
            View::Settings => 0,
        }
    }

    /// The focused list's cursor: the library panes have their own, the
    /// single-list views share `selected_index`.
    fn focused_index_mut(&mut self) -> &mut usize {
        match self.current_view {
            View::Subscriptions => &mut self.subscription_index,
            View::Episodes => &mut self.episode_index,
            _ => &mut self.selected_index,
        }
    }

    /// Clamp a stored cursor to `[0, len-1]` and point a list state at it, so a
    /// stateful render highlights the right row and scrolls to keep it visible.
    fn point(list_state: &mut ListState, index: usize, len: usize) {
        let selected = if len == 0 {
            None
        } else {
            Some(index.min(len - 1))
        };
        list_state.select(selected);
    }

    /// Point the Queue/Search list state at `selected_index`. Called by those
    /// views each frame before rendering.
    pub fn sync_list_selection(&mut self) {
        let len = self.current_list_len();
        Self::point(&mut self.list_state, self.selected_index, len);
    }

    /// Point the left-pane (Subscriptions) list state at its cursor.
    pub fn sync_subscription_selection(&mut self) {
        Self::point(
            &mut self.subscription_list_state,
            self.subscription_index,
            self.subscriptions.len(),
        );
    }

    /// Point the right-pane (Episodes) list state at its cursor.
    pub fn sync_episode_selection(&mut self) {
        Self::point(
            &mut self.episode_list_state,
            self.episode_index,
            self.episodes.len(),
        );
    }

    /// Highest selectable index in the focused list (0 when empty).
    fn max_index(&self) -> usize {
        self.current_list_len().saturating_sub(1)
    }

    pub fn next_item(&mut self) {
        let max = self.max_index();
        let idx = self.focused_index_mut();
        if *idx < max {
            *idx += 1;
        }
    }

    pub fn previous_item(&mut self) {
        let idx = self.focused_index_mut();
        if *idx > 0 {
            *idx -= 1;
        }
    }

    // View navigation methods

    pub fn next_view(&mut self) {
        self.set_view(next_top_view(self.current_view));
    }

    pub fn previous_view(&mut self) {
        self.set_view(prev_top_view(self.current_view));
    }

    // List navigation methods

    pub fn goto_top(&mut self) {
        *self.focused_index_mut() = 0;
    }

    pub fn goto_bottom(&mut self) {
        let max = self.max_index();
        *self.focused_index_mut() = max;
    }

    pub fn page_up(&mut self) {
        let idx = self.focused_index_mut();
        *idx = idx.saturating_sub(10);
    }

    pub fn page_down(&mut self) {
        let max = self.max_index();
        let idx = self.focused_index_mut();
        *idx = (*idx + 10).min(max);
    }
}
