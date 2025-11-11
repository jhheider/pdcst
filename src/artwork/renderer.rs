use crate::models::ArtworkProtocol;
use anyhow::Result;
use ratatui::layout::Rect;
use std::path::Path;

pub struct ArtworkRenderer {
    protocol: ArtworkProtocol,
}

impl ArtworkRenderer {
    pub fn new(protocol: ArtworkProtocol) -> Self {
        Self { protocol }
    }

    /// Render artwork in the given area
    /// Note: This is a placeholder implementation
    /// Full implementation would require ratatui-image integration
    pub fn render(&self, _image_path: &Path, _area: Rect) -> Result<()> {
        match self.protocol {
            ArtworkProtocol::None => {
                // Don't render anything
                Ok(())
            }
            _ => {
                // TODO: Implement actual rendering with ratatui-image
                // This would involve:
                // 1. Loading the image with the `image` crate
                // 2. Resizing to fit the terminal area
                // 3. Using ratatui-image to render based on protocol
                tracing::debug!("Artwork rendering not fully implemented yet");
                Ok(())
            }
        }
    }

    pub fn supports_artwork(&self) -> bool {
        self.protocol != ArtworkProtocol::None
    }
}
