use super::*;
use crate::app::events::EventBus;

// Helper function to create an AudioPlayer for tests
fn create_test_player() -> AudioPlayer {
    let event_bus = Arc::new(EventBus::new());
    AudioPlayer::new(event_bus).unwrap()
}

#[test]
fn run_dry_event_distinguishes_failure_from_completion() {
    let id = Uuid::new_v4();

    // Natural end -> completion (auto-advance marks played + advances).
    match run_dry_event(id, false) {
        StateEvent::PlaybackCompleted { episode_id } => assert_eq!(episode_id, id),
        other => panic!("expected PlaybackCompleted, got {other:?}"),
    }

    // Mid-stream download failure -> interruption (auto-retry / sticky notice,
    // position kept, not marked played), never a completion.
    match run_dry_event(id, true) {
        StateEvent::StreamInterrupted { episode_id } => assert_eq!(episode_id, id),
        other => panic!("expected StreamInterrupted, got {other:?}"),
    }
}

#[tokio::test]
async fn test_audio_player_is_send_sync() {
    // This test verifies AudioPlayer can be moved across thread boundaries
    let player = Arc::new(create_test_player());

    let p1 = player.clone();
    let p2 = player.clone();

    let h1 = tokio::spawn(async move {
        p1.pause().await;
    });

    let h2 = tokio::spawn(async move {
        let _ = p2.get_position().await;
    });

    h1.await.unwrap();
    h2.await.unwrap();

    // If this test compiles and runs, AudioPlayer is Send + Sync
}

#[tokio::test]
async fn test_new_player_is_stopped() {
    let player = create_test_player();
    assert!(!player.is_playing().await);
    assert_eq!(player.get_position().await, 0.0);
}

#[tokio::test]
async fn test_volume_clamps_to_valid_range() {
    let player = create_test_player();

    player.set_volume(-1.0).await;
    assert_eq!(player.get_volume().await, 0.0);

    player.set_volume(2.0).await;
    assert_eq!(player.get_volume().await, 1.0);

    player.set_volume(0.5).await;
    assert_eq!(player.get_volume().await, 0.5);
}

#[tokio::test]
async fn test_speed_clamps_to_valid_range() {
    let player = create_test_player();

    player.set_speed(0.05).await;
    let speed = player.get_speed().await;
    assert!(
        speed >= 0.1,
        "Speed should be clamped to 0.1, got {}",
        speed
    );

    player.set_speed(10.0).await;
    let speed = player.get_speed().await;
    assert!(
        speed <= 4.0,
        "Speed should be clamped to 4.0, got {}",
        speed
    );
}

#[tokio::test]
async fn test_pause_sets_state() {
    let player = create_test_player();

    player.pause().await;

    assert!(player.is_paused().await);
    assert!(!player.is_playing().await);
}

#[tokio::test]
async fn test_play_sets_state() {
    let player = create_test_player();

    // Set paused state first
    player.pause().await;
    assert!(player.is_paused().await);

    // Resume playback
    player.play().await;

    // Note: Without actual audio, is_playing may not be true
    // but is_paused should be false
    assert!(!player.is_paused().await);
}

#[tokio::test]
async fn test_stop_resets_state() {
    let player = create_test_player();

    player.stop().await;

    assert!(!player.is_playing().await);
    assert!(!player.is_paused().await);
    assert!(player.is_stopped().await);
    assert_eq!(player.get_position().await, 0.0);
    assert_eq!(player.get_current_episode().await, None);
}

#[tokio::test]
async fn test_seek_forward_with_operation_lock() {
    let player = create_test_player();

    let result = player.seek_forward(Duration::from_secs(30)).await;

    // Should succeed even without audio loaded
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_seek_backward_with_operation_lock() {
    let player = create_test_player();

    let result = player.seek_backward(Duration::from_secs(10)).await;

    // Should succeed even without audio loaded
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_seek_to_specific_position() {
    let player = create_test_player();

    let target = Duration::from_secs(100);
    let result = player.seek_to(target).await;

    // Should succeed even without audio loaded
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_get_current_episode_initially_none() {
    let player = create_test_player();

    assert_eq!(player.get_current_episode().await, None);
}

#[tokio::test]
async fn test_volume_persists_across_calls() {
    let player = create_test_player();

    player.set_volume(0.3).await;
    assert_eq!(player.get_volume().await, 0.3);

    player.set_volume(0.7).await;
    assert_eq!(player.get_volume().await, 0.7);
}

#[tokio::test]
async fn test_speed_persists_across_calls() {
    let player = create_test_player();

    player.set_speed(1.5).await;
    assert_eq!(player.get_speed().await, 1.5);

    player.set_speed(2.0).await;
    assert_eq!(player.get_speed().await, 2.0);
}

#[tokio::test]
async fn test_concurrent_volume_changes() {
    let player = Arc::new(create_test_player());

    let mut handles = vec![];

    // Spawn 10 concurrent volume changes
    for i in 0..10 {
        let p = player.clone();
        let handle = tokio::spawn(async move {
            let vol = (i as f32) * 0.1;
            p.set_volume(vol).await;
        });
        handles.push(handle);
    }

    // Wait for all to complete
    for handle in handles {
        handle.await.unwrap();
    }

    // Should have some valid volume (no panics)
    let final_vol = player.get_volume().await;
    assert!((0.0..=1.0).contains(&final_vol));
}

#[tokio::test]
async fn test_concurrent_speed_changes() {
    let player = Arc::new(create_test_player());

    let mut handles = vec![];

    // Spawn 10 concurrent speed changes
    for i in 1..=10 {
        let p = player.clone();
        let handle = tokio::spawn(async move {
            let speed = i as f32 * 0.5;
            p.set_speed(speed).await;
        });
        handles.push(handle);
    }

    // Wait for all to complete
    for handle in handles {
        handle.await.unwrap();
    }

    // Should have some valid speed (no panics)
    let final_speed = player.get_speed().await;
    assert!(final_speed > 0.0);
}

#[tokio::test]
async fn test_concurrent_seeks() {
    let player = Arc::new(create_test_player());

    let mut handles = vec![];

    // Spawn 5 concurrent seeks
    for i in 0..5 {
        let p = player.clone();
        let handle = tokio::spawn(async move {
            let pos = Duration::from_secs(i * 10);
            let _ = p.seek_to(pos).await;
        });
        handles.push(handle);
    }

    // Wait for all to complete; should not deadlock
    let timeout =
        tokio::time::timeout(Duration::from_secs(5), futures::future::join_all(handles)).await;

    assert!(timeout.is_ok(), "Concurrent seeks deadlocked");
}

#[tokio::test]
async fn test_mixed_concurrent_operations() {
    let player = Arc::new(create_test_player());

    let mut handles = vec![];

    // Mix of different operations
    for i in 0..20 {
        let p = player.clone();
        let handle = tokio::spawn(async move {
            match i % 5 {
                0 => p.set_volume(0.5).await,
                1 => p.set_speed(1.5).await,
                2 => {
                    p.pause().await;
                }
                3 => {
                    p.play().await;
                }
                _ => {
                    let _ = p.seek_forward(Duration::from_secs(5)).await;
                }
            }
        });
        handles.push(handle);
    }

    // All should complete without deadlock
    let timeout =
        tokio::time::timeout(Duration::from_secs(5), futures::future::join_all(handles)).await;

    assert!(timeout.is_ok(), "Mixed operations deadlocked");
}

#[tokio::test]
async fn test_is_stopped_after_creation() {
    let player = create_test_player();

    assert!(player.is_stopped().await);
}

#[tokio::test]
async fn test_multiple_players_independent() {
    let player1 = create_test_player();
    let player2 = create_test_player();

    player1.set_volume(0.3).await;
    player2.set_volume(0.7).await;

    assert_eq!(player1.get_volume().await, 0.3);
    assert_eq!(player2.get_volume().await, 0.7);
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn prop_volume_always_clamped(vol in -1000.0f32..1000.0f32) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let player = create_test_player();
                player.set_volume(vol).await;
                let actual = player.get_volume().await;
                prop_assert!((0.0..=1.0).contains(&actual),
                    "Volume {} should be clamped to [0.0, 1.0], got {}",
                    vol, actual);
                Ok(())
            })?;
        }

        #[test]
        fn prop_speed_always_positive(speed in -100.0f32..100.0f32) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let player = create_test_player();
                player.set_speed(speed).await;
                let actual = player.get_speed().await;
                prop_assert!(actual > 0.0,
                    "Speed {} should always be positive, got {}",
                    speed, actual);
                Ok(())
            })?;
        }

        #[test]
        fn prop_speed_within_bounds(speed in -100.0f32..100.0f32) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let player = create_test_player();
                player.set_speed(speed).await;
                let actual = player.get_speed().await;
                prop_assert!((0.1..=4.0).contains(&actual),
                    "Speed {} should be clamped to [0.1, 4.0], got {}",
                    speed, actual);
                Ok(())
            })?;
        }

        #[test]
        fn prop_seek_backward_never_negative(seek_back_secs in 0u64..10000) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let player = create_test_player();

                player.seek_backward(Duration::from_secs(seek_back_secs)).await.unwrap();
                let pos = player.get_position().await;

                prop_assert!(pos >= 0.0,
                    "Position after seeking back {} seconds should not be negative, got {}",
                    seek_back_secs, pos);
                Ok(())
            })?;
        }

        #[test]
        fn prop_position_never_negative(operations in prop::collection::vec(0u8..5, 0..100)) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let player = create_test_player();

                // Perform random operations
                for op in operations {
                    match op {
                        0 => { player.stop().await; },
                        1 => { let _ = player.seek_forward(Duration::from_secs(10)).await; },
                        2 => { let _ = player.seek_backward(Duration::from_secs(5)).await; },
                        3 => { let _ = player.seek_to(Duration::from_secs(100)).await; },
                        _ => { player.pause().await; },
                    }

                    let pos = player.get_position().await;
                    prop_assert!(pos >= 0.0,
                        "Position should never be negative after operation {}, got {}",
                        op, pos);
                }
                Ok(())
            })?;
        }

        #[test]
        fn prop_volume_persistence(vol1 in 0.0f32..1.0f32, vol2 in 0.0f32..1.0f32) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let player = create_test_player();

                player.set_volume(vol1).await;
                let retrieved1 = player.get_volume().await;
                prop_assert!((retrieved1 - vol1).abs() < 0.001,
                    "Volume {} should persist, got {}",
                    vol1, retrieved1);

                player.set_volume(vol2).await;
                let retrieved2 = player.get_volume().await;
                prop_assert!((retrieved2 - vol2).abs() < 0.001,
                    "Volume {} should persist, got {}",
                    vol2, retrieved2);
                Ok(())
            })?;
        }

        #[test]
        fn prop_speed_persistence(speed1 in 0.1f32..4.0f32, speed2 in 0.1f32..4.0f32) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let player = create_test_player();

                player.set_speed(speed1).await;
                let retrieved1 = player.get_speed().await;
                prop_assert!((retrieved1 - speed1).abs() < 0.001,
                    "Speed {} should persist, got {}",
                    speed1, retrieved1);

                player.set_speed(speed2).await;
                let retrieved2 = player.get_speed().await;
                prop_assert!((retrieved2 - speed2).abs() < 0.001,
                    "Speed {} should persist, got {}",
                    speed2, retrieved2);
                Ok(())
            })?;
        }
    }
}
