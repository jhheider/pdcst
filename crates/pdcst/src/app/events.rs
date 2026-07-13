use crate::feed::SearchResult;
use tokio::sync::broadcast;
use uuid::Uuid;

/// StateEvent represents all possible state changes in the application
/// These events are published by various components and consumed by the UI and other listeners
#[derive(Debug, Clone)]
pub enum StateEvent {
    // Audio events
    PlaybackStarted {
        episode_id: Uuid,
    },
    PlaybackPaused,
    PlaybackResumed,
    PlaybackStopped,
    PlaybackCompleted {
        episode_id: Uuid,
    },
    PlaybackPosition {
        position_secs: f64,
    },
    PlaybackError {
        error: String,
    },

    // Volume/Speed events
    VolumeChanged {
        volume: f32,
    },
    SpeedChanged {
        speed: f32,
    },

    // Download events
    DownloadStarted {
        episode_id: Uuid,
    },
    DownloadProgress {
        episode_id: Uuid,
        percent: f32,
    },
    DownloadCompleted {
        episode_id: Uuid,
    },
    DownloadFailed {
        episode_id: Uuid,
        error: String,
    },
    DownloadCancelled {
        episode_id: Uuid,
    },

    // Queue events
    QueueUpdated,
    QueueAdvanced {
        next_episode_id: Uuid,
    },

    // Subscription events
    FeedRefreshStarted {
        subscription_id: Uuid,
    },
    FeedRefreshCompleted {
        subscription_id: Uuid,
        new_episodes: usize,
    },
    FeedRefreshFailed {
        subscription_id: Uuid,
        error: String,
    },

    // Feed recovery: a title search turned up a different feed URL for a
    // subscription (usually a failing one whose URL moved), or found nothing new.
    FeedFixFound {
        subscription_id: Uuid,
        podcast_title: String,
        artist: String,
        new_url: String,
    },
    FeedFixNotFound {
        subscription_id: Uuid,
    },
    /// No confident title match, but the search did return candidates: offer them
    /// as a picker (the user judges from the metadata and chooses one to re-point).
    FeedFixCandidates {
        subscription_id: Uuid,
        results: Vec<SearchResult>,
    },

    // Search events (delivered off the event loop so the UI never blocks on the
    // network call).
    SearchCompleted {
        results: Vec<SearchResult>,
    },
    SearchFailed {
        error: String,
    },

    // Database events
    EpisodeMarkedPlayed {
        episode_id: Uuid,
    },
    EpisodeMarkedUnplayed {
        episode_id: Uuid,
    },
    SubscriptionAdded {
        subscription_id: Uuid,
    },
    SubscriptionRemoved {
        subscription_id: Uuid,
    },
}

/// EventBus provides a centralized event publishing and subscription system
/// Uses tokio::sync::broadcast for efficient multi-consumer event distribution
pub struct EventBus {
    sender: broadcast::Sender<StateEvent>,
}

impl EventBus {
    /// Create a new EventBus with a buffer capacity of 1000 events
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(1000);
        Self { sender }
    }

    /// Publish an event to all subscribers
    /// Silently ignores errors if there are no active subscribers
    pub fn publish(&self, event: StateEvent) {
        let _ = self.sender.send(event);
    }

    /// Subscribe to events and receive a receiver for consuming them
    /// Each subscriber gets their own receiver and will receive all future events
    pub fn subscribe(&self) -> broadcast::Receiver<StateEvent> {
        self.sender.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_event_bus_publish_subscribe() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        let episode_id = Uuid::new_v4();
        bus.publish(StateEvent::PlaybackStarted { episode_id });

        let received = rx.recv().await.unwrap();
        match received {
            StateEvent::PlaybackStarted { episode_id: id } => assert_eq!(id, episode_id),
            _ => panic!("Expected PlaybackStarted event"),
        }
    }

    #[tokio::test]
    async fn test_event_bus_multiple_subscribers() {
        let bus = EventBus::new();
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        bus.publish(StateEvent::PlaybackPaused);

        let received1 = rx1.recv().await.unwrap();
        let received2 = rx2.recv().await.unwrap();

        assert!(matches!(received1, StateEvent::PlaybackPaused));
        assert!(matches!(received2, StateEvent::PlaybackPaused));
    }

    #[tokio::test]
    async fn test_event_bus_no_subscribers() {
        let bus = EventBus::new();
        // Should not panic when publishing with no subscribers
        bus.publish(StateEvent::PlaybackPaused);
    }

    #[tokio::test]
    async fn test_event_bus_subscribe_after_publish() {
        let bus = EventBus::new();

        // Publish before subscribing
        bus.publish(StateEvent::PlaybackPaused);

        // New subscriber should not receive old events
        let mut rx = bus.subscribe();

        // Publish new event
        let episode_id = Uuid::new_v4();
        bus.publish(StateEvent::PlaybackStarted { episode_id });

        // Should receive only the new event
        let received = rx.recv().await.unwrap();
        match received {
            StateEvent::PlaybackStarted { episode_id: id } => assert_eq!(id, episode_id),
            _ => panic!("Expected PlaybackStarted event"),
        }
    }

    #[tokio::test]
    async fn test_event_bus_volume_changed() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        bus.publish(StateEvent::VolumeChanged { volume: 0.5 });

        let received = rx.recv().await.unwrap();
        match received {
            StateEvent::VolumeChanged { volume } => assert_eq!(volume, 0.5),
            _ => panic!("Expected VolumeChanged event"),
        }
    }

    #[tokio::test]
    async fn test_event_bus_download_events() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        let episode_id = Uuid::new_v4();

        // Test download lifecycle
        bus.publish(StateEvent::DownloadStarted { episode_id });
        bus.publish(StateEvent::DownloadProgress {
            episode_id,
            percent: 50.0,
        });
        bus.publish(StateEvent::DownloadCompleted { episode_id });

        let evt1 = rx.recv().await.unwrap();
        let evt2 = rx.recv().await.unwrap();
        let evt3 = rx.recv().await.unwrap();

        assert!(matches!(evt1, StateEvent::DownloadStarted { .. }));
        assert!(matches!(evt2, StateEvent::DownloadProgress { percent, .. } if percent == 50.0));
        assert!(matches!(evt3, StateEvent::DownloadCompleted { .. }));
    }
}
