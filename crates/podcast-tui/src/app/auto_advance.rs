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

                    // Mark episode as played
                    match db.mark_episode_played(completed_episode_id, true).await {
                        Err(e) => {
                            tracing::error!("Failed to mark episode as played: {}", e);
                        }
                        _ => {
                            event_bus.publish(StateEvent::EpisodeMarkedPlayed {
                                episode_id: completed_episode_id,
                            });
                        }
                    }

                    // Delete-on-finish: reclaim a finished episode's download.
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

                    // Remove from queue
                    if let Err(e) = queue_manager.remove_episode(completed_episode_id).await {
                        tracing::error!("Failed to remove episode from queue: {}", e);
                    }

                    // Play next episode in queue (with retry on failure)
                    loop {
                        match queue_manager.get_next().await {
                            Ok(Some(next_item)) => {
                                let next_episode_id = next_item.episode_id;
                                tracing::info!("Auto-advancing to next episode in queue");

                                // Load the episode and play it (from the top),
                                // reusing the shared play path so auto-advance
                                // streams to disk exactly like a manual play.
                                let load_result = match db.get_episode(next_episode_id).await {
                                    Ok(Some(next_episode)) => {
                                        match state::load_and_play(
                                            &audio_player,
                                            &audio_streamer,
                                            &next_episode,
                                            std::time::Duration::ZERO,
                                        )
                                        .await
                                        {
                                            Ok(()) => {
                                                tracing::info!(
                                                    "Auto-playing: {}",
                                                    next_episode.title
                                                );
                                                event_bus.publish(StateEvent::QueueAdvanced {
                                                    next_episode_id,
                                                });
                                                Ok(())
                                            }
                                            Err(e) => Err(format!("Failed to play: {}", e)),
                                        }
                                    }
                                    Ok(None) => Err(format!(
                                        "Episode {} not found in database",
                                        next_episode_id
                                    )),
                                    Err(e) => Err(format!("Database error: {}", e)),
                                };

                                match load_result {
                                    Ok(_) => {
                                        // Successfully loaded and started playing
                                        break;
                                    }
                                    Err(error_msg) => {
                                        // Failed to play this episode - emit error and try next
                                        tracing::error!(
                                            "Failed to auto-play episode {}: {}",
                                            next_episode_id,
                                            error_msg
                                        );
                                        event_bus.publish(StateEvent::PlaybackError {
                                            error: format!("Auto-play failed: {}", error_msg),
                                        });

                                        // Remove failed episode from queue and try next
                                        if let Err(e) =
                                            queue_manager.remove_episode(next_episode_id).await
                                        {
                                            tracing::error!(
                                                "Failed to remove failed episode from queue: {}",
                                                e
                                            );
                                            break;
                                        }

                                        // Continue loop to try next episode
                                    }
                                }
                            }
                            Ok(None) => {
                                tracing::info!("Queue empty, no more episodes to play");
                                break;
                            }
                            Err(e) => {
                                tracing::error!("Failed to get next from queue: {}", e);
                                event_bus.publish(StateEvent::PlaybackError {
                                    error: format!("Queue error: {}", e),
                                });
                                break;
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
