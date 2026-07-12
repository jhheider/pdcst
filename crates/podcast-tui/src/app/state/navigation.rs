//! AppState: view and list navigation (which view, which item, scroll).

#[allow(unused_imports)]
use super::*;

impl AppState {
    pub fn set_view(&mut self, view: View) {
        self.current_view = view;
        self.selected_index = 0;
        // New view, new list: clear the scroll offset and selection so the next
        // render starts at the top rather than inheriting the old view's scroll.
        self.list_state = ListState::default();
        // Entering Search always starts in the query box.
        if view == View::Search {
            self.search_focus = SearchFocus::Input;
        }
    }

    /// Number of items in the current view's list (0 for non-list views).
    pub fn current_list_len(&self) -> usize {
        match self.current_view {
            View::Subscriptions => self.subscriptions.len(),
            View::Episodes => self.episodes.len(),
            View::Queue => self.queue_items.len(),
            View::Search => self.search_results.len(),
            View::Settings => 0,
        }
    }

    /// Point `list_state` at the current selection (clamped to the list), so a
    /// stateful list render highlights the right row and scrolls to keep it
    /// visible. Called by the UI each frame before rendering a list.
    pub fn sync_list_selection(&mut self) {
        let len = self.current_list_len();
        let selected = if len == 0 {
            None
        } else {
            Some(self.selected_index.min(len - 1))
        };
        self.list_state.select(selected);
    }

    /// Highest selectable index in the current view (0 when empty).
    fn max_index(&self) -> usize {
        self.current_list_len().saturating_sub(1)
    }

    pub fn next_item(&mut self) {
        if self.selected_index < self.max_index() {
            self.selected_index += 1;
        }
    }

    pub fn previous_item(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
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
        self.selected_index = 0;
    }

    pub fn goto_bottom(&mut self) {
        self.selected_index = self.max_index();
    }

    pub fn page_up(&mut self) {
        self.selected_index = self.selected_index.saturating_sub(10);
    }

    pub fn page_down(&mut self) {
        self.selected_index = (self.selected_index + 10).min(self.max_index());
    }
}
