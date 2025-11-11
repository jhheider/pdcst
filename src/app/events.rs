use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone)]
pub enum AppEvent {
    Key(KeyEvent),
    PlayPause,
    Stop,
    NextTrack,
    PreviousTrack,
    VolumeUp,
    VolumeDown,
    SpeedUp,
    SpeedDown,
    Quit,
}

impl AppEvent {
    pub fn from_key_event(key: KeyEvent) -> Option<Self> {
        match (key.code, key.modifiers) {
            (KeyCode::Char('q'), KeyModifiers::NONE) => Some(Self::Quit),
            (KeyCode::Char(' '), KeyModifiers::NONE) => Some(Self::PlayPause),
            (KeyCode::Char('s'), KeyModifiers::NONE) => Some(Self::Stop),
            (KeyCode::Char('+'), KeyModifiers::NONE) => Some(Self::VolumeUp),
            (KeyCode::Char('-'), KeyModifiers::NONE) => Some(Self::VolumeDown),
            (KeyCode::Char(']'), KeyModifiers::NONE) => Some(Self::SpeedUp),
            (KeyCode::Char('['), KeyModifiers::NONE) => Some(Self::SpeedDown),
            _ => Some(Self::Key(key)),
        }
    }
}
