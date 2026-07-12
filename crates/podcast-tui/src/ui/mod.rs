use crate::app::{AppState, state::Modal, state::SearchFocus, state::View};
use crate::models::Episode;
use crate::utils::time::format_duration;
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, Paragraph, Wrap},
};

/// Selected-row style: reverse video, so it reads correctly on any terminal
/// theme (light or dark) rather than assuming a dark background.
fn selection_style() -> Style {
    Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
}

/// A one-character listen-state marker for an episode row: played, in-progress
/// (has a saved resume position), or unplayed.
fn listen_marker(ep: &Episode) -> &'static str {
    if ep.played {
        "x"
    } else if ep.playback_position_seconds > 0 {
        "~"
    } else {
        " "
    }
}

pub struct Ui;

impl Ui {
    pub fn new() -> Self {
        Self
    }

    pub fn render(&self, f: &mut Frame, state: &mut AppState) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Header
                Constraint::Min(0),    // Main content
                Constraint::Length(4), // Footer/Status
            ])
            .split(f.area());

        // Render header
        self.render_header(f, chunks[0], state);

        // Render main content based on view
        match state.current_view {
            View::Subscriptions => self.render_subscriptions(f, chunks[1], state),
            View::Episodes => self.render_episodes(f, chunks[1], state),
            View::Queue => self.render_queue(f, chunks[1], state),
            View::Search => self.render_search(f, chunks[1], state),
            View::Settings => self.render_settings(f, chunks[1], state),
        }

        // Render footer
        self.render_footer(f, chunks[2], state);

        // Render modals on top
        match &state.modal {
            Modal::Help => self.render_help_modal(f, f.area()),
            Modal::Error(msg) => self.render_error_modal(f, f.area(), msg),
            Modal::Confirm { message, action } => {
                self.render_confirm_modal(f, f.area(), message, action)
            }
            Modal::None => {}
        }
    }

    fn render_header(&self, f: &mut Frame, area: Rect, state: &AppState) {
        let view_name = match state.current_view {
            View::Subscriptions => "📻 Subscriptions",
            View::Episodes => "🎙️  Episodes",
            View::Queue => "📋 Queue",
            View::Search => "🔍 Search",
            View::Settings => "⚙️  Settings",
        };

        let title = format!(" Podcast TUI - {} ", view_name);

        let header = Paragraph::new(title)
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan)),
            );

        f.render_widget(header, area);
    }

    fn render_subscriptions(&self, f: &mut Frame, area: Rect, state: &mut AppState) {
        // First-run guidance: an empty Subscriptions view is the literal fresh
        // install, so point at the two ways to add a podcast.
        if state.subscriptions.is_empty() {
            let empty = Paragraph::new(vec![
                Line::from(""),
                Line::from("No podcasts yet."),
                Line::from(""),
                Line::from("Press '/' to search and subscribe,"),
                Line::from("or import an OPML file with --import <file>."),
            ])
            .style(Style::default().fg(Color::Gray))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::White))
                    .title(" Subscriptions (0) ")
                    .title_style(
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
            );
            f.render_widget(empty, area);
            return;
        }

        let items: Vec<ListItem> = state
            .subscriptions
            .iter()
            .map(|sub| {
                let icon = if state
                    .current_subscription
                    .as_ref()
                    .is_some_and(|s| s.id == sub.id)
                {
                    "> "
                } else {
                    "  "
                };
                // Auto-queue indicator: Qv = add to bottom, Q^ = add to top.
                let aq = match (sub.auto_queue, sub.auto_queue_to_top) {
                    (false, _) => "   ",
                    (true, false) => "Qv ",
                    (true, true) => "Q^ ",
                };
                ListItem::new(format!("{}{}{}", icon, aq, sub.title))
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::White))
                    .title(format!(" Subscriptions ({}) ", state.subscriptions.len()))
                    .title_style(
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .highlight_style(selection_style());

        state.sync_list_selection();
        f.render_stateful_widget(list, area, &mut state.list_state);
    }

    fn render_episodes(&self, f: &mut Frame, area: Rect, state: &mut AppState) {
        let title = state
            .current_subscription
            .as_ref()
            .map(|s| s.title.clone())
            .unwrap_or_else(|| "Episodes".to_string());
        let playing_id = state.current_episode.as_ref().map(|e| e.id);

        let items: Vec<ListItem> = state
            .episodes
            .iter()
            .map(|ep| {
                // Dim finished episodes so the unheard ones stand out.
                let base = if ep.played {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default().fg(Color::White)
                };

                let now = if playing_id == Some(ep.id) { ">" } else { " " };
                let listen = listen_marker(ep);
                let download_icon = match &ep.download_status {
                    crate::models::DownloadStatus::Downloaded => "v",
                    crate::models::DownloadStatus::Downloading => "|",
                    crate::models::DownloadStatus::Failed => "!",
                    crate::models::DownloadStatus::NotDownloaded => " ",
                };

                let text = format!(
                    "{}{} {} {} | {}",
                    now,
                    listen,
                    download_icon,
                    ep.title,
                    ep.duration_formatted()
                );
                ListItem::new(text).style(base)
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::White))
                    .title(format!(" {} ({}) ", title, state.episodes.len()))
                    .title_style(
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .highlight_style(selection_style());

        state.sync_list_selection();
        f.render_stateful_widget(list, area, &mut state.list_state);
    }

    fn render_queue(&self, f: &mut Frame, area: Rect, state: &mut AppState) {
        if state.queue_items.is_empty() {
            let empty_msg = Paragraph::new(vec![
                Line::from(""),
                Line::from("Up Next is empty"),
                Line::from(""),
                Line::from("Press 'a' on an episode to add it to the queue"),
            ])
            .style(Style::default().fg(Color::Gray))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::White))
                    .title(" Up Next (0) ")
                    .title_style(
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
            );

            f.render_widget(empty_msg, area);
            return;
        }

        let playing_id = state.current_episode.as_ref().map(|e| e.id);
        let items: Vec<ListItem> = state
            .queue_items
            .iter()
            .enumerate()
            .map(|(i, ep)| {
                let now = if playing_id == Some(ep.id) { ">" } else { " " };
                let text = format!(
                    "{}{} {:2}. {} | {}",
                    now,
                    listen_marker(ep),
                    i + 1,
                    ep.title,
                    ep.duration_formatted()
                );
                ListItem::new(text)
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::White))
                    .title(format!(" Up Next ({}) ", state.queue_items.len()))
                    .title_style(
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .highlight_style(selection_style());

        state.sync_list_selection();
        f.render_stateful_widget(list, area, &mut state.list_state);
    }

    fn render_search(&self, f: &mut Frame, area: Rect, state: &mut AppState) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Search input
                Constraint::Min(0),    // Results
            ])
            .split(area);

        // The active pane (query box vs results list) gets a cyan border; the
        // inactive one is dimmed, so it is always clear where keys will go.
        let input_focused = state.search_focus == SearchFocus::Input;
        let input_border = if input_focused {
            Color::Cyan
        } else {
            Color::DarkGray
        };

        // Search input box
        let input = Paragraph::new(state.search_input.as_str())
            .style(Style::default().fg(Color::Yellow))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(input_border))
                    .title(" Search Podcasts (iTunes) ")
                    .title_style(
                        Style::default()
                            .fg(input_border)
                            .add_modifier(Modifier::BOLD),
                    ),
            );

        f.render_widget(input, chunks[0]);

        // Show the cursor only while the box is focused.
        if input_focused {
            f.set_cursor_position((
                chunks[0].x + state.search_cursor as u16 + 1,
                chunks[0].y + 1,
            ));
        }

        // Search results
        if state.search_results.is_empty() {
            let empty_msg = if state.search_input.is_empty() {
                Paragraph::new(vec![
                    Line::from(""),
                    Line::from("Type to search for podcasts..."),
                    Line::from("Press Enter to search"),
                ])
            } else {
                Paragraph::new(vec![
                    Line::from(""),
                    Line::from("No results found"),
                    Line::from("Try a different search term"),
                ])
            }
            .style(Style::default().fg(Color::Gray))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::White))
                    .title(" Results (0) ")
                    .title_style(
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
            );

            f.render_widget(empty_msg, chunks[1]);
        } else {
            let items: Vec<ListItem> = state
                .search_results
                .iter()
                .map(|result| ListItem::new(format!("{} - {}", result.title, result.artist)))
                .collect();

            let results_border = if input_focused {
                Color::White
            } else {
                Color::Cyan
            };
            let list = List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(results_border))
                        .title(format!(
                            " Results ({}) - Enter to subscribe ",
                            state.search_results.len()
                        ))
                        .title_style(
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD),
                        ),
                )
                .highlight_style(selection_style());

            state.sync_list_selection();
            f.render_stateful_widget(list, chunks[1], &mut state.list_state);
        }
    }

    fn render_settings(&self, f: &mut Frame, area: Rect, state: &AppState) {
        let settings_text = vec![
            Line::from(""),
            Line::from(Span::styled(
                "⚙️  Settings",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from("Configuration:"),
            Line::from(""),
            Line::from(Span::styled(
                format!("Download Directory: {:?}", state.config.download_dir),
                Style::default().fg(Color::White),
            )),
            Line::from(Span::styled(
                format!("Data Directory: {:?}", state.config.data_dir),
                Style::default().fg(Color::White),
            )),
            Line::from(Span::styled(
                format!(
                    "Downloads: {} concurrent",
                    state.config.max_concurrent_downloads
                ),
                Style::default().fg(Color::White),
            )),
            Line::from(Span::styled(
                format!(
                    "Refreshes: {} concurrent",
                    state.config.max_concurrent_refreshes
                ),
                Style::default().fg(Color::White),
            )),
            Line::from(Span::styled(
                match state.config.auto_refresh_interval_minutes {
                    0 => "Auto-refresh: off (manual only)".to_string(),
                    n => format!("Auto-refresh: every {n} min"),
                },
                Style::default().fg(Color::White),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Auto-queue:",
                Style::default().fg(Color::Cyan),
            )),
            Line::from(Span::styled(
                format!("  Max depth: {}", state.config.queue_max_depth),
                Style::default().fg(Color::White),
            )),
            Line::from(Span::styled(
                format!(
                    "  Default direction: {}",
                    if state.config.auto_queue_to_top_default {
                        "top"
                    } else {
                        "bottom"
                    }
                ),
                Style::default().fg(Color::White),
            )),
            Line::from(Span::styled(
                format!("  Smart interleave: {}", state.config.smart_interleave),
                Style::default().fg(Color::White),
            )),
            Line::from(Span::styled(
                "  Toggle a feed's auto-queue with 'A' in Subscriptions.",
                Style::default().fg(Color::Gray),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Press '?' for help",
                Style::default().fg(Color::Gray),
            )),
        ];

        let settings = Paragraph::new(settings_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::White))
                    .title(" Settings ")
                    .title_style(
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .wrap(Wrap { trim: true });

        f.render_widget(settings, area);
    }

    fn render_footer(&self, f: &mut Frame, area: Rect, state: &AppState) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // Playback status
                Constraint::Length(1), // Progress bar
                Constraint::Length(1), // Help text
            ])
            .split(area);

        // Playback status line: now-playing plus a persistent "Up Next: N" so
        // the queue depth is visible from any view (the auto-queue is the point).
        let up_next = format!("Up Next: {}", state.queue_items.len());
        let status_text = if let Some(episode) = &state.current_episode {
            let status_icon = if state.is_playing { ">" } else { "||" };
            format!(
                "{} {} | {:.1}x | Vol {:.0}% | {}",
                status_icon,
                episode.title,
                state.playback_speed,
                state.volume * 100.0,
                up_next
            )
        } else {
            format!("No episode playing | {}", up_next)
        };

        let status = Paragraph::new(status_text)
            .style(Style::default().fg(Color::White))
            .alignment(Alignment::Left);

        f.render_widget(status, chunks[0]);

        // Progress bar, labelled with elapsed / duration (12:34 / 45:00) rather
        // than a bare percent.
        if let Some(ep) = &state.current_episode {
            let elapsed = state.playback_position.max(0.0) as i64;
            let total = ep.duration_seconds.unwrap_or(0).max(0);
            let percent = if total > 0 {
                ((elapsed as f64 / total as f64) * 100.0) as u16
            } else {
                0
            };
            let label = if total > 0 {
                format!("{} / {}", format_duration(elapsed), format_duration(total))
            } else {
                format_duration(elapsed)
            };

            let gauge = Gauge::default()
                .block(Block::default())
                .gauge_style(
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )
                .percent(percent.min(100))
                .label(label);

            f.render_widget(gauge, chunks[1]);
        } else {
            let empty = Paragraph::new(" ");
            f.render_widget(empty, chunks[1]);
        }

        // Help text
        let help_text = "[1-4] Views | [Tab] Cycle | [Enter] Open | [/] Search | [Space] Play | [?] Help | [q] Quit";

        let help = Paragraph::new(help_text)
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);

        f.render_widget(help, chunks[2]);

        // Status message overlay if present
        if let Some(msg) = state.current_status() {
            let status_area = Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: 1,
            };

            let status_msg = Paragraph::new(msg)
                .style(
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
                .alignment(Alignment::Center);

            f.render_widget(status_msg, status_area);
        }
    }

    fn render_help_modal(&self, f: &mut Frame, area: Rect) {
        let popup_area = centered_rect(80, 80, area);

        let help_text = vec![
            Line::from(Span::styled(
                "⌨️  Keyboard Shortcuts",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Navigation:",
                Style::default().fg(Color::Cyan),
            )),
            Line::from("  j/↓      Move down"),
            Line::from("  k/↑      Move up"),
            Line::from("  g        Go to top"),
            Line::from("  G        Go to bottom"),
            Line::from("  PgUp/PgDn Page up/down"),
            Line::from(""),
            Line::from(Span::styled("Views:", Style::default().fg(Color::Cyan))),
            Line::from("  1        Subscriptions"),
            Line::from("  2        Queue"),
            Line::from("  3        Search"),
            Line::from("  4        Settings"),
            Line::from("  Tab      Next view"),
            Line::from("  Shift+Tab Previous view"),
            Line::from("  Enter    Open podcast (episodes)"),
            Line::from("  Esc      Back to subscriptions"),
            Line::from(""),
            Line::from(Span::styled("Playback:", Style::default().fg(Color::Cyan))),
            Line::from("  Space    Play/Pause"),
            Line::from("  n        Next in queue"),
            Line::from("  p        Restart episode"),
            Line::from("  ←/→      Seek -10s/+10s"),
            Line::from("  </  >     Seek -30s/+30s"),
            Line::from(""),
            Line::from(Span::styled("Controls:", Style::default().fg(Color::Cyan))),
            Line::from("  +/-      Volume up/down"),
            Line::from("  m        Mute"),
            Line::from("  [/]      Speed down/up"),
            Line::from("  a        Add to queue"),
            Line::from("  d        Download episode"),
            Line::from("  x        Remove (queue) / delete download"),
            Line::from("  n        Skip to next in queue"),
            Line::from("  r        Refresh feed"),
            Line::from("  R        Refresh all"),
            Line::from("  s        Toggle played"),
            Line::from("  A        Cycle auto-queue (off/bottom/top)"),
            Line::from(""),
            Line::from(Span::styled("Other:", Style::default().fg(Color::Cyan))),
            Line::from("  Enter    Open / play / subscribe"),
            Line::from("  /        Search mode"),
            Line::from("  Esc      Close modal/Back"),
            Line::from("  ?        This help"),
            Line::from("  q        Quit (Ctrl-C while typing)"),
            Line::from(""),
            Line::from(Span::styled(
                "Press Esc to close",
                Style::default().fg(Color::Gray),
            )),
        ];

        let help_widget = Paragraph::new(help_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow))
                    .title(" Help ")
                    .title_style(
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .wrap(Wrap { trim: false });

        f.render_widget(Clear, popup_area);
        f.render_widget(help_widget, popup_area);
    }

    fn render_error_modal(&self, f: &mut Frame, area: Rect, message: &str) {
        let popup_area = centered_rect(60, 30, area);

        let error_text = vec![
            Line::from(""),
            Line::from(Span::styled(
                "❌ Error",
                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(message),
            Line::from(""),
            Line::from(Span::styled(
                "Press Esc to close",
                Style::default().fg(Color::Gray),
            )),
        ];

        let error_widget = Paragraph::new(error_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Red))
                    .title(" Error ")
                    .title_style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            )
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });

        f.render_widget(Clear, popup_area);
        f.render_widget(error_widget, popup_area);
    }

    fn render_confirm_modal(&self, f: &mut Frame, area: Rect, message: &str, _action: &str) {
        let popup_area = centered_rect(50, 20, area);

        let confirm_text = vec![
            Line::from(""),
            Line::from(Span::styled(
                "⚠️  Confirm",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(message),
            Line::from(""),
            Line::from(Span::styled(
                "Press Enter to confirm, Esc to cancel",
                Style::default().fg(Color::Gray),
            )),
        ];

        let confirm_widget = Paragraph::new(confirm_text)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow))
                    .title(" Confirm ")
                    .title_style(
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .alignment(Alignment::Center)
            .wrap(Wrap { trim: true });

        f.render_widget(Clear, popup_area);
        f.render_widget(confirm_widget, popup_area);
    }
}

impl Default for Ui {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper function to create a centered rect
fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
