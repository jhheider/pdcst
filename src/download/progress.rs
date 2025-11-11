use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct DownloadProgress {
    pub episode_id: Uuid,
    pub total_bytes: Option<u64>,
    pub downloaded_bytes: Arc<RwLock<u64>>,
    pub complete: Arc<RwLock<bool>>,
}

impl DownloadProgress {
    pub fn new(episode_id: Uuid, total_bytes: Option<u64>) -> Self {
        Self {
            episode_id,
            total_bytes,
            downloaded_bytes: Arc::new(RwLock::new(0)),
            complete: Arc::new(RwLock::new(false)),
        }
    }

    pub async fn update(&self, bytes: u64) {
        let mut downloaded = self.downloaded_bytes.write().await;
        *downloaded += bytes;
    }

    pub async fn set_complete(&self) {
        let mut complete = self.complete.write().await;
        *complete = true;
    }

    pub async fn get_progress(&self) -> f64 {
        if let Some(total) = self.total_bytes {
            let downloaded = *self.downloaded_bytes.read().await;
            (downloaded as f64 / total as f64) * 100.0
        } else {
            0.0
        }
    }

    pub async fn is_complete(&self) -> bool {
        *self.complete.read().await
    }
}
