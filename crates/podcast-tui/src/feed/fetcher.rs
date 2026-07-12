use anyhow::{Context, Result};
use reqwest::Client;

pub struct FeedFetcher {
    pub(crate) client: Client,
}

impl FeedFetcher {
    pub fn new() -> Self {
        crate::ensure_crypto_provider();
        let client = Client::builder()
            .user_agent("podcast-tui/1.0")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self { client }
    }

    pub async fn fetch_feed(&self, url: &str) -> Result<String> {
        tracing::debug!("Fetching feed from: {}", url);

        let response = self
            .client
            .get(url)
            .send()
            .await
            .with_context(|| format!("Failed to fetch feed from: {}", url))?;

        if !response.status().is_success() {
            anyhow::bail!("HTTP error {}: {}", response.status(), url);
        }

        let content = response
            .text()
            .await
            .context("Failed to read feed content")?;

        Ok(content)
    }
}

impl Default for FeedFetcher {
    fn default() -> Self {
        Self::new()
    }
}
