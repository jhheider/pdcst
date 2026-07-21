use crate::models::Config;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// A commented config written to the default path on first run, so every tunable
/// is discoverable and editable without hunting the source. Every value is
/// optional and shown at its default; the whole thing parses to `Config::default`
/// while fully commented. Storage paths are omitted (they default per platform).
const DEFAULT_CONFIG_TEMPLATE: &str = r#"# pdcst configuration.
#
# Written automatically on first run. Every setting below is OPTIONAL and shown
# at its default; uncomment and edit any you want to change, or delete this file
# to regenerate it. `pdcst --print-config` shows the effective values and this
# file's path.

# --- Playback ---
# default_playback_rate = 1.0

# --- Up Next auto-queue (the core feature) ---
# queue_max_depth = 20               # how many episodes to keep queued
# auto_queue_to_top_default = false  # new episodes go to the top (true) or bottom (false)
# smart_interleave = true            # avoid two adjacent episodes of the same show

# --- Refresh ---
# auto_refresh_interval_minutes = 60 # 0 disables the periodic refresh (launch refresh still runs)
# max_concurrent_refreshes = 5
# max_concurrent_downloads = 3

# --- On-disk cache retention ---
# delete_on_finish = true
# max_cache_episodes = 50            # 0 = unlimited
# max_cache_megabytes = 4096         # 0 = unlimited

# --- Resume ---
# save_position_interval_seconds = 10

# --- Storage paths (advanced; default to your platform's data/download dirs) ---
# data_dir = "/path/to/data"
# download_dir = "/path/to/downloads"
# log_dir = "/path/to/logs"
"#;

impl Config {
    /// The default config file location, `<platform config dir>/pdcst/config.toml`
    /// (e.g. `~/.config/pdcst/config.toml` on Linux). Read on startup when
    /// `--config` is not given.
    pub fn default_config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("pdcst")
            .join("config.toml")
    }

    /// Load config for a normal launch: read the default config file if it
    /// exists, otherwise write a commented template there (so the settings are
    /// discoverable) and use the built-in defaults. A malformed or unreadable
    /// existing file is surfaced rather than silently ignored.
    pub fn load_default() -> Result<Self> {
        let path = Self::default_config_path();
        let config = if path.exists() {
            tracing::info!("loading config from {}", path.display());
            Self::load_from_file(&path)?
        } else {
            let config = Config::default();
            write_default_template(&path);
            config.ensure_dirs()?;
            config
        };
        Ok(config)
    }

    /// Load config from an explicit `path` (the `--config` flag). Missing fields
    /// fall back to defaults, so a partial file is valid.
    pub fn load_from_file(path: &Path) -> Result<Self> {
        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;

        let config: Config = toml::from_str(&contents).context("Failed to parse config file")?;
        config.ensure_dirs()?;
        Ok(config)
    }

    pub fn save_to_file(&self, path: &Path) -> Result<()> {
        let contents = toml::to_string_pretty(self).context("Failed to serialize config")?;

        fs::write(path, contents)
            .with_context(|| format!("Failed to write config file: {}", path.display()))?;

        Ok(())
    }

    /// Create the storage directories this config points at. Idempotent.
    fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(&self.data_dir).context("Failed to create data directory")?;
        fs::create_dir_all(&self.download_dir).context("Failed to create download directory")?;
        fs::create_dir_all(&self.log_dir).context("Failed to create log directory")?;
        Ok(())
    }

    pub fn database_path(&self) -> std::path::PathBuf {
        self.data_dir.join("podcast.db")
    }

    /// Directory for temp files backing progressive stream-to-disk playback.
    /// Created on demand; entries are purged as episodes change.
    pub fn stream_cache_dir(&self) -> std::path::PathBuf {
        self.data_dir.join("stream-cache")
    }
}

/// Best-effort: write the commented default template to `path` (creating its
/// parent). A failure here is not fatal (the app runs on built-in defaults), so
/// it is logged, not propagated.
fn write_default_template(path: &Path) {
    if let Some(parent) = path.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        tracing::warn!("could not create config dir {}: {e}", parent.display());
        return;
    }
    match fs::write(path, DEFAULT_CONFIG_TEMPLATE) {
        Ok(()) => tracing::info!("wrote a default config to {}", path.display()),
        Err(e) => tracing::warn!("could not write default config to {}: {e}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_path_is_under_pdcst() {
        let path = Config::default_config_path();
        assert!(
            path.ends_with("pdcst/config.toml"),
            "got {}",
            path.display()
        );
    }

    #[test]
    fn partial_config_fills_defaults() {
        // Only one knob set; everything else must fall back to defaults.
        let toml = "queue_max_depth = 42\n";
        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.queue_max_depth, 42);
        assert_eq!(config.max_concurrent_downloads, 3, "unspecified -> default");
        assert!(!config.data_dir.as_os_str().is_empty(), "path defaulted");
    }

    #[test]
    fn default_template_parses_to_defaults() {
        // Fully commented: parses to the built-in defaults, so the file shipped on
        // first run never changes behavior until a user edits it.
        let config: Config = toml::from_str(DEFAULT_CONFIG_TEMPLATE).unwrap();
        let default = Config::default();
        assert_eq!(config.queue_max_depth, default.queue_max_depth);
        assert_eq!(config.smart_interleave, default.smart_interleave);
        assert_eq!(config.delete_on_finish, default.delete_on_finish);
        assert_eq!(
            config.auto_refresh_interval_minutes,
            default.auto_refresh_interval_minutes
        );
    }

    #[test]
    fn load_from_file_round_trips_a_saved_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = Config {
            data_dir: dir.path().join("data"),
            download_dir: dir.path().join("dl"),
            log_dir: dir.path().join("logs"),
            ..Config::default()
        };
        config.queue_max_depth = 7;
        config.save_to_file(&path).unwrap();

        let loaded = Config::load_from_file(&path).unwrap();
        assert_eq!(loaded.queue_max_depth, 7);
        assert!(dir.path().join("data").exists(), "ensure_dirs ran");
    }
}
