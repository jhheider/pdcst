pub mod components;

use crate::app::{state::View, AppState};
use ratatui::{
    backend::Backend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
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
                Constraint::Length(3), // Footer/Status
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
    }

    fn render_header(&self, f: &mut Frame, area: Rect, state: &AppState) {
        let title = match state.current_view {
            View::Subscriptions => "Subscriptions",
            View::Episodes => "Episodes",
            View::Queue => "Queue",
            View::Search => "Search",
            View::Settings => "Settings",
        };

        let header = Paragraph::new(format!("Podcast TUI - {}", title))
            .style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
            .block(Block::default().borders(Borders::ALL));

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
                } else {
                    Style::default()
                };

                ListItem::new(sub.title.clone()).style(style)
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title("Subscriptions"),
            )
            .highlight_style(Style::default().add_modifier(Modifier::ITALIC));

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
                } else {
                    Style::default()
                };

                let duration = ep.duration_formatted();
                let played_marker = if ep.played { "✓" } else { " " };

                ListItem::new(format!("[{}] {} ({})", played_marker, ep.title, duration))
                    .style(style)
            })
            .collect();

        let list = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(title))
            .highlight_style(Style::default().add_modifier(Modifier::ITALIC));

        f.render_widget(list, area);
    }

    fn render_queue(&self, f: &mut Frame, area: Rect, _state: &AppState) {
        let placeholder = Paragraph::new("Queue view (not yet implemented)")
            .block(Block::default().borders(Borders::ALL).title("Queue"));

        f.render_widget(placeholder, area);
    }

    fn render_search(&self, f: &mut Frame, area: Rect, _state: &AppState) {
        let placeholder = Paragraph::new("Search view (not yet implemented)")
            .block(Block::default().borders(Borders::ALL).title("Search"));

        f.render_widget(placeholder, area);
    }

    fn render_settings(&self, f: &mut Frame, area: Rect, _state: &AppState) {
        let placeholder = Paragraph::new("Settings view (not yet implemented)")
            .block(Block::default().borders(Borders::ALL).title("Settings"));

        f.render_widget(placeholder, area);
    }

    fn render_footer(&self, f: &mut Frame, area: Rect, state: &AppState) {
        let status = if let Some(episode) = &state.current_episode {
            let status_text = if state.is_playing {
                "Playing"
            } else {
                "Paused"
            };
            format!(
                "{}: {} | Speed: {:.1}x | Volume: {:.0}%",
                status_text,
                episode.title,
                state.playback_speed,
                state.volume * 100.0
            )
        } else {
            "No episode playing".to_string()
        };

        let help_text = "[1] Subscriptions [2] Queue [3] Search [Space] Play/Pause [q] Quit";

        let footer_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1)])
            .split(area);

        let status_widget = Paragraph::new(status).style(Style::default().fg(Color::White));

        let help_widget = Paragraph::new(help_text).style(Style::default().fg(Color::DarkGray));

        f.render_widget(Block::default().borders(Borders::ALL), area);
        f.render_widget(status_widget, footer_chunks[0]);
        f.render_widget(help_widget, footer_chunks[1]);
    }
}

impl Default for Ui {
    fn default() -> Self {
        Self::new()
    }
}
