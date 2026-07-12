//! The auto-advance task.
//!
//! On every `PlaybackCompleted`, it marks the finished episode played,
//! optionally reclaims its download (`delete_on_finish`), removes it from the
//! queue, and plays the next queued episode - skipping any that fail to load.
//! This is the seed of the Phase C auto-queue; for now it lives as a standalone
//! background task spawned at startup.

use crate::app::events::{EventBus, StateEvent};
use crate::app::state;
use crate::audio::{AudioPlayer, AudioStreamer};
use crate::download::DownloadManager;
use crate::queue::QueueManager;
use crate::storage::Database;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Everything the auto-advance loop needs, grouped so `spawn` takes one arg.
pub(crate) struct AutoAdvance {
    pub event_rx: broadcast::Receiver<StateEvent>,
    pub db: Arc<Database>,
    pub event_bus: Arc<EventBus>,
    pub queue_manager: Arc<QueueManager>,
    pub audio_player: Arc<AudioPlayer>,
    pub audio_streamer: Arc<AudioStreamer>,
    pub download_manager: Arc<DownloadManager>,
    pub delete_on_finish: bool,
}

/// Spawn the auto-advance loop. It runs until the event bus closes.
pub(crate) fn spawn(deps: AutoAdvance) {
    let AutoAdvance {
        mut event_rx,
        db,
        event_bus,
        queue_manager,
        audio_player,
        audio_streamer,
        download_manager,
        delete_on_finish,
    } = deps;

    tokio::spawn(async move {
        loop {
            match event_rx.recv().await {
                Ok(StateEvent::PlaybackCompleted {
                    episode_id: completed_episode_id,
                }) => {
                    tracing::info!(
                        "Episode {} completed, checking queue for next episode",
                        completed_episode_id
                    );

                    // Delete-on-finish: reclaim the finished episode's download
                    // before it leaves the queue.
                    if delete_on_finish
                        && let Ok(Some(finished)) = db.get_episode(completed_episode_id).await
                        && finished.is_downloaded()
                    {
                        match download_manager.delete_download(&finished).await {
                            Ok(()) => tracing::info!(
                                "delete-on-finish: removed download for '{}'",
                                finished.title
                            ),
                            Err(e) => tracing::warn!(
                                "delete-on-finish failed for '{}': {}",
                                finished.title,
                                e
                            ),
                        }
                    }

                    // Advance the queue via the shared path (mark played, remove,
                    // take next), then play it - retrying past any that fail to
                    // load. `load_and_play` streams to disk like a manual play.
                    let mut next = match queue_manager.advance(completed_episode_id, true).await {
                        Ok(next) => next,
                        Err(e) => {
                            tracing::error!("Failed to advance queue: {}", e);
                            None
                        }
                    };
                    while let Some(episode) = next {
                        match state::load_and_play(
                            &audio_player,
                            &audio_streamer,
                            &episode,
                            std::time::Duration::ZERO,
                        )
                        .await
                        {
                            Ok(()) => {
                                tracing::info!("Auto-playing: {}", episode.title);
                                event_bus.publish(StateEvent::QueueAdvanced {
                                    next_episode_id: episode.id,
                                });
                                break;
                            }
                            Err(e) => {
                                tracing::error!("Failed to auto-play '{}': {}", episode.title, e);
                                event_bus.publish(StateEvent::PlaybackError {
                                    error: format!("Auto-play failed: {}", e),
                                });
                                // Drop the failed episode (do not mark it played)
                                // and try the next one.
                                next = match queue_manager.advance(episode.id, false).await {
                                    Ok(next) => next,
                                    Err(e) => {
                                        tracing::error!("Failed to advance past failure: {}", e);
                                        None
                                    }
                                };
                            }
                        }
                    }
                }
                Ok(_) => {
                    // Ignore other events
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!("Auto-advance task lagged, skipped {} events", skipped);
                    // Continue processing - we'll catch the next PlaybackCompleted
                }
                Err(broadcast::error::RecvError::Closed) => {
                    tracing::info!("Event bus closed, stopping auto-advance");
                    break;
                }
            }
        }
    });
}
