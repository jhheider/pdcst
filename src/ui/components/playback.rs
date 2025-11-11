use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::{Block, Borders, Paragraph},
};

pub struct PlaybackPanel;

impl PlaybackPanel {
    pub fn render(
        episode_title: &str,
        position: f64,
        duration: Option<i64>,
        is_playing: bool,
        speed: f32,
        volume: f32,
    ) -> Paragraph<'static> {
        let status = if is_playing { "▶" } else { "⏸" };

        let position_str = format!(
            "{:02}:{:02}",
            (position as i64) / 60,
            (position as i64) % 60
        );
        let duration_str = duration
            .map(|d| format!("{:02}:{:02}", d / 60, d % 60))
            .unwrap_or_else(|| "--:--".to_string());

        let content = format!(
            "{} {}\n{} {} / {}\nSpeed: {:.1}x | Volume: {:.0}%",
            status,
            episode_title,
            "═".repeat(20),
            position_str,
            duration_str,
            speed,
            volume * 100.0
        );

        Paragraph::new(content)
            .style(Style::default().fg(Color::White))
            .block(Block::default().borders(Borders::ALL).title("Playback"))
    }
}
