# Event-Driven Architecture Plan

## Current State Analysis

### Polling Mechanism (Inefficient)
**Location:** `src/app/mod.rs:119` and `src/app/state.rs:191-203`

```rust
// Main loop polls every 100ms
if event::poll(Duration::from_millis(100))? {
    // Handle keyboard input
}

// AppState::update() polls state
pub async fn update(&mut self) -> Result<()> {
    self.is_playing = self.audio_player.is_playing().await;  // Poll audio state
    self.playback_speed = self.audio_player.get_speed().await;
    self.volume = self.audio_player.get_volume().await;
    // Load subscriptions...
}
```

**Problems:**
1. Wastes CPU cycles polling when nothing changed
2. 100ms latency for state updates
3. UI redraws unnecessarily
4. Battery drain on mobile devices (future concern)

### Existing Event Infrastructure
**Location:** `src/audio/player/mod.rs:95-99`

```rust
completion_tx: tokio::sync::mpsc::UnboundedSender<Uuid>,
completion_rx: Arc<Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<Uuid>>>>,
```

✅ **Already has playback completion events!**

---

## Proposed Event System Design

### 1. StateEvent Enum

Create a comprehensive event type covering all state changes:

```rust
#[derive(Debug, Clone)]
pub enum StateEvent {
    // Audio events
    PlaybackStarted { episode_id: Uuid },
    PlaybackPaused,
    PlaybackResumed,
    PlaybackStopped,
    PlaybackCompleted { episode_id: Uuid },
    PlaybackPosition { position_secs: f64 },
    PlaybackError { error: String },

    // Volume/Speed events
    VolumeChanged { volume: f32 },
    SpeedChanged { speed: f32 },

    // Download events
    DownloadStarted { episode_id: Uuid },
    DownloadProgress { episode_id: Uuid, percent: f32 },
    DownloadCompleted { episode_id: Uuid },
    DownloadFailed { episode_id: Uuid, error: String },
    DownloadCancelled { episode_id: Uuid },

    // Queue events
    QueueUpdated,
    QueueAdvanced { next_episode_id: Uuid },

    // Subscription events
    FeedRefreshStarted { subscription_id: Uuid },
    FeedRefreshCompleted { subscription_id: Uuid, new_episodes: usize },
    FeedRefreshFailed { subscription_id: Uuid, error: String },

    // Database events
    EpisodeMarkedPlayed { episode_id: Uuid },
    EpisodeMarkedUnplayed { episode_id: Uuid },
    SubscriptionAdded { subscription_id: Uuid },
    SubscriptionRemoved { subscription_id: Uuid },
}
```

### 2. Event Bus Architecture

Use `tokio::sync::broadcast` for multi-consumer events:

```rust
pub struct EventBus {
    sender: broadcast::Sender<StateEvent>,
}

impl EventBus {
    pub fn new() -> Self {
        let (sender, _) = broadcast::channel(1000);  // Buffer up to 1000 events
        Self { sender }
    }

    pub fn publish(&self, event: StateEvent) {
        let _ = self.sender.send(event);  // Ignore errors if no subscribers
    }

    pub fn subscribe(&self) -> broadcast::Receiver<StateEvent> {
        self.sender.subscribe()
    }
}
```

### 3. Integration Points

#### AudioPlayer Changes
- Emit events on state changes (play, pause, stop)
- Emit position updates every 1 second (not 100ms)
- Emit completion events (already exists)
- Emit volume/speed change events

#### DownloadManager Changes
- Emit download started events
- Emit progress events (throttle to every 1 second)
- Emit completion/failure/cancellation events

#### QueueManager Changes
- Emit queue updated events on add/remove/reorder
- Emit queue advanced events on auto-advance

#### FeedRefresher Changes
- Emit refresh started/completed/failed events

#### AppState Changes
- Remove `update()` polling method
- Subscribe to events in constructor
- Update local state when receiving events
- Still expose synchronous getters (cached values)

#### Main Loop Changes
```rust
async fn run_loop(&mut self, terminal: &mut Terminal) -> Result<()> {
    let mut event_rx = self.state.subscribe_events();
    let mut tick_interval = tokio::time::interval(Duration::from_millis(50));

    loop {
        tokio::select! {
            // Handle state events
            Ok(state_event) = event_rx.recv() => {
                self.handle_state_event(state_event).await?;
                // Redraw only when state changes
                terminal.draw(|f| self.ui.render(f, &self.state))?;
            }

            // Handle keyboard input (non-blocking)
            _ = tick_interval.tick() => {
                if event::poll(Duration::from_millis(0))? {
                    if let Event::Key(key) = event::read()? {
                        match key.code {
                            KeyCode::Char('q') => break,
                            _ => self.handle_key_event(key.code).await?,
                        }
                        // Redraw after input
                        terminal.draw(|f| self.ui.render(f, &self.state))?;
                    }
                }
            }
        }
    }

    Ok(())
}
```

---

## Implementation Steps

### Phase 1: Core Event Infrastructure (2 hours)
1. ✅ Create `StateEvent` enum in `src/app/events.rs`
2. ✅ Create `EventBus` struct with broadcast channels
3. ✅ Add `EventBus` to `App` and `AppState`
4. ✅ Write tests for event bus

### Phase 2: AudioPlayer Events (2-3 hours)
1. ✅ Add `EventBus` parameter to AudioPlayer constructor
2. ✅ Emit `PlaybackStarted/Paused/Resumed/Stopped` events
3. ✅ Emit `VolumeChanged/SpeedChanged` events
4. ✅ Emit `PlaybackPosition` events every 1 second
5. ✅ Convert existing completion channel to use EventBus
6. ✅ Update tests

### Phase 3: DownloadManager Events (2 hours)
1. ✅ Add `EventBus` to DownloadManager
2. ✅ Emit download lifecycle events
3. ✅ Throttle progress events to 1 second intervals
4. ✅ Update tests

### Phase 4: QueueManager & FeedRefresher Events (1-2 hours)
1. ✅ Add events for queue operations
2. ✅ Add events for feed refresh operations
3. ✅ Update tests

### Phase 5: AppState Event Consumer (2 hours)
1. ✅ Remove `update()` method
2. ✅ Subscribe to events in `new()`
3. ✅ Spawn background task to handle events and update state
4. ✅ Ensure thread-safe state access

### Phase 6: Main Loop Refactor (1-2 hours)
1. ✅ Convert to `tokio::select!` pattern
2. ✅ Remove polling timer
3. ✅ Only redraw on events or input
4. ✅ Test responsiveness

### Phase 7: Testing & Cleanup (2 hours)
1. ✅ Integration tests for event flow
2. ✅ Test that UI updates correctly
3. ✅ Remove dead polling code
4. ✅ Run full test suite
5. ✅ Performance testing (CPU usage should drop significantly)

---

## Benefits

### Performance
- **CPU Usage:** Down from constant polling to event-driven (50-80% reduction expected)
- **Battery:** Reduced wake-ups means better battery life
- **Responsiveness:** Events processed immediately instead of waiting for next poll

### Code Quality
- **Decoupling:** Components don't need to expose polling interfaces
- **Testability:** Can inject events for testing without real audio/downloads
- **Maintainability:** Clear event contracts between components

### User Experience
- **Instant Feedback:** UI updates immediately when state changes
- **Smoother Animation:** Progress bars update at optimal intervals
- **Better Performance:** More resources for actual work

---

## Risks & Mitigation

### Risk 1: Event Storms
**Problem:** Too many events flooding the channel
**Mitigation:**
- Throttle high-frequency events (position updates: 1/sec, download progress: 1/sec)
- Use bounded channels with appropriate buffer size (1000 events)
- Monitor channel capacity in tests

### Risk 2: Missed Events
**Problem:** UI might miss events if channel full
**Mitigation:**
- Use broadcast channel (all subscribers get events)
- Handle `RecvError::Lagged` gracefully by requesting full state refresh
- Log missed events for debugging

### Risk 3: Event Ordering
**Problem:** Events might arrive out of order
**Mitigation:**
- Keep events atomic (e.g., PlaybackStarted includes episode_id)
- Don't rely on strict ordering for independent events
- Use version numbers if needed

### Risk 4: Deadlocks
**Problem:** Event handlers might create cycles
**Mitigation:**
- Never block in event handlers
- Use try_send() instead of send() when publishing from event handlers
- Document and test event flow carefully

---

## Success Criteria

1. **Performance:** CPU usage drops by >50% during idle playback
2. **Responsiveness:** UI updates within 50ms of state changes
3. **Stability:** No regressions in existing tests
4. **Code Quality:** Fewer async polling patterns in codebase
5. **User Experience:** Smoother UI with instant feedback

---

## Timeline

**Total Estimated Time:** 12-16 hours

- **Phase 1-2:** 4-5 hours (Core + Audio)
- **Phase 3-4:** 3-4 hours (Downloads + Queue/Refresh)
- **Phase 5-6:** 3-4 hours (AppState + Main Loop)
- **Phase 7:** 2-3 hours (Testing & Cleanup)

**Recommended Approach:** Implement incrementally, test after each phase.

---

**Created:** 2025-11-11
**Status:** Planning
**Priority:** High (foundational change)
