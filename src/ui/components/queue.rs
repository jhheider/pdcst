use ratatui::{
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem},
};

pub struct QueuePanel;

impl QueuePanel {
    pub fn render_list(items: Vec<String>, selected: usize) -> List<'static> {
        let list_items: Vec<ListItem> = items
            .into_iter()
            .enumerate()
            .map(|(i, title)| {
                let style = if i == selected {
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };

                ListItem::new(title).style(style)
            })
            .collect();

        List::new(list_items)
            .block(Block::default().borders(Borders::ALL).title("Queue"))
            .highlight_style(Style::default().add_modifier(Modifier::ITALIC))
    }
}
