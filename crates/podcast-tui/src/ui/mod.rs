pub mod components;

use crate::app::{AppState, state::Modal, state::View};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, Paragraph, Wrap},
};

pub struct Ui;

impl Ui {
    pub fn new() -> Self {
        Self
    }

    pub fn render(&self, f: &mut Frame, state: &AppState) {
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

    fn render_subscriptions(&self, f: &mut Frame, area: Rect, state: &AppState) {
        let items: Vec<ListItem> = state
            .subscriptions
            .iter()
            .enumerate()
            .map(|(i, sub)| {
                let style = if i == state.selected_index {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                        .bg(Color::DarkGray)
                } else {
                    Style::default().fg(Color::White)
                };

                let icon = if state
                    .current_subscription
                    .as_ref()
                    .is_some_and(|s| s.id == sub.id)
                {
                    "▶ "
                } else {
                    "  "
                };

                ListItem::new(format!("{}{}", icon, sub.title)).style(style)
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
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            );

        f.render_widget(list, area);
    }

    fn render_episodes(&self, f: &mut Frame, area: Rect, state: &AppState) {
        let title = state
            .current_subscription
            .as_ref()
            .map(|s| s.title.as_str())
            .unwrap_or("Episodes");

        let items: Vec<ListItem> = state
            .episodes
            .iter()
            .enumerate()
            .map(|(i, ep)| {
                let style = if i == state.selected_index {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                        .bg(Color::DarkGray)
                } else if ep.played {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default().fg(Color::White)
                };

                let played_icon = if ep.played { "✓" } else { " " };
                let download_icon = match &ep.download_status {
                    crate::models::DownloadStatus::Downloaded => "💾",
                    crate::models::DownloadStatus::Downloading => "⏬",
                    crate::models::DownloadStatus::Failed => "❌",
                    crate::models::DownloadStatus::NotDownloaded => "  ",
                };

                let duration = ep.duration_formatted();
                let text = format!(
                    "{} {} {} | {}",
                    played_icon, download_icon, ep.title, duration
                );

                ListItem::new(text).style(style)
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
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            );

        f.render_widget(list, area);
    }

    fn render_queue(&self, f: &mut Frame, area: Rect, state: &AppState) {
        if state.queue_items.is_empty() {
            let empty_msg = Paragraph::new(vec![
                Line::from(""),
                Line::from("📋 Queue is empty"),
                Line::from(""),
                Line::from("Press 'a' on an episode to add it to the queue"),
            ])
            .style(Style::default().fg(Color::Gray))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::White))
                    .title(" Queue (0) ")
                    .title_style(
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
            );

            f.render_widget(empty_msg, area);
            return;
        }

        let items: Vec<ListItem> = state
            .queue_items
            .iter()
            .enumerate()
            .map(|(i, ep)| {
                let style = if i == state.selected_index {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                        .bg(Color::DarkGray)
                } else {
                    Style::default().fg(Color::White)
                };

                let number = format!("{}.", i + 1);
                let duration = ep.duration_formatted();
                let text = format!("{:3} {} | {}", number, ep.title, duration);

                ListItem::new(text).style(style)
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::White))
                    .title(format!(" Queue ({}) ", state.queue_items.len()))
                    .title_style(
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .highlight_style(
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            );

        f.render_widget(list, area);
    }

    fn render_search(&self, f: &mut Frame, area: Rect, state: &AppState) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3), // Search input
                Constraint::Min(0),    // Results
            ])
            .split(area);

        // Search input box
        let input = Paragraph::new(state.search_input.as_str())
            .style(Style::default().fg(Color::Yellow))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(" 🔍 Search Podcasts (iTunes) ")
                    .title_style(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
            );

        f.render_widget(input, chunks[0]);

        // Show cursor in search box
        f.set_cursor_position((
            chunks[0].x + state.search_cursor as u16 + 1,
            chunks[0].y + 1,
        ));

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
                .enumerate()
                .map(|(i, result)| {
                    let style = if i == state.selected_index {
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD)
                            .bg(Color::DarkGray)
                    } else {
                        Style::default().fg(Color::White)
                    };

                    let text = format!("{} - {}", result.title, result.artist);

                    ListItem::new(text).style(style)
                })
                .collect();

            let list = List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::White))
                        .title(format!(" Results ({}) ", state.search_results.len()))
                        .title_style(
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD),
                        ),
                )
                .highlight_style(
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                );

            f.render_widget(list, chunks[1]);
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

        // Playback status line
        let status_text = if let Some(episode) = &state.current_episode {
            let status_icon = if state.is_playing {
                "▶️ "
            } else {
                "⏸️ "
            };
            format!(
                "{}{} | Speed: {:.1}x | Volume: {:.0}%",
                status_icon,
                episode.title,
                state.playback_speed,
                state.volume * 100.0
            )
        } else {
            "No episode playing".to_string()
        };

        let status = Paragraph::new(status_text)
            .style(Style::default().fg(Color::White))
            .alignment(Alignment::Left);

        f.render_widget(status, chunks[0]);

        // Progress bar
        if state.is_playing || state.current_episode.is_some() {
            let progress = if let Some(ep) = &state.current_episode {
                if let Some(duration) = ep.duration_seconds {
                    if duration > 0 {
                        ((state.playback_position / duration as f64) * 100.0) as u16
                    } else {
                        0
                    }
                } else {
                    0
                }
            } else {
                0
            };

            let gauge = Gauge::default()
                .block(Block::default())
                .gauge_style(
                    Style::default()
                        .fg(Color::Cyan)
                        .bg(Color::Black)
                        .add_modifier(Modifier::BOLD),
                )
                .percent(progress.min(100))
                .label(format!("{:.0}%", progress));

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
            Line::from("  x        Delete download"),
            Line::from("  r        Refresh feed"),
            Line::from("  R        Refresh all"),
            Line::from("  s        Toggle played"),
            Line::from(""),
            Line::from(Span::styled("Other:", Style::default().fg(Color::Cyan))),
            Line::from("  Enter    Select/Subscribe"),
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
