//! Keyboard input handling: modal gate, search text-entry gate, then the global
//! keymap. Called from the run loop for every key press.

use super::App;
use crate::app::state;
use anyhow::Result;
use crossterm::event::KeyCode;

impl App {
    pub(crate) async fn handle_key_event(&mut self, key: KeyCode) -> Result<()> {
        use state::{Modal, View};

        // Handle modal-specific keys first
        match &self.state.modal {
            Modal::Help | Modal::Error(_) => {
                match key {
                    KeyCode::Esc | KeyCode::Char('q') => {
                        self.state.close_modal();
                        return Ok(());
                    }
                    _ => return Ok(()), // Ignore other keys when modal is open
                }
            }
            Modal::Confirm { .. } => {
                match key {
                    KeyCode::Enter => {
                        // TODO: Execute confirmed action
                        self.state.close_modal();
                        return Ok(());
                    }
                    KeyCode::Esc => {
                        self.state.close_modal();
                        return Ok(());
                    }
                    _ => return Ok(()),
                }
            }
            Modal::None => {}
        }

        // Handle search input. Only while the query box has focus: every
        // printable key (digits and 'q' included) types into it; Esc and Enter
        // escape to the global handlers. Once results are focused, keys fall
        // through so j/k browse and Enter subscribes.
        if self.state.current_view == View::Search
            && self.state.search_focus == state::SearchFocus::Input
            && !matches!(key, KeyCode::Esc)
        {
            match key {
                KeyCode::Char(c) if !c.is_control() => {
                    self.state.append_search_char(c);
                    return Ok(());
                }
                KeyCode::Backspace => {
                    self.state.delete_search_char();
                    return Ok(());
                }
                KeyCode::Enter => {
                    // Run the search, then move focus to the results list.
                    if !self.state.search_input.is_empty() {
                        self.state.set_status("Searching...".to_string());
                        match self
                            .state
                            .search_podcasts(&self.state.search_input.clone())
                            .await
                        {
                            Ok(_) => {
                                self.state.clear_status();
                                self.state.focus_search_results();
                            }
                            Err(e) => {
                                self.state.show_error(format!("Search failed: {}", e));
                            }
                        }
                    }
                    return Ok(());
                }
                _ => {}
            }
        }

        // Global shortcuts
        match key {
            // Help modal
            KeyCode::Char('?') => {
                self.state.show_help_modal();
                return Ok(());
            }

            // Quit (a literal 'q' while typing is handled by the search gate above).
            KeyCode::Char('q') => {
                self.state.should_quit = true;
                return Ok(());
            }

            // Esc - close a modal, step back within Search, leave Search, or
            // drill back out of Episodes.
            KeyCode::Esc => {
                if self.state.modal != Modal::None {
                    self.state.close_modal();
                } else if self.state.current_view == View::Search {
                    // From results, step back to the query box; from the box, exit.
                    if self.state.search_focus == state::SearchFocus::Results {
                        self.state.focus_search_input();
                    } else {
                        self.state.exit_search_mode();
                    }
                } else if self.state.current_view == View::Episodes {
                    self.state.set_view(View::Subscriptions);
                }
                return Ok(());
            }

            // Playback controls
            KeyCode::Char(' ') => {
                if let Err(e) = self.state.toggle_playback().await {
                    self.state.show_error(format!("Playback error: {}", e));
                }
            }
            KeyCode::Char('n') => {
                if let Err(e) = self.state.play_next_in_queue().await {
                    self.state.show_error(format!("Failed to play next: {}", e));
                }
            }
            KeyCode::Char('p') | KeyCode::Char('P') => {
                if let Err(e) = self.state.restart_current_episode().await {
                    self.state.show_error(format!("Failed to restart: {}", e));
                }
            }

            // Volume controls
            KeyCode::Char('+') | KeyCode::Char('=') => {
                if let Err(e) = self.state.increase_volume(0.1).await {
                    tracing::error!("Volume error: {}", e);
                }
            }
            KeyCode::Char('-') | KeyCode::Char('_') => {
                if let Err(e) = self.state.decrease_volume(0.1).await {
                    tracing::error!("Volume error: {}", e);
                }
            }
            KeyCode::Char('m') => {
                if let Err(e) = self.state.toggle_mute().await {
                    tracing::error!("Mute error: {}", e);
                }
            }

            // Playback speed controls
            KeyCode::Char('[') => {
                if let Err(e) = self.state.decrease_speed(0.1).await {
                    tracing::error!("Speed error: {}", e);
                }
            }
            KeyCode::Char(']') => {
                if let Err(e) = self.state.increase_speed(0.1).await {
                    tracing::error!("Speed error: {}", e);
                }
            }

            // Seeking
            KeyCode::Left => {
                if let Err(e) = self.state.seek_backward(10.0).await {
                    tracing::error!("Seek error: {}", e);
                }
            }
            KeyCode::Right => {
                if let Err(e) = self.state.seek_forward(10.0).await {
                    tracing::error!("Seek error: {}", e);
                }
            }
            KeyCode::Char('<') => {
                if let Err(e) = self.state.seek_backward(30.0).await {
                    tracing::error!("Seek error: {}", e);
                }
            }
            KeyCode::Char('>') => {
                if let Err(e) = self.state.seek_forward(30.0).await {
                    tracing::error!("Seek error: {}", e);
                }
            }

            // View navigation
            KeyCode::Char('1') => {
                self.state.set_view(View::Subscriptions);
                self.state.selected_index = 0;
            }
            KeyCode::Char('2') => {
                self.state.set_view(View::Queue);
                self.state.selected_index = 0;
                // Load queue items
                if let Err(e) = self.state.load_queue().await {
                    self.state
                        .show_error(format!("Failed to load queue: {}", e));
                }
            }
            KeyCode::Char('3') => {
                self.state.set_view(View::Search);
                self.state.selected_index = 0;
                self.state.clear_search_input();
            }
            KeyCode::Char('4') => {
                self.state.set_view(View::Settings);
                self.state.selected_index = 0;
            }
            KeyCode::Tab => {
                self.state.next_view();
                self.state.selected_index = 0;
                // Load data for new view
                if self.state.current_view == View::Queue {
                    let _ = self.state.load_queue().await;
                }
            }
            KeyCode::BackTab => {
                self.state.previous_view();
                self.state.selected_index = 0;
                // Load data for new view
                if self.state.current_view == View::Queue {
                    let _ = self.state.load_queue().await;
                }
            }

            // List navigation
            KeyCode::Up | KeyCode::Char('k') => {
                self.state.previous_item();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.state.next_item();
            }
            KeyCode::Char('g') => {
                self.state.goto_top();
            }
            KeyCode::Char('G') => {
                self.state.goto_bottom();
            }
            KeyCode::PageUp => {
                self.state.page_up();
            }
            KeyCode::PageDown => {
                self.state.page_down();
            }

            // Item selection and actions
            KeyCode::Enter => {
                if let Err(e) = self.state.select_item().await {
                    self.state.show_error(format!("Selection failed: {}", e));
                }
            }
            KeyCode::Char('a') => {
                match self.state.add_selected_to_queue().await {
                    Err(e) => {
                        self.state
                            .show_error(format!("Failed to add to queue: {}", e));
                    }
                    _ => {
                        // Transient status auto-clears at render time (no block).
                        self.state.set_status("Added to queue".to_string());
                    }
                }
            }
            KeyCode::Char('d') => {
                self.state.set_status("Starting download...".to_string());
                match self.state.download_selected_episode().await {
                    Err(e) => {
                        self.state.show_error(format!("Download failed: {}", e));
                    }
                    _ => {
                        self.state.set_status("Download started".to_string());
                    }
                }
            }
            // 'x' means "remove": from the Queue, drop the selected item; from
            // Episodes, delete its download. Each is a no-op in the other view.
            KeyCode::Char('x') => {
                if self.state.current_view == View::Queue {
                    match self.state.remove_selected_from_queue().await {
                        Err(e) => self.state.show_error(format!("Failed to remove: {}", e)),
                        _ => self.state.set_status("Removed from queue".to_string()),
                    }
                } else {
                    match self.state.delete_selected_download().await {
                        Err(e) => self.state.show_error(format!("Failed to delete: {}", e)),
                        _ => self.state.set_status("Download deleted".to_string()),
                    }
                }
            }
            KeyCode::Char('r') => {
                self.state.set_status("Refreshing feed...".to_string());
                match self.state.refresh_selected_subscription().await {
                    Err(e) => {
                        self.state.show_error(format!("Refresh failed: {}", e));
                    }
                    _ => {
                        self.state.set_status("Feed refreshed".to_string());
                    }
                }
            }
            KeyCode::Char('R') => {
                self.state.set_status("Refreshing all feeds...".to_string());
                match self.state.refresh_all_subscriptions().await {
                    Err(e) => {
                        self.state.show_error(format!("Refresh all failed: {}", e));
                    }
                    _ => {
                        self.state.set_status("All feeds refreshed".to_string());
                    }
                }
            }
            KeyCode::Char('s') => match self.state.toggle_played_status().await {
                Err(e) => {
                    self.state
                        .show_error(format!("Failed to toggle played: {}", e));
                }
                _ => {
                    self.state.set_status("Toggled played status".to_string());
                }
            },

            // Search
            KeyCode::Char('/') => {
                self.state.enter_search_mode();
            }

            _ => {}
        }

        Ok(())
    }
}
