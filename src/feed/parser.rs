use crate::models::{Episode, Subscription};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rss::Channel;

pub struct FeedParser;

impl FeedParser {
    pub fn parse_channel(rss_content: &str) -> Result<Channel> {
        Channel::read_from(rss_content.as_bytes()).context("Failed to parse RSS feed")
    }

    pub fn subscription_from_channel(rss_url: String, channel: &Channel) -> Subscription {
        let mut sub = Subscription::new(channel.title().to_string(), rss_url);

        sub.description = Some(channel.description().to_string());
        sub.author = channel
            .itunes_ext()
            .and_then(|itunes| itunes.author())
            .map(|s| s.to_string());
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
        let title = item.title()?.to_string();
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

        episode.description = item.description().map(|s| s.to_string());

        if let Some(enclosure) = item.enclosure() {
            episode.file_type = Some(enclosure.mime_type().to_string());
            if let Ok(size) = enclosure.length().parse::<i64>() {
                episode.file_size_bytes = Some(size);
            }
        }

        // Try to get duration from iTunes extension
        if let Some(itunes) = item.itunes_ext() {
            if let Some(duration_str) = itunes.duration() {
                episode.duration_seconds = Self::parse_duration(duration_str);
            }
        }

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
}
