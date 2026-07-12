use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// Result from a podcast search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub artist: String,
    pub feed_url: String,
    pub artwork_url: Option<String>,
    pub description: Option<String>,
    pub genre: Option<String>,
    pub track_count: Option<i32>,
}

/// Response from iTunes Search API
#[derive(Debug, Deserialize)]
struct ItunesSearchResponse {
    #[serde(rename = "resultCount")]
    result_count: i32,
    results: Vec<ItunesResult>,
}

#[derive(Debug, Deserialize)]
struct ItunesResult {
    #[serde(rename = "collectionName")]
    collection_name: Option<String>,
    #[serde(rename = "trackName")]
    track_name: Option<String>,
    #[serde(rename = "artistName")]
    artist_name: Option<String>,
    #[serde(rename = "feedUrl")]
    feed_url: Option<String>,
    #[serde(rename = "artworkUrl600")]
    artwork_url_600: Option<String>,
    #[serde(rename = "artworkUrl100")]
    artwork_url_100: Option<String>,
    #[serde(rename = "primaryGenreName")]
    primary_genre_name: Option<String>,
    #[serde(rename = "trackCount")]
    track_count: Option<i32>,
    #[serde(rename = "collectionCensoredName")]
    collection_censored_name: Option<String>,
}

/// Podcast search client using iTunes Search API
pub struct PodcastSearch {
    client: reqwest::Client,
}

impl PodcastSearch {
    pub fn new() -> Self {
        Self {
            client: {
                crate::ensure_crypto_provider();
                reqwest::Client::new()
            },
        }
    }

    /// Search for podcasts using the iTunes Search API
    ///
    /// Returns a list of matching podcasts with their feed URLs.
    pub async fn search(&self, query: &str) -> Result<Vec<SearchResult>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }

        tracing::debug!("Searching for podcasts: {}", query);

        // Build iTunes Search API URL
        let url = format!(
            "https://itunes.apple.com/search?term={}&media=podcast&entity=podcast&limit=50",
            urlencoding::encode(query)
        );

        // Make HTTP request
        let response = self
            .client
            .get(&url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .context("Failed to send search request")?;

        if !response.status().is_success() {
            anyhow::bail!("Search request failed with status: {}", response.status());
        }

        // Parse JSON response
        let search_response: ItunesSearchResponse = response
            .json()
            .await
            .context("Failed to parse search response")?;

        tracing::info!(
            "Found {} results for query: {}",
            search_response.result_count,
            query
        );

        // Convert to SearchResult
        let results = search_response
            .results
            .into_iter()
            .filter_map(|r| self.convert_itunes_result(r))
            .collect();

        Ok(results)
    }

    fn convert_itunes_result(&self, result: ItunesResult) -> Option<SearchResult> {
        // Must have a feed URL
        let feed_url = result.feed_url?;

        // Get title (prefer collectionName, fallback to trackName)
        let title = result
            .collection_name
            .or(result.track_name)
            .or(result.collection_censored_name)?;

        // Artist name is usually available
        let artist = result.artist_name.unwrap_or_else(|| "Unknown".to_string());

        // Prefer high-res artwork
        let artwork_url = result.artwork_url_600.or(result.artwork_url_100);

        Some(SearchResult {
            title,
            artist,
            feed_url,
            artwork_url,
            description: None, // iTunes API doesn't provide full descriptions
            genre: result.primary_genre_name,
            track_count: result.track_count,
        })
    }
}

impl Default for PodcastSearch {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_itunes_result_valid() {
        let search = PodcastSearch::new();
        let itunes_result = ItunesResult {
            collection_name: Some("Test Podcast".to_string()),
            track_name: None,
            artist_name: Some("Test Artist".to_string()),
            feed_url: Some("https://example.com/feed.xml".to_string()),
            artwork_url_600: Some("https://example.com/art600.jpg".to_string()),
            artwork_url_100: Some("https://example.com/art100.jpg".to_string()),
            primary_genre_name: Some("Technology".to_string()),
            track_count: Some(100),
            collection_censored_name: None,
        };

        let result = search.convert_itunes_result(itunes_result);
        assert!(result.is_some());

        let result = result.unwrap();
        assert_eq!(result.title, "Test Podcast");
        assert_eq!(result.artist, "Test Artist");
        assert_eq!(result.feed_url, "https://example.com/feed.xml");
        assert_eq!(
            result.artwork_url,
            Some("https://example.com/art600.jpg".to_string())
        );
        assert_eq!(result.genre, Some("Technology".to_string()));
        assert_eq!(result.track_count, Some(100));
    }

    #[test]
    fn test_convert_itunes_result_missing_feed_url() {
        let search = PodcastSearch::new();
        let itunes_result = ItunesResult {
            collection_name: Some("Test Podcast".to_string()),
            track_name: None,
            artist_name: Some("Test Artist".to_string()),
            feed_url: None, // Missing feed URL
            artwork_url_600: None,
            artwork_url_100: None,
            primary_genre_name: None,
            track_count: None,
            collection_censored_name: None,
        };

        let result = search.convert_itunes_result(itunes_result);
        assert!(result.is_none(), "Should return None when feed_url is missing");
    }

    #[test]
    fn test_convert_itunes_result_fallback_title() {
        let search = PodcastSearch::new();
        let itunes_result = ItunesResult {
            collection_name: None,
            track_name: Some("Track Name".to_string()),
            artist_name: Some("Test Artist".to_string()),
            feed_url: Some("https://example.com/feed.xml".to_string()),
            artwork_url_600: None,
            artwork_url_100: None,
            primary_genre_name: None,
            track_count: None,
            collection_censored_name: None,
        };

        let result = search.convert_itunes_result(itunes_result);
        assert!(result.is_some());
        assert_eq!(result.unwrap().title, "Track Name");
    }

    #[test]
    fn test_convert_itunes_result_artwork_fallback() {
        let search = PodcastSearch::new();
        let itunes_result = ItunesResult {
            collection_name: Some("Test".to_string()),
            track_name: None,
            artist_name: Some("Artist".to_string()),
            feed_url: Some("https://example.com/feed.xml".to_string()),
            artwork_url_600: None,
            artwork_url_100: Some("https://example.com/art100.jpg".to_string()),
            primary_genre_name: None,
            track_count: None,
            collection_censored_name: None,
        };

        let result = search.convert_itunes_result(itunes_result);
        assert!(result.is_some());
        assert_eq!(
            result.unwrap().artwork_url,
            Some("https://example.com/art100.jpg".to_string())
        );
    }

    #[tokio::test]
    async fn test_search_empty_query() {
        let search = PodcastSearch::new();
        let results = search.search("").await.unwrap();
        assert_eq!(results.len(), 0, "Empty query should return no results");
    }

    // Note: This test makes a real network request to iTunes API
    // Comment out if running in CI without network access
    #[tokio::test]
    #[ignore] // Use `cargo test -- --ignored` to run
    async fn test_search_integration() {
        let search = PodcastSearch::new();
        let results = search.search("rust programming").await.unwrap();

        // Should find at least a few Rust-related podcasts
        assert!(!results.is_empty(), "Should find some podcasts");

        // Verify structure
        for result in results.iter().take(3) {
            assert!(!result.title.is_empty());
            assert!(!result.feed_url.is_empty());
            assert!(result.feed_url.starts_with("http"));
        }
    }
}
