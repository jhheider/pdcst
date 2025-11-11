use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ArtworkProtocol {
    Sixel,
    Kitty,
    ITerm2,
    None,
}

impl ArtworkProtocol {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Sixel => "sixel",
            Self::Kitty => "kitty",
            Self::ITerm2 => "iterm2",
            Self::None => "none",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "sixel" => Self::Sixel,
            "kitty" => Self::Kitty,
            "iterm2" => Self::ITerm2,
            _ => Self::None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    #[serde(default)]
    pub colors: HashMap<String, String>,
}

impl Default for Theme {
    fn default() -> Self {
        let mut colors = HashMap::new();
        colors.insert("foreground".to_string(), "#ffffff".to_string());
        colors.insert("background".to_string(), "#000000".to_string());
        colors.insert("accent".to_string(), "#00ff00".to_string());

        Self {
            name: "default".to_string(),
            colors,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyBindings {
    #[serde(default)]
    pub bindings: HashMap<String, String>,
}

impl Default for KeyBindings {
    fn default() -> Self {
        let mut bindings = HashMap::new();
        bindings.insert("play_pause".to_string(), " ".to_string()); // Space
        bindings.insert("quit".to_string(), "q".to_string());
        bindings.insert("skip_forward".to_string(), "l".to_string());
        bindings.insert("skip_backward".to_string(), "h".to_string());
        bindings.insert("volume_up".to_string(), "+".to_string());
        bindings.insert("volume_down".to_string(), "-".to_string());
        bindings.insert("speed_up".to_string(), "]".to_string());
        bindings.insert("speed_down".to_string(), "[".to_string());

        Self { bindings }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    // Storage
    pub data_dir: PathBuf,
    pub download_dir: PathBuf,
    pub artwork_dir: PathBuf,
    pub log_dir: PathBuf,

    // Playback
    #[serde(default = "default_playback_rate")]
    pub default_playback_rate: f32,
    #[serde(default = "default_skip_forward")]
    pub skip_forward_seconds: u64,
    #[serde(default = "default_skip_backward")]
    pub skip_backward_seconds: u64,
    #[serde(default)]
    pub trim_silence: bool,

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

    // UI
    #[serde(default)]
    pub theme: Theme,
    #[serde(default)]
    pub keybindings: KeyBindings,
    #[serde(default = "default_true")]
    pub show_artwork: bool,
    pub artwork_protocol: Option<ArtworkProtocol>,
}

fn default_playback_rate() -> f32 {
    1.0
}

fn default_skip_forward() -> u64 {
    30
}

fn default_skip_backward() -> u64 {
    10
}

fn default_save_position_interval() -> u64 {
    10
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
            .join("podcast-tui");
        let download_dir = dirs::download_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Podcasts");
        let artwork_dir = data_dir.join("artwork");
        let log_dir = data_dir.join("logs");

        Self {
            data_dir,
            download_dir,
            artwork_dir,
            log_dir,
            default_playback_rate: default_playback_rate(),
            skip_forward_seconds: default_skip_forward(),
            skip_backward_seconds: default_skip_backward(),
            trim_silence: false,
            save_position_interval_seconds: default_save_position_interval(),
            auto_refresh_interval_minutes: default_auto_refresh(),
            max_concurrent_refreshes: default_concurrent_refreshes(),
            max_concurrent_downloads: default_concurrent_downloads(),
            theme: Theme::default(),
            keybindings: KeyBindings::default(),
            show_artwork: true,
            artwork_protocol: None, // Auto-detect
        }
    }
}
