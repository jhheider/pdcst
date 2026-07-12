use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};

const ITUNES_SEARCH_API: &str = "https://itunes.apple.com/search";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PodcastSearchResult {
    pub title: String,
    pub author: String,
    pub feed_url: String,
    pub artwork_url: Option<String>,
    pub description: Option<String>,
    pub genres: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ITunesResponse {
    #[serde(rename = "resultCount")]
    result_count: i32,
    results: Vec<ITunesResult>,
}

#[derive(Debug, Deserialize)]
struct ITunesResult {
    #[serde(rename = "collectionName")]
    collection_name: Option<String>,
    #[serde(rename = "artistName")]
    artist_name: Option<String>,
    #[serde(rename = "feedUrl")]
    feed_url: Option<String>,
    #[serde(rename = "artworkUrl600")]
    artwork_url_600: Option<String>,
    #[serde(rename = "artworkUrl100")]
    artwork_url_100: Option<String>,
    genres: Option<Vec<String>>,
}

pub struct ITunesSearch {
    client: Client,
}

impl ITunesSearch {
    pub fn new() -> Self {
        crate::ensure_crypto_provider();
        let client = Client::builder()
            .user_agent("podcast-tui/1.0")
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .expect("Failed to create HTTP client");

        Self { client }
    }

    pub async fn search(
        &self,
        query: &str,
        limit: Option<u32>,
    ) -> Result<Vec<PodcastSearchResult>> {
        let limit = limit.unwrap_or(25);

        tracing::info!("Searching iTunes for: {}", query);

        let response = self
            .client
            .get(ITUNES_SEARCH_API)
            .query(&[
                ("term", query),
                ("media", "podcast"),
                ("limit", &limit.to_string()),
            ])
            .send()
            .await
            .context("Failed to search iTunes API")?;

        if !response.status().is_success() {
            anyhow::bail!("iTunes API error: {}", response.status());
        }

        let itunes_response: ITunesResponse = response
            .json()
            .await
            .context("Failed to parse iTunes API response")?;

        tracing::info!(
            "Found {} results for '{}'",
            itunes_response.result_count,
            query
        );

        let results = itunes_response
            .results
            .into_iter()
            .filter_map(|result| {
                // Only include results with a feed URL
                let feed_url = result.feed_url?;

                Some(PodcastSearchResult {
                    title: result
                        .collection_name
                        .unwrap_or_else(|| "Unknown".to_string()),
                    author: result.artist_name.unwrap_or_else(|| "Unknown".to_string()),
                    feed_url,
                    artwork_url: result.artwork_url_600.or(result.artwork_url_100),
                    description: None, // iTunes API doesn't provide descriptions
                    genres: result.genres.unwrap_or_default(),
                })
            })
            .collect();

        Ok(results)
    }
}

impl Default for ITunesSearch {
    fn default() -> Self {
        Self::new()
    }
}
