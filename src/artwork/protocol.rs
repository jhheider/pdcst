use crate::models::ArtworkProtocol;
use std::env;

/// Detect which terminal graphics protocol is supported
pub fn detect_artwork_protocol() -> ArtworkProtocol {
    // Check for iTerm2
    if let Ok(term_program) = env::var("TERM_PROGRAM") {
        if term_program == "iTerm.app" {
            return ArtworkProtocol::ITerm2;
        }
    }

    // Check for Kitty
    if let Ok(term) = env::var("TERM") {
        if term.contains("kitty") {
            return ArtworkProtocol::Kitty;
        }
    }

    // Check for Sixel support (common in xterm, mlterm, foot, wezterm)
    // Many terminals that support sixel set this variable
    if env::var("TERM")
        .ok()
        .map(|t| t.contains("xterm"))
        .unwrap_or(false)
    {
        // Could do more sophisticated detection, but default to Sixel for xterm
        return ArtworkProtocol::Sixel;
    }

    // Check for WezTerm (supports both Kitty and Sixel)
    if env::var("WEZTERM_EXECUTABLE").is_ok() {
        return ArtworkProtocol::Kitty; // WezTerm prefers Kitty protocol
    }

    // No supported protocol detected
    ArtworkProtocol::None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_protocol() {
        // This will return None in CI environments
        let protocol = detect_artwork_protocol();
        // Just make sure it doesn't panic
        assert!(matches!(
            protocol,
            ArtworkProtocol::None
                | ArtworkProtocol::Sixel
                | ArtworkProtocol::Kitty
                | ArtworkProtocol::ITerm2
        ));
    }
}
