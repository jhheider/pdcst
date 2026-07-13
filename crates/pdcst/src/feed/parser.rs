use crate::models::{Episode, Subscription};
use crate::utils::text::clean_feed_text;
use anyhow::{Context, Result};
use atom_syndication::Feed;
use chrono::{DateTime, Utc};
use rss::Channel;

pub struct FeedParser;

impl FeedParser {
    pub fn parse_channel(rss_content: &str) -> Result<Channel> {
        Channel::read_from(rss_content.as_bytes()).context("Failed to parse RSS feed")
    }

    /// Parse a feed's episodes, trying strict RSS 2.0 first and falling back to
    /// Atom. Most podcast feeds are RSS, but some are Atom outright and a few
    /// have quirks a strict RSS parse rejects - the fallback keeps those from
    /// failing wholesale. If neither format fits, the error names both failures
    /// so the subscription row can show *why* (see the per-feed error surfacing).
    pub fn parse_episodes(subscription_id: uuid::Uuid, content: &str) -> Result<Vec<Episode>> {
        match Channel::read_from(content.as_bytes()) {
            Ok(channel) => Ok(Self::episodes_from_channel(subscription_id, &channel)),
            Err(rss_err) => match Feed::read_from(content.as_bytes()) {
                Ok(feed) => Ok(Self::episodes_from_atom(subscription_id, &feed)),
                Err(atom_err) => Err(anyhow::anyhow!(
                    "not valid RSS ({rss_err}) or Atom ({atom_err})"
                )),
            },
        }
    }

    pub fn subscription_from_channel(rss_url: String, channel: &Channel) -> Subscription {
        // Feed text carries HTML entities the XML layer does not resolve and, in
        // titles/notes, ZWJ emoji that break terminal width math. Normalize once
        // here so the stored title/description/author are clean, render-safe UTF-8.
        let mut sub = Subscription::new(clean_feed_text(channel.title()), rss_url);

        sub.description = Some(clean_feed_text(channel.description()));
        sub.author = channel
            .itunes_ext()
            .and_then(|itunes| itunes.author())
            .map(clean_feed_text);
        sub.website_url = Some(channel.link().to_string());
        sub.artwork_url = channel
            .image()
            .map(|img| img.url().to_string())
            .or_else(|| {
                channel
                    .itunes_ext()
                    .and_then(|itunes| itunes.image())
                    .map(|s| s.to_string())
            });

        if let Some(itunes) = channel.itunes_ext() {
            let categories = itunes.categories();
            sub.categories = categories
                .iter()
                .map(|cat| cat.text().to_string())
                .collect();
        }

        sub
    }

    pub fn episodes_from_channel(subscription_id: uuid::Uuid, channel: &Channel) -> Vec<Episode> {
        channel
            .items()
            .iter()
            .filter_map(|item| Self::episode_from_item(subscription_id, item))
            .collect()
    }

    fn episode_from_item(subscription_id: uuid::Uuid, item: &rss::Item) -> Option<Episode> {
        let title = clean_feed_text(item.title()?);
        let url = item.enclosure()?.url().to_string();
        let guid = item
            .guid()
            .map(|g| g.value().to_string())
            .unwrap_or_else(|| url.clone());

        let published_at = item
            .pub_date()
            .and_then(|date_str| {
                // Try parsing RFC 2822 format first
                DateTime::parse_from_rfc2822(date_str)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            })
            .unwrap_or_else(Utc::now);

        let mut episode = Episode::new(subscription_id, title, url, guid, published_at);

        episode.description = item.description().map(clean_feed_text);

        if let Some(enclosure) = item.enclosure() {
            episode.file_type = Some(enclosure.mime_type().to_string());
            if let Ok(size) = enclosure.length().parse::<i64>() {
                episode.file_size_bytes = Some(size);
            }
        }

        // Try to get duration from iTunes extension
        if let Some(itunes) = item.itunes_ext()
            && let Some(duration_str) = itunes.duration()
        {
            episode.duration_seconds = Self::parse_duration(duration_str);
        }

        Some(episode)
    }

    fn episodes_from_atom(subscription_id: uuid::Uuid, feed: &Feed) -> Vec<Episode> {
        feed.entries()
            .iter()
            .filter_map(|entry| Self::episode_from_atom_entry(subscription_id, entry))
            .collect()
    }

    fn episode_from_atom_entry(
        subscription_id: uuid::Uuid,
        entry: &atom_syndication::Entry,
    ) -> Option<Episode> {
        // The audio lives on a `<link rel="enclosure">`; an entry without one is
        // not a playable episode (e.g. a text-only post), so skip it.
        let enclosure = entry.links().iter().find(|l| l.rel() == "enclosure")?;
        let url = enclosure.href().to_string();
        if url.is_empty() {
            return None;
        }

        let title = clean_feed_text(&entry.title().value);
        let guid = if entry.id().is_empty() {
            url.clone()
        } else {
            entry.id().to_string()
        };
        // Atom carries RFC 3339 timestamps already parsed by the crate. Prefer
        // <published>, fall back to the required <updated>.
        let published_at = entry
            .published()
            .copied()
            .unwrap_or_else(|| *entry.updated())
            .with_timezone(&Utc);

        let mut episode = Episode::new(subscription_id, title, url, guid, published_at);
        episode.description = entry
            .summary()
            .map(|t| clean_feed_text(&t.value))
            .or_else(|| entry.content().and_then(|c| c.value().map(clean_feed_text)));
        episode.file_type = enclosure.mime_type().map(String::from);
        episode.file_size_bytes = enclosure.length().and_then(|l| l.parse::<i64>().ok());

        Some(episode)
    }

    fn parse_duration(duration_str: &str) -> Option<i64> {
        // Duration can be in formats: "HH:MM:SS", "MM:SS", or just seconds as a number
        let parts: Vec<&str> = duration_str.split(':').collect();

        match parts.len() {
            1 => {
                // Just seconds
                duration_str.parse::<i64>().ok()
            }
            2 => {
                // MM:SS
                let minutes = parts[0].parse::<i64>().ok()?;
                let seconds = parts[1].parse::<i64>().ok()?;
                Some(minutes * 60 + seconds)
            }
            3 => {
                // HH:MM:SS
                let hours = parts[0].parse::<i64>().ok()?;
                let minutes = parts[1].parse::<i64>().ok()?;
                let seconds = parts[2].parse::<i64>().ok()?;
                Some(hours * 3600 + minutes * 60 + seconds)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_duration() {
        assert_eq!(FeedParser::parse_duration("3661"), Some(3661));
        assert_eq!(FeedParser::parse_duration("45:30"), Some(2730));
        assert_eq!(FeedParser::parse_duration("1:30:45"), Some(5445));
    }

    const RSS: &str = r#"<?xml version="1.0"?>
        <rss version="2.0"><channel><title>Show</title><description>d</description>
        <link>https://example.com</link>
        <item>
            <title>Ep 1</title>
            <guid>guid-1</guid>
            <enclosure url="https://example.com/1.mp3" length="123" type="audio/mpeg"/>
        </item>
        </channel></rss>"#;

    // A minimal Atom feed with a podcast-style enclosure link.
    const ATOM: &str = r#"<?xml version="1.0" encoding="utf-8"?>
        <feed xmlns="http://www.w3.org/2005/Atom">
          <title>Atom Show</title>
          <id>urn:show</id>
          <updated>2026-07-10T10:00:00Z</updated>
          <entry>
            <title>Atom Ep 1</title>
            <id>urn:ep:1</id>
            <updated>2026-07-10T10:00:00Z</updated>
            <published>2026-07-10T10:00:00Z</published>
            <summary>An atom episode.</summary>
            <link rel="enclosure" type="audio/mpeg" length="456"
                  href="https://example.com/atom1.mp3"/>
          </entry>
        </feed>"#;

    #[test]
    fn parse_episodes_reads_strict_rss() {
        let sub = uuid::Uuid::new_v4();
        let eps = FeedParser::parse_episodes(sub, RSS).unwrap();
        assert_eq!(eps.len(), 1);
        assert_eq!(eps[0].title, "Ep 1");
        assert_eq!(eps[0].guid, "guid-1");
        assert_eq!(eps[0].url, "https://example.com/1.mp3");
    }

    #[test]
    fn parse_episodes_falls_back_to_atom() {
        let sub = uuid::Uuid::new_v4();
        let eps = FeedParser::parse_episodes(sub, ATOM).unwrap();
        assert_eq!(eps.len(), 1, "the atom entry with an enclosure is parsed");
        assert_eq!(eps[0].title, "Atom Ep 1");
        assert_eq!(eps[0].guid, "urn:ep:1");
        assert_eq!(eps[0].url, "https://example.com/atom1.mp3");
        assert_eq!(eps[0].file_size_bytes, Some(456));
        assert_eq!(eps[0].file_type.as_deref(), Some("audio/mpeg"));
        assert_eq!(eps[0].description.as_deref(), Some("An atom episode."));
    }

    #[test]
    fn ingest_normalizes_entities_and_emoji_in_title_and_description() {
        // A ZWJ rainbow flag (flag + VS16 + ZWJ + rainbow) plus HTML entities,
        // injected via escapes since a raw string would not interpret `\u{}`.
        let flag = "\u{1F3F3}\u{FE0F}\u{200D}\u{1F308}";
        let rss = format!(
            r#"<?xml version="1.0"?>
            <rss version="2.0"><channel><title>Q&amp;A Show</title><description>d</description>
            <link>https://example.com</link>
            <item>
                <title>It&#8217;s here &amp; now {flag}</title>
                <description>Ben &amp; Jerry&#x2019;s &mdash; a review</description>
                <guid>guid-1</guid>
                <enclosure url="https://example.com/1.mp3" length="123" type="audio/mpeg"/>
            </item>
            </channel></rss>"#
        );

        let sub_id = uuid::Uuid::new_v4();
        let eps = FeedParser::parse_episodes(sub_id, &rss).unwrap();
        // Entities decoded; the ZWJ flag's joiners stripped to width-honest bases.
        assert_eq!(eps[0].title, "It\u{2019}s here & now \u{1F3F3}\u{1F308}");
        assert_eq!(
            eps[0].description.as_deref(),
            Some("Ben & Jerry\u{2019}s \u{2014} a review")
        );

        let channel = FeedParser::parse_channel(&rss).unwrap();
        let sub = FeedParser::subscription_from_channel("https://x/feed".into(), &channel);
        assert_eq!(sub.title, "Q&A Show");
    }

    #[test]
    fn parse_episodes_errors_on_neither_format() {
        let sub = uuid::Uuid::new_v4();
        let err = FeedParser::parse_episodes(sub, "not xml at all")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("RSS") && err.contains("Atom"),
            "names both: {err}"
        );
    }
}
