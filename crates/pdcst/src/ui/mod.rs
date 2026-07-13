use crate::app::{AppState, state::Modal, state::SearchFocus, state::View};
use crate::models::Episode;
use crate::utils::text::truncate_display;
use crate::utils::time::{format_duration, format_relative_time};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Gauge, List, ListItem, Paragraph, Wrap},
};
use std::collections::HashSet;
use uuid::Uuid;

/// Selected-row style: reverse video, so it reads correctly on any terminal
/// theme (light or dark) rather than assuming a dark background.
fn selection_style() -> Style {
    Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
}

// --- Semantic colours -------------------------------------------------------
// A small, meaning-carrying palette so colour is information, not decoration,
// and the eye gets anchors in a wall of list rows:
//   cyan   = focus / now-playing        green  = new / unheard / downloaded
//   yellow = in-progress / queued / busy red    = error / failed
//   blue   = dates & times              dark-gray = secondary / played / rules

/// Dimmed style for separators and secondary metadata.
fn dim_style() -> Style {
    Style::default().fg(Color::DarkGray)
}

/// Style for a relative date/time (`4 days ago`, `1h 5m`): a calm blue that
/// stands apart from the gray rules without shouting.
fn time_style() -> Style {
    Style::default().fg(Color::Blue)
}

/// The now-playing `>` marker pops in the focus accent; an idle row is blank.
fn now_marker_style(is_now: bool) -> Style {
    if is_now {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    }
}

/// Listen-state marker colour: in-progress (a saved resume position) pops
/// yellow, a finished episode dims, an unheard one is plain.
fn listen_marker_style(ep: &Episode) -> Style {
    if ep.played {
        Style::default().fg(Color::DarkGray)
    } else if ep.playback_position_seconds > 0 {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    }
}

/// Download-status icon colour: done green, in-flight yellow, failed red.
fn download_icon_style(status: &crate::models::DownloadStatus) -> Style {
    use crate::models::DownloadStatus::*;
    match status {
        Downloaded => Style::default().fg(Color::Green),
        Downloading => Style::default().fg(Color::Yellow),
        Failed => Style::default().fg(Color::Red),
        NotDownloaded => Style::default().fg(Color::DarkGray),
    }
}

/// Border colour for a pane: cyan when it holds keyboard focus, dim otherwise,
/// so it is always obvious which pane the keys drive.
fn pane_border(focused: bool) -> Color {
    if focused {
        Color::Cyan
    } else {
        Color::DarkGray
    }
}

/// The host of a feed URL (e.g. `feeds.simplecast.com`), for the search-result
/// metadata line - a moved feed is often recognisable by where it now lives.
/// Returns `None` for a URL without a clear host.
fn feed_host(url: &str) -> Option<String> {
    let after_scheme = url.split("://").nth(1).unwrap_or(url);
    let host = after_scheme.split('/').next().unwrap_or("");
    let host = host.split('@').next_back().unwrap_or(host); // drop any userinfo
    let host = host.split(':').next().unwrap_or(host); // drop any port
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
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

/// A one-line, tag-stripped preview of an episode description for the card's
/// third line. HTML tags and collapsed whitespace are removed; the result is
/// truncated to `max` display columns (grapheme-aware, so a cluster is never
/// split). Feed text is already entity-decoded and emoji-normalized at ingest
/// (see [`crate::utils::text::clean_feed_text`]), so this only strips markup and
/// bounds the width. Returns an empty string when there's nothing worth showing.
fn description_snippet(desc: &str, max: usize) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut last_was_space = false;
    for c in desc.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if in_tag => {}
            c if c.is_whitespace() => {
                if !last_was_space && !out.is_empty() {
                    out.push(' ');
                    last_was_space = true;
                }
            }
            c => {
                out.push(c);
                last_was_space = false;
            }
        }
    }
    truncate_display(out.trim_end(), max)
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

        // Render main content based on view. Subscriptions and Episodes are the
        // two panes of the library and are drawn together (which pane is focused
        // is `current_view`); the other views take the full width.
        match state.current_view {
            View::Subscriptions | View::Episodes => self.render_library(f, chunks[1], state),
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
            View::Subscriptions | View::Episodes => "📚 Library",
            View::Queue => "📋 Queue",
            View::Search => "🔍 Search",
            View::Settings => "⚙️  Settings",
        };

        let title = format!(" pdcst - {} ", view_name);

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

    /// The library: Subscriptions (left) and Episodes (right) side by side, so a
    /// wide terminal shows the feed you are browsing and its episodes at once.
    /// `current_view` picks which pane has focus (and the reverse-video cursor);
    /// the left pane live-previews the highlighted feed into the right one.
    fn render_library(&self, f: &mut Frame, area: Rect, state: &mut AppState) {
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(38), Constraint::Percentage(62)])
            .split(area);

        let subs_focused = state.current_view == View::Subscriptions;
        self.render_subscriptions_pane(f, panes[0], state, subs_focused);
        self.render_episodes_pane(f, panes[1], state, !subs_focused);
    }

    fn render_subscriptions_pane(
        &self,
        f: &mut Frame,
        area: Rect,
        state: &mut AppState,
        focused: bool,
    ) {
        let border = pane_border(focused);

        // First-run guidance: an empty list is the literal fresh install, so
        // point at the two ways to add a podcast.
        if state.subscriptions.is_empty() {
            let empty = Paragraph::new(vec![
                Line::from(""),
                Line::from("No podcasts yet."),
                Line::from(""),
                Line::from("Press '/' to search"),
                Line::from("and subscribe, or"),
                Line::from("import an OPML file"),
                Line::from("with --import <file>."),
                Line::from(""),
                Line::from("Then press 'A' on a"),
                Line::from("feed to auto-fill"),
                Line::from("Up Next."),
            ])
            .style(Style::default().fg(Color::Gray))
            .alignment(Alignment::Center)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border))
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

        let current_id = state.current_subscription.as_ref().map(|s| s.id);
        let items: Vec<ListItem> = state
            .subscriptions
            .iter()
            .map(|sub| {
                let cur = if current_id == Some(sub.id) { ">" } else { " " };
                // Auto-queue indicator: Qv = add to bottom, Q^ = add to top.
                let aq = match (sub.auto_queue, sub.auto_queue_to_top) {
                    (false, _) => "  ",
                    (true, false) => "Qv",
                    (true, true) => "Q^",
                };
                let has_error = sub.last_error.is_some();
                let err_mark = if has_error { "!" } else { " " };

                // Line 1: markers + title. A failing feed's title is dimmed and
                // its `!` is red, so a dead/unparseable feed reads at a glance.
                let title_style = if has_error {
                    Style::default().fg(Color::Gray)
                } else {
                    Style::default().fg(Color::White)
                };
                let line1 = Line::from(vec![
                    Span::styled(format!("{cur}{aq}"), Style::default().fg(Color::Cyan)),
                    Span::styled(
                        format!("{err_mark} "),
                        Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(sub.title.clone(), title_style),
                ]);

                // Line 2: the error (if any), else the counts + newest-episode
                // age. A nonzero unheard count is green so feeds with something
                // new to hear catch the eye; the age is blue like episode dates.
                let dim = dim_style();
                let line2 = if let Some(err) = &sub.last_error {
                    Line::from(Span::styled(
                        format!("   {err}"),
                        Style::default().fg(Color::Red),
                    ))
                } else {
                    let new_style = if sub.new_count > 0 {
                        Style::default().fg(Color::Green)
                    } else {
                        dim
                    };
                    let mut spans = vec![
                        Span::raw("   "),
                        Span::styled(format!("{} new", sub.new_count), new_style),
                        Span::styled(" | ", dim),
                        Span::styled(format!("{} eps", sub.episode_count), dim),
                    ];
                    if let Some(latest) = &sub.latest_episode_at {
                        spans.push(Span::styled(" | ", dim));
                        spans.push(Span::styled(format_relative_time(latest), time_style()));
                    }
                    Line::from(spans)
                };

                ListItem::new(vec![line1, line2])
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border))
                    .title(format!(" Subscriptions ({}) ", state.subscriptions.len()))
                    .title_style(
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .highlight_style(selection_style());

        state.sync_subscription_selection();
        f.render_stateful_widget(list, area, &mut state.subscription_list_state);
    }

    fn render_episodes_pane(&self, f: &mut Frame, area: Rect, state: &mut AppState, focused: bool) {
        let border = pane_border(focused);
        let title = state
            .current_subscription
            .as_ref()
            .map(|s| s.title.clone())
            .unwrap_or_else(|| "Episodes".to_string());

        // A freshly-subscribed feed can be empty until its refresh lands - say so
        // rather than showing a blank box.
        if state.episodes.is_empty() {
            let hint = if state.current_subscription.is_some() {
                "No episodes yet. Press 'r' to refresh this feed."
            } else {
                "Select a podcast to see its episodes."
            };
            let empty = Paragraph::new(vec![Line::from(""), Line::from(hint)])
                .style(Style::default().fg(Color::Gray))
                .alignment(Alignment::Center)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(border))
                        .title(format!(" {} (0) ", title))
                        .title_style(
                            Style::default()
                                .fg(Color::Green)
                                .add_modifier(Modifier::BOLD),
                        ),
                );
            f.render_widget(empty, area);
            return;
        }

        let playing_id = state.current_episode.as_ref().map(|e| e.id);
        // Queue membership is a UI cross-reference (no flag on Episode): build the
        // set of queued ids once per render rather than scanning per row.
        let queued: HashSet<Uuid> = state.queue_items.iter().map(|e| e.id).collect();

        let items: Vec<ListItem> = state
            .episodes
            .iter()
            .map(|ep| {
                // Dim finished episodes so the unheard ones stand out.
                let title_style = if ep.played {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default().fg(Color::White)
                };
                let dim = dim_style();

                let is_now = playing_id == Some(ep.id);
                let now = if is_now { ">" } else { " " };
                let download_icon = match &ep.download_status {
                    crate::models::DownloadStatus::Downloaded => "v",
                    crate::models::DownloadStatus::Downloading => "|",
                    crate::models::DownloadStatus::Failed => "!",
                    crate::models::DownloadStatus::NotDownloaded => " ",
                };

                // Line 1: state markers (each in its own semantic colour) + title.
                let line1 = Line::from(vec![
                    Span::styled(now, now_marker_style(is_now)),
                    Span::styled(listen_marker(ep), listen_marker_style(ep)),
                    Span::raw(" "),
                    Span::styled(download_icon, download_icon_style(&ep.download_status)),
                    Span::raw(" "),
                    Span::styled(ep.title.clone(), title_style),
                ]);

                // Line 2: newest-first date (blue), duration, and a queue badge
                // (yellow) - fields already in the model, coloured so each card
                // has an anchor rather than another gray line.
                let mut line2_spans = vec![
                    Span::raw("   "),
                    Span::styled(format_relative_time(&ep.published_at), time_style()),
                    Span::styled(" | ", dim),
                    Span::styled(ep.duration_formatted(), dim),
                ];
                if queued.contains(&ep.id) {
                    line2_spans.push(Span::styled(" | ", dim));
                    line2_spans.push(Span::styled(
                        "queued",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ));
                }
                let line2 = Line::from(line2_spans);

                // Line 3: a one-line description snippet, if there is one.
                let mut lines = vec![line1, line2];
                if let Some(desc) = &ep.description {
                    let snippet = description_snippet(desc, 70);
                    if !snippet.is_empty() {
                        lines.push(Line::from(Span::styled(format!("   {snippet}"), dim)));
                    }
                }

                ListItem::new(lines)
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(border))
                    .title(format!(" {} ({}) ", title, state.episodes.len()))
                    .title_style(
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
            )
            .highlight_style(selection_style());

        state.sync_episode_selection();
        f.render_stateful_widget(list, area, &mut state.episode_list_state);
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
                let is_now = playing_id == Some(ep.id);
                let now = if is_now { ">" } else { " " };
                let dim = dim_style();
                ListItem::new(Line::from(vec![
                    Span::styled(now, now_marker_style(is_now)),
                    Span::styled(listen_marker(ep), listen_marker_style(ep)),
                    Span::styled(format!(" {:2}. ", i + 1), dim),
                    Span::styled(ep.title.clone(), Style::default().fg(Color::White)),
                    Span::styled(" | ", dim),
                    Span::styled(ep.duration_formatted(), dim),
                ]))
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

        // In feed-recovery mode the same box is a picker: title it so, and a
        // chosen result re-points the feed rather than adding a subscription.
        let fixing = state.feed_fix_target.is_some();
        let input_title = if fixing {
            " Pick a feed to re-point (iTunes) "
        } else {
            " Search Podcasts (iTunes) "
        };

        // Search input box
        let input = Paragraph::new(state.search_input.as_str())
            .style(Style::default().fg(Color::Yellow))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(input_border))
                    .title(input_title)
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
            let dim = Style::default().fg(Color::DarkGray);
            let items: Vec<ListItem> = state
                .search_results
                .iter()
                .map(|result| {
                    // Card: title, then the metadata that helps tell shows apart -
                    // artist, genre, episode count, and the feed's host (so a
                    // moved feed is recognisable by where it now lives).
                    let mut meta_parts: Vec<String> = Vec::new();
                    if !result.artist.is_empty() {
                        meta_parts.push(result.artist.clone());
                    }
                    if let Some(genre) = &result.genre {
                        meta_parts.push(genre.clone());
                    }
                    if let Some(n) = result.track_count {
                        meta_parts.push(format!("{n} eps"));
                    }
                    if let Some(host) = feed_host(&result.feed_url) {
                        meta_parts.push(host);
                    }
                    let meta = format!("   {}", meta_parts.join(" | "));

                    ListItem::new(vec![
                        Line::from(Span::styled(
                            result.title.clone(),
                            Style::default().fg(Color::White),
                        )),
                        Line::from(Span::styled(meta, dim)),
                    ])
                })
                .collect();

            let results_border = if input_focused {
                Color::White
            } else {
                Color::Cyan
            };
            let action = if fixing {
                "Enter to re-point"
            } else {
                "Enter to subscribe"
            };
            let list = List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(results_border))
                        .title(format!(
                            " Results ({}) - {action} ",
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
        let dim = dim_style();
        let queue_len = state.queue_items.len();
        let up_next_style = if queue_len > 0 {
            Style::default().fg(Color::Green)
        } else {
            dim
        };
        let status = if let Some(notice) = &state.playback_notice {
            // A stream-drop notice takes over the line (sticky, non-blocking, in
            // yellow) until playback recovers or the user resumes.
            Paragraph::new(format!("! {notice}")).style(Style::default().fg(Color::Yellow))
        } else if let Some(episode) = &state.current_episode {
            let (icon, icon_style) = if state.is_playing {
                (
                    ">",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                (
                    "||",
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
            };
            Paragraph::new(Line::from(vec![
                Span::styled(icon, icon_style),
                Span::raw(" "),
                Span::styled(episode.title.clone(), Style::default().fg(Color::White)),
                Span::styled(" | ", dim),
                Span::styled(
                    format!("{:.1}x", state.playback_speed),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(" | ", dim),
                Span::styled(format!("Vol {:.0}%", state.volume * 100.0), dim),
                Span::styled(" | ", dim),
                Span::styled(format!("Up Next: {queue_len}"), up_next_style),
            ]))
        } else {
            Paragraph::new(Line::from(vec![
                Span::styled("No episode playing", dim),
                Span::styled(" | ", dim),
                Span::styled(format!("Up Next: {queue_len}"), up_next_style),
            ]))
        };
        let status = status.alignment(Alignment::Left);

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

        // Contextual key hints: show the actions available in the current view,
        // so features like the auto-queue toggle ([A]) are discoverable without
        // opening Help. A common tail carries the always-available keys.
        let view_hint = match state.current_view {
            View::Subscriptions => {
                "[l/Enter] Episodes  [A] Auto-queue  [O] Order  [S] Seen all  [r] Refresh  [u] Unsub"
            }
            View::Episodes => {
                "[Enter] Play  [h/Esc] Back  [a] Queue  [d] Download  [s] Played  [S] Seen"
            }
            View::Queue => "[Enter] Play  [x] Remove  [n] Skip",
            View::Search if state.feed_fix_target.is_some() => {
                "type to search  [Enter] Re-point feed  [Esc] Cancel"
            }
            View::Search => "type to search  [Enter] Search/Subscribe  [Esc] Back",
            View::Settings => "config via --config <file>",
        };
        let help_text = format!("{view_hint}  |  [?] Help  [q] Quit");

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
            Line::from("  1        Library (feeds | episodes)"),
            Line::from("  2        Queue"),
            Line::from("  3        Search"),
            Line::from("  4        Settings"),
            Line::from("  Tab      Next view"),
            Line::from("  Shift+Tab Previous view"),
            Line::from("  l/Enter  Feeds -> episodes pane"),
            Line::from("  h/Esc    Episodes -> feeds pane"),
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
            Line::from("  r        Refresh feed"),
            Line::from("  R        Refresh all"),
            Line::from("  f        Fix feed (find moved URL)"),
            Line::from("  s        Toggle played"),
            Line::from("  S        Mark seen (episode) / seen all (feed)"),
            Line::from("  A        Cycle auto-queue (off/bottom/top)"),
            Line::from("  O        Toggle order (newest / oldest first)"),
            Line::from("  u        Unsubscribe (subscriptions)"),
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
        let popup_area = centered_rect(70, 30, area);

        let mut confirm_text = vec![
            Line::from(""),
            Line::from(Span::styled(
                "⚠️  Confirm",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];
        // The message may be multi-line (e.g. a feed re-point shows the old ->
        // new URL); render each line rather than collapsing them.
        for line in message.lines() {
            confirm_text.push(Line::from(line.to_string()));
        }
        confirm_text.push(Line::from(""));
        confirm_text.push(Line::from(Span::styled(
            "Press Enter to confirm, Esc to cancel",
            Style::default().fg(Color::Gray),
        )));

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

#[cfg(test)]
mod tests {
    use super::{description_snippet, feed_host};

    #[test]
    fn feed_host_extracts_the_domain() {
        assert_eq!(
            feed_host("https://feeds.simplecast.com/H1YsStlE").as_deref(),
            Some("feeds.simplecast.com")
        );
        assert_eq!(
            feed_host("http://rss.buzzsprout.com:8080/2506785.rss").as_deref(),
            Some("rss.buzzsprout.com")
        );
        assert_eq!(feed_host("").as_deref(), None);
    }

    #[test]
    fn snippet_strips_tags_and_collapses_whitespace() {
        let html = "<p>Hello   <b>there</b>,\n  world</p>";
        assert_eq!(description_snippet(html, 100), "Hello there, world");
    }

    #[test]
    fn snippet_truncates_with_ellipsis() {
        let long = "abcdefghijklmnopqrstuvwxyz";
        let out = description_snippet(long, 10);
        // At most `max` display columns, ending in a single ellipsis glyph.
        assert_eq!(out.chars().count(), 10);
        assert!(out.ends_with('\u{2026}'));
    }

    #[test]
    fn snippet_empty_for_tags_only() {
        assert_eq!(description_snippet("<br/><hr/>", 50), "");
    }
}
