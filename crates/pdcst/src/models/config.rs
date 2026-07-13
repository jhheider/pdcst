use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    // Storage
    pub data_dir: PathBuf,
    pub download_dir: PathBuf,
    pub log_dir: PathBuf,

    // Playback
    #[serde(default = "default_playback_rate")]
    pub default_playback_rate: f32,

    // Cache retention (keeps on-disk audio from growing unbounded). A cap of 0
    // means unlimited. Enforced on startup and periodically while running.
    #[serde(default = "default_true")]
    pub delete_on_finish: bool,
    #[serde(default = "default_max_cache_episodes")]
    pub max_cache_episodes: usize,
    #[serde(default = "default_max_cache_megabytes")]
    pub max_cache_megabytes: u64,

    // Auto-queue (Phase C). Global defaults; per-subscription `auto_queue` /
    // `auto_queue_to_top` override the direction.
    #[serde(default = "default_queue_max_depth")]
    pub queue_max_depth: usize,
    /// Default auto-add direction for feeds that do not override it: true = top.
    #[serde(default)]
    pub auto_queue_to_top_default: bool,
    /// Avoid two adjacent episodes of the same podcast when auto-filling.
    #[serde(default = "default_true")]
    pub smart_interleave: bool,

    // Position saving
    #[serde(default = "default_save_position_interval")]
    pub save_position_interval_seconds: u64,

    // Sync
    #[serde(default = "default_auto_refresh")]
    pub auto_refresh_interval_minutes: u64,
    #[serde(default = "default_concurrent_refreshes")]
    pub max_concurrent_refreshes: usize,
    #[serde(default = "default_concurrent_downloads")]
    pub max_concurrent_downloads: usize,
}

fn default_playback_rate() -> f32 {
    1.0
}

fn default_save_position_interval() -> u64 {
    10
}

fn default_max_cache_episodes() -> usize {
    50
}

fn default_max_cache_megabytes() -> u64 {
    4096
}

fn default_queue_max_depth() -> usize {
    20
}

fn default_auto_refresh() -> u64 {
    60
}

fn default_concurrent_refreshes() -> usize {
    5
}

fn default_concurrent_downloads() -> usize {
    3
}

fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        let data_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("pdcst");
        let download_dir = dirs::download_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Podcasts");
        let log_dir = data_dir.join("logs");

        Self {
            data_dir,
            download_dir,
            log_dir,
            default_playback_rate: default_playback_rate(),
            delete_on_finish: true,
            max_cache_episodes: default_max_cache_episodes(),
            max_cache_megabytes: default_max_cache_megabytes(),
            queue_max_depth: default_queue_max_depth(),
            auto_queue_to_top_default: false,
            smart_interleave: true,
            save_position_interval_seconds: default_save_position_interval(),
            auto_refresh_interval_minutes: default_auto_refresh(),
            max_concurrent_refreshes: default_concurrent_refreshes(),
            max_concurrent_downloads: default_concurrent_downloads(),
        }
    }
}
