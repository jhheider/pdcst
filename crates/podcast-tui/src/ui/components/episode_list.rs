use crate::models::Episode;
use ratatui::{
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem},
};

pub struct EpisodeList;

impl EpisodeList {
    pub fn render(episodes: &[Episode], selected: usize, title: &str) -> List<'static> {
        let items: Vec<ListItem> = episodes
            .iter()
            .enumerate()
            .map(|(i, ep)| {
                let style = if i == selected {
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

        List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title.to_string()),
            )
            .highlight_style(Style::default().add_modifier(Modifier::ITALIC))
    }
}
