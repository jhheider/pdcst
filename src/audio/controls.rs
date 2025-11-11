use crate::audio::AudioPlayer;
use std::sync::Arc;
use std::time::Duration;

pub struct PlaybackControls {
    player: Arc<AudioPlayer>,
}

impl PlaybackControls {
    pub fn new(player: Arc<AudioPlayer>) -> Self {
        Self { player }
    }

    pub async fn toggle_play_pause(&self) {
        if self.player.is_playing().await {
            self.player.pause().await;
        } else if self.player.is_paused().await {
            self.player.play().await;
        }
    }

    pub async fn stop(&self) {
        self.player.stop().await;
    }

    pub async fn increase_speed(&self, increment: f32) {
        let current = self.player.get_speed().await;
        let new_speed = (current + increment).clamp(0.5, 3.0);
        self.player.set_speed(new_speed).await;
    }

    pub async fn decrease_speed(&self, decrement: f32) {
        let current = self.player.get_speed().await;
        let new_speed = (current - decrement).clamp(0.5, 3.0);
        self.player.set_speed(new_speed).await;
    }

    pub async fn increase_volume(&self, increment: f32) {
        let current = self.player.get_volume().await;
        let new_volume = (current + increment).clamp(0.0, 1.0);
        self.player.set_volume(new_volume).await;
    }

    pub async fn decrease_volume(&self, decrement: f32) {
        let current = self.player.get_volume().await;
        let new_volume = (current - decrement).clamp(0.0, 1.0);
        self.player.set_volume(new_volume).await;
    }

    pub async fn skip_forward(&self, seconds: u64) {
        if let Err(e) = self.player.seek_forward(Duration::from_secs(seconds)).await {
            tracing::warn!("Failed to skip forward: {}", e);
        }
    }

    pub async fn skip_backward(&self, seconds: u64) {
        if let Err(e) = self.player.seek_backward(Duration::from_secs(seconds)).await {
            tracing::warn!("Failed to skip backward: {}", e);
        }
    }
}
