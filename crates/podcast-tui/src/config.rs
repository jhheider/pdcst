use crate::models::Config;
use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

impl Config {
    pub fn load_default() -> Result<Self> {
        let config = Config::default();

        // Create directories if they don't exist
        fs::create_dir_all(&config.data_dir).context("Failed to create data directory")?;
        fs::create_dir_all(&config.download_dir).context("Failed to create download directory")?;
        fs::create_dir_all(&config.artwork_dir).context("Failed to create artwork directory")?;
        fs::create_dir_all(&config.log_dir).context("Failed to create log directory")?;

        Ok(config)
    }

    pub fn load_from_file(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;

        let config: Config = toml::from_str(&contents).context("Failed to parse config file")?;

        // Create directories if they don't exist
        fs::create_dir_all(&config.data_dir)?;
        fs::create_dir_all(&config.download_dir)?;
        fs::create_dir_all(&config.artwork_dir)?;
        fs::create_dir_all(&config.log_dir)?;

        Ok(config)
    }

    pub fn save_to_file(&self, path: &Path) -> Result<()> {
        let contents = toml::to_string_pretty(self).context("Failed to serialize config")?;

        fs::write(path, contents)
            .with_context(|| format!("Failed to write config file: {}", path.display()))?;

        Ok(())
    }

    pub fn database_path(&self) -> std::path::PathBuf {
        self.data_dir.join("podcast.db")
    }
}
