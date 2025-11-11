# Podcast TUI Remediation Plan

## Executive Summary

This plan addresses critical bugs, missing features, and architectural issues identified in code review. Each task has clear pass/fail criteria and is organized by priority.

**Total Estimated Time**: 100-130 hours (17-22 days @ 6h/day)

---

## Phase 1: CRITICAL BUG FIXES (Must Complete First)

### 1.1 Fix AudioPlayer Thread Safety 🚨 **BLOCKER**

**Problem**: `AudioPlayer` contains `!Send` types but is wrapped in `Arc` and used across async boundaries. Will panic at runtime.

**Root Cause**: `rodio::OutputStream` is `!Send` (src/audio/player.rs:10)

**Solution**: Spawn dedicated audio thread, use message-passing architecture.

**Implementation**:
```rust
// src/audio/player.rs - Complete rewrite
use std::sync::mpsc;  // Sync channel for thread boundary

pub struct AudioPlayer {
    command_tx: mpsc::Sender<AudioCommand>,
    state: Arc<RwLock<AudioState>>,
}

enum AudioCommand {
    Play { episode_id: Uuid, data: Vec<u8> },
    Pause,
    Resume,
    Stop,
    Seek(Duration),
    SetSpeed(f32),
    SetVolume(f32),
    Shutdown,
}

struct AudioState {
    current_episode: Option<Uuid>,
    position: Duration,
    is_playing: bool,
    speed: f32,
    volume: f32,
}

impl AudioPlayer {
    pub fn new() -> Result<Self> {
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(RwLock::new(AudioState::default()));

        let state_clone = state.clone();
        std::thread::spawn(move || {
            audio_thread(rx, state_clone);
        });

        Ok(Self { command_tx: tx, state })
    }

    pub async fn play_from_memory(&self, episode_id: Uuid, data: &[u8]) -> Result<()> {
        self.command_tx.send(AudioCommand::Play {
            episode_id,
            data: data.to_vec(),
        })?;
        Ok(())
    }
}

fn audio_thread(rx: mpsc::Receiver<AudioCommand>, state: Arc<RwLock<AudioState>>) {
    let (_stream, stream_handle) = OutputStream::try_default().unwrap();
    let mut sink: Option<Sink> = None;

    while let Ok(cmd) = rx.recv() {
        match cmd {
            AudioCommand::Play { episode_id, data } => {
                // Handle playback
            }
            AudioCommand::Shutdown => break,
            // ... handle other commands
        }

        // Update shared state
        // Use polling or callbacks for position updates
    }
}
```

**Files Modified**:
- `src/audio/player.rs` (rewrite ~300 lines)
- `src/audio/mod.rs` (export new types)

**Pass Criteria**:
1. ✅ Compile check: `cargo clippy` shows zero `arc_with_non_send_sync` warnings
2. ✅ Type check: `AudioPlayer` implements `Send + Sync`
   ```rust
   fn assert_send_sync<T: Send + Sync>() {}
   assert_send_sync::<AudioPlayer>();
   ```
3. ✅ Runtime test: Move `AudioPlayer` across thread boundaries
   ```rust
   #[tokio::test]
   async fn test_audio_player_is_send_sync() {
       let player = Arc::new(AudioPlayer::new().unwrap());
       let p1 = player.clone();
       let p2 = player.clone();

       let h1 = tokio::spawn(async move { p1.pause().await });
       let h2 = tokio::spawn(async move { p2.get_position().await });

       h1.await.unwrap().unwrap();
       h2.await.unwrap();
   }
   ```
4. ✅ Manual verification: Run app for 5 minutes without panics

**Time Estimate**: 6-8 hours

---

### 1.2 Fix Seeking Implementation

**Problem**:
- Copies entire episode into memory (line 52: `audio_data.to_vec()`)
- Re-decodes from start on every seek (line 80: `Cursor::new(audio_data.clone())`)
- O(n) complexity for seeking to position n

**Performance Impact**: 60MB episode × 2 copies = 120MB RAM per episode

**Solution**: Implement frame-accurate seeking with `symphonia` crate.

**Implementation**:
```toml
# Cargo.toml
symphonia = { version = "0.5", features = ["mp3", "aac", "flac", "vorbis"] }
```

```rust
// src/audio/decoder.rs - NEW FILE
use symphonia::core::formats::FormatReader;
use symphonia::core::io::MediaSourceStream;

pub struct SeekableDecoder {
    reader: Box<dyn FormatReader>,
    track_id: u32,
    sample_rate: u32,
    channels: usize,
}

impl SeekableDecoder {
    pub fn new(data: Vec<u8>) -> Result<Self> {
        let mss = MediaSourceStream::new(Box::new(Cursor::new(data)), Default::default());
        let format_opts = FormatOptions::default();
        let metadata_opts = MetadataOptions::default();

        let probed = symphonia::default::get_probe()
            .format(&Hint::new(), mss, &format_opts, &metadata_opts)?;

        let reader = probed.format;
        let track = reader.default_track()
            .context("No default track")?;

        Ok(Self {
            reader,
            track_id: track.id,
            sample_rate: track.codec_params.sample_rate.unwrap_or(44100),
            channels: track.codec_params.channels.unwrap().count(),
        })
    }

    pub fn seek(&mut self, position: Duration) -> Result<()> {
        let time_base = self.reader.default_track().codec_params.time_base.unwrap();
        let timestamp = time_base.calc_timestamp(position.as_secs_f64());

        self.reader.seek(
            SeekMode::Accurate,
            SeekTo::TimeStamp { ts: timestamp, track_id: self.track_id }
        )?;

        Ok(())
    }

    pub fn read_samples(&mut self, buf: &mut [f32]) -> Result<usize> {
        // Read and decode samples
        // Return number of samples read
    }
}
```

```rust
// src/audio/player.rs - Update audio thread
fn audio_thread(...) {
    let mut decoder: Option<SeekableDecoder> = None;

    while let Ok(cmd) = rx.recv() {
        match cmd {
            AudioCommand::Play { data, .. } => {
                decoder = Some(SeekableDecoder::new(data)?);
                // Start playback
            }
            AudioCommand::Seek(position) => {
                if let Some(dec) = &mut decoder {
                    dec.seek(position)?;
                    // Resume from new position
                }
            }
        }
    }
}
```

**Files Modified**:
- `src/audio/decoder.rs` (NEW, ~200 lines)
- `src/audio/player.rs` (update to use SeekableDecoder, ~50 lines changed)
- `Cargo.toml` (add symphonia)

**Pass Criteria**:
1. ✅ Performance benchmark: Seek to any position in <100ms
   ```rust
   #[tokio::test]
   async fn bench_seek_performance() {
       let player = create_player_with_60min_episode().await;

       let start = Instant::now();
       player.seek_to(Duration::from_secs(3000)).await.unwrap();
       let elapsed = start.elapsed();

       assert!(elapsed < Duration::from_millis(100),
               "Seek took {:?}, expected <100ms", elapsed);
   }
   ```

2. ✅ Memory stability: No spikes during seeks
   ```rust
   #[tokio::test]
   async fn test_seek_memory_stable() {
       let player = create_player_with_60min_episode().await;
       let baseline = get_memory_usage();

       for i in 0..100 {
           player.seek_to(Duration::from_secs(i * 30)).await.unwrap();
       }

       let final_mem = get_memory_usage();
       let growth = final_mem - baseline;
       assert!(growth < 5_000_000, // 5MB tolerance
               "Memory grew by {}MB during seeks", growth / 1_000_000);
   }
   ```

3. ✅ Accuracy test: Seek to position, verify playback continues from correct spot
   ```rust
   #[tokio::test]
   async fn test_seek_accuracy() {
       let player = create_player_with_test_audio().await; // Known audio content
       player.seek_to(Duration::from_secs(30)).await.unwrap();

       let pos = player.get_position().await;
       assert!((29.5..=30.5).contains(&pos), "Position inaccurate: {}", pos);
   }
   ```

**Time Estimate**: 8-10 hours

---

### 1.3 Wire Up Audio Playback Pipeline

**Problem**: `play_episode()` has `TODO` comment (line 161), never loads/plays audio.

**Impact**: The entire seeking implementation is unreachable dead code.

**Solution**: Connect AudioStreamer → AudioPlayer in `play_episode()`.

**Implementation**:
```rust
// src/app/state.rs
pub struct AppState {
    // ... existing fields ...
    pub audio_streamer: Arc<AudioStreamer>,  // ADD THIS
}

impl AppState {
    async fn play_episode(&mut self, episode: Episode) -> Result<()> {
        tracing::info!("Playing episode: {}", episode.title);

        // 1. Load audio data
        let audio_data = if episode.is_downloaded() {
            // Load from local file
            let path = episode.local_path.as_ref()
                .context("Downloaded episode missing local_path")?;
            tracing::debug!("Loading from file: {}", path);

            let state = self.audio_streamer
                .load_from_file(episode.id, Path::new(path))
                .await?;
            state.get_buffer().await
        } else {
            // Stream from URL
            tracing::debug!("Streaming from URL: {}", episode.url);

            let state = self.audio_streamer
                .stream_episode(episode.id, &episode.url)
                .await?;
            state.get_buffer().await
        };

        // 2. Play through audio player
        self.audio_player
            .play_from_memory(episode.id, &audio_data)
            .await
            .context("Failed to play audio")?;

        // 3. Update app state
        self.current_episode = Some(episode.clone());
        self.is_playing = true;

        // 4. Save playback state to database
        self.db.save_playback_state(&PlaybackState {
            episode_id: episode.id,
            position: 0.0,
            playing: true,
        }).await?;

        tracing::info!("Playback started successfully");
        Ok(())
    }
}
```

```rust
// src/storage/db.rs - Add missing method
impl Database {
    pub async fn save_playback_state(&self, state: &PlaybackState) -> Result<()> {
        sqlx::query(
            "UPDATE playback_state
             SET current_episode_id = ?,
                 position_seconds = ?,
                 status = ?,
                 updated_at = CURRENT_TIMESTAMP
             WHERE id = 1"
        )
        .bind(state.episode_id.to_string())
        .bind(state.position)
        .bind(if state.playing { "Playing" } else { "Paused" })
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    pub async fn load_playback_state(&self) -> Result<Option<PlaybackState>> {
        let row = sqlx::query(
            "SELECT current_episode_id, position_seconds, status
             FROM playback_state WHERE id = 1"
        )
        .fetch_optional(&self.pool)
        .await?;

        // Parse and return
        Ok(row.map(|r| PlaybackState { /* ... */ }))
    }
}
```

**Files Modified**:
- `src/app/state.rs` (~30 lines changed)
- `src/storage/db.rs` (~40 lines added)
- `src/app/mod.rs` (pass AudioStreamer to AppState::new)

**Pass Criteria**:
1. ✅ Integration test with real MP3 file
   ```rust
   #[tokio::test]
   async fn test_play_episode_end_to_end() {
       let mut app = create_test_app().await;

       // Create test episode with bundled MP3
       let episode = Episode {
           id: Uuid::new_v4(),
           url: format!("file://{}", test_mp3_path()),
           // ... other fields ...
       };

       app.state.play_episode(episode).await
           .expect("Failed to play episode");

       // Wait for playback to start
       tokio::time::sleep(Duration::from_millis(500)).await;

       // Verify playing
       assert!(app.state.audio_player.is_playing().await);

       // Verify position advancing
       let pos1 = app.state.audio_player.get_position().await;
       tokio::time::sleep(Duration::from_secs(1)).await;
       let pos2 = app.state.audio_player.get_position().await;

       assert!(pos2 > pos1, "Position not advancing: {} -> {}", pos1, pos2);
   }
   ```

2. ✅ Database persistence check
   ```rust
   #[tokio::test]
   async fn test_playback_state_persisted() {
       let app = create_test_app().await;
       let episode = create_test_episode();

       app.state.play_episode(episode.clone()).await.unwrap();

       // Check database
       let saved_state = app.state.db.load_playback_state().await.unwrap();
       assert!(saved_state.is_some());
       assert_eq!(saved_state.unwrap().episode_id, episode.id);
   }
   ```

3. ✅ Manual test: Run app, select episode, press Enter, verify audio plays

**Time Estimate**: 4-5 hours

---

### 1.4 Fix Database Migration System

**Problem**:
- No version tracking (runs same SQL every startup)
- Can't evolve schema (relies on `IF NOT EXISTS`)
- Not using sqlx's built-in migration system

**Future Impact**: Adding a column will require manual ALTER statements and break existing deployments.

**Solution**: Use sqlx migrations with version tracking.

**Implementation**:
```bash
# Delete old migration
rm migrations/001_initial_schema.sql

# Create new migrations using sqlx CLI
cargo install sqlx-cli --no-default-features --features sqlite

# Create migrations
sqlx migrate add -r initial_schema
sqlx migrate add -r add_indices
sqlx migrate add -r add_playback_state
```

```sql
-- migrations/20250111000001_initial_schema.up.sql
CREATE TABLE subscriptions (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    description TEXT,
    author TEXT,
    rss_url TEXT NOT NULL UNIQUE,
    website_url TEXT,
    artwork_url TEXT,
    artwork_path TEXT,
    categories TEXT,
    auto_queue BOOLEAN NOT NULL DEFAULT 0,
    priority TEXT NOT NULL DEFAULT 'Medium',
    auto_download BOOLEAN NOT NULL DEFAULT 0,
    last_refreshed DATETIME NOT NULL,
    created_at DATETIME NOT NULL
);

CREATE TABLE episodes (
    id TEXT PRIMARY KEY,
    subscription_id TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT,
    url TEXT NOT NULL,
    guid TEXT NOT NULL,
    published_at DATETIME NOT NULL,
    duration_seconds INTEGER,
    file_size_bytes INTEGER,
    file_type TEXT,
    download_status TEXT NOT NULL DEFAULT 'NotDownloaded',
    local_path TEXT,
    playback_position_seconds INTEGER NOT NULL DEFAULT 0,
    played BOOLEAN NOT NULL DEFAULT 0,
    last_played_at DATETIME,
    created_at DATETIME NOT NULL,
    FOREIGN KEY (subscription_id) REFERENCES subscriptions(id) ON DELETE CASCADE,
    UNIQUE(subscription_id, guid)
);

-- migrations/20250111000001_initial_schema.down.sql
DROP TABLE IF EXISTS episodes;
DROP TABLE IF EXISTS subscriptions;
```

```rust
// src/storage/db.rs
impl Database {
    pub async fn new(db_path: &Path) -> Result<Self> {
        // Ensure parent directory exists
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", db_path.display()))?
            .create_if_missing(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(options)
            .await
            .context("Failed to connect to database")?;

        let db = Self { pool };

        // Run migrations using sqlx
        sqlx::migrate!("./migrations")
            .run(&db.pool)
            .await
            .context("Failed to run database migrations")?;

        Ok(db)
    }

    // REMOVE the old run_migrations() method
}
```

```toml
# Cargo.toml - Ensure migrate feature enabled
[dependencies]
sqlx = { version = "0.7", features = ["runtime-tokio-rustls", "sqlite", "chrono", "uuid", "migrate"] }
```

**Files Modified**:
- `migrations/` directory (restructure)
- `src/storage/db.rs` (remove old run_migrations, use sqlx::migrate!)
- `Cargo.toml` (add "migrate" feature)

**Pass Criteria**:
1. ✅ CLI verification: Migrations are tracked
   ```bash
   sqlx migrate info
   # Should show:
   # 20250111000001/installed initial_schema
   # 20250111000002/installed add_indices
   # 20250111000003/installed add_playback_state
   ```

2. ✅ Idempotency test: Running migrations twice doesn't fail
   ```rust
   #[tokio::test]
   async fn test_migrations_idempotent() {
       let temp = tempfile::tempdir().unwrap();
       let db_path = temp.path().join("test.db");

       // First run
       let db1 = Database::new(&db_path).await.unwrap();

       // Second run - should not error
       let db2 = Database::new(&db_path).await.unwrap();

       // Verify migrations table exists
       let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
           .fetch_one(&db2.pool)
           .await
           .unwrap();

       assert!(count > 0);
   }
   ```

3. ✅ Schema evolution test: Add new migration, verify it applies
   ```bash
   sqlx migrate add test_column
   echo "ALTER TABLE episodes ADD COLUMN test_field TEXT;" > migrations/*_test_column.up.sql
   cargo test test_migrations_idempotent
   ```

4. ✅ Rollback test: Down migrations work
   ```bash
   sqlx migrate revert
   # Verify schema rolled back
   ```

**Time Estimate**: 3-4 hours

---

### 1.5 Fix Concurrency Bugs

**Problems**:
1. No lock ordering → potential deadlocks
2. `.unwrap()` on semaphore → panics (src/feed/refresher.rs:34)
3. Race condition in `seek_to()` (src/audio/player.rs:214-217)
4. Locks held across await points

**Solution**: Establish lock ordering, remove panics, fix races.

**Implementation**:

**A. Document Lock Ordering**:
```rust
// src/audio/player.rs - Add at top of file
//! # Lock Ordering Convention
//!
//! To prevent deadlocks, always acquire locks in this order:
//! 1. operation_lock (ensures atomic operations)
//! 2. audio_buffer
//! 3. sink
//! 4. start_position
//! 5. playback_started_at
//! 6. current_episode
//!
//! Never acquire a lower-numbered lock while holding a higher-numbered lock.
//! Never skip levels when acquiring multiple locks.
```

**B. Add Operation Lock for Atomic Operations**:
```rust
// src/audio/player.rs - With new threaded architecture
pub struct AudioPlayer {
    command_tx: mpsc::Sender<AudioCommand>,
    state: Arc<RwLock<AudioState>>,
    operation_lock: Arc<Mutex<()>>,  // ADD THIS - ensures operations are atomic
}

pub async fn seek_to(&self, position: Duration) -> Result<()> {
    // Acquire operation lock first - prevents concurrent seeks
    let _guard = self.operation_lock.lock().await;

    // Now operation is atomic
    let was_playing = self.is_playing().await;

    self.command_tx.send(AudioCommand::Seek(position))?;

    // Wait for seek to complete
    // ... implementation ...

    if !was_playing {
        self.command_tx.send(AudioCommand::Pause)?;
    }

    Ok(())
}
```

**C. Fix Semaphore Panic**:
```rust
// src/feed/refresher.rs
tokio::spawn(async move {
    // Before:
    // let _permit = semaphore.acquire().await.unwrap();  // PANICS

    // After:
    let _permit = semaphore.acquire().await
        .map_err(|e| anyhow::anyhow!("Semaphore closed: {}", e))?;

    Self::refresh_feed(fetcher, db, sub).await
})
```

**D. Don't Hold Locks Across Await** (with new architecture this is less of an issue, but document it):
```rust
// GUIDELINES in src/audio/player.rs
//! # Async Best Practices
//!
//! - Never hold a lock guard across an .await point
//! - Clone data before releasing lock, then await
//! - Use message passing (channels) instead of shared state when possible
//!
//! Example:
//! ```
//! // BAD:
//! let guard = self.data.lock().await;
//! some_async_fn().await;  // Still holding lock!
//!
//! // GOOD:
//! let data = {
//!     let guard = self.data.lock().await;
//!     guard.clone()
//! };  // Lock released
//! some_async_fn().await;
//! ```
```

**Files Modified**:
- `src/audio/player.rs` (add operation_lock, document ordering)
- `src/feed/refresher.rs` (fix unwrap)
- `src/CONCURRENCY.md` (NEW - document concurrency patterns)

**Pass Criteria**:
1. ✅ No unwraps in concurrent code
   ```bash
   rg "\.unwrap\(\)" src/audio src/feed src/queue | grep -v "test" | grep -v "expect"
   # Should return empty
   ```

2. ✅ Stress test: No deadlocks under load
   ```rust
   #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
   async fn stress_test_concurrent_operations() {
       let player = Arc::new(AudioPlayer::new().unwrap());
       let episode_data = load_test_audio();

       player.play_from_memory(Uuid::new_v4(), &episode_data).await.unwrap();

       let mut handles = vec![];

       // Spawn 200 concurrent operations
       for i in 0..200 {
           let p = player.clone();
           let h = tokio::spawn(async move {
               match i % 5 {
                   0 => p.seek_forward(Duration::from_secs(5)).await,
                   1 => p.seek_backward(Duration::from_secs(3)).await,
                   2 => { p.pause().await; Ok(()) },
                   3 => { p.play().await; Ok(()) },
                   _ => { p.set_volume(0.5).await; Ok(()) },
               }
           });
           handles.push(h);
       }

       // Wait for all to complete - should not deadlock
       let timeout = tokio::time::timeout(
           Duration::from_secs(10),
           futures::future::join_all(handles)
       ).await;

       assert!(timeout.is_ok(), "Operations deadlocked or timed out");
   }
   ```

3. ✅ Clippy warnings check
   ```bash
   cargo clippy --all-targets 2>&1 | grep "holding.*lock"
   # Should be empty
   ```

4. ✅ Manual test: Run with concurrent feed refresh + seeking for 5 minutes
   ```bash
   # In one terminal:
   cargo run

   # Exercise: refresh feeds, play episode, seek rapidly, switch episodes
   # Monitor for panics or hangs
   ```

**Time Estimate**: 4-5 hours

---

## Phase 2: TEST COVERAGE

**Goal**: Achieve 70%+ coverage on critical paths

### 2.1 Audio Player Tests

**Target**: 80% coverage, 15+ tests

**Implementation** (`src/audio/player.rs`):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn create_test_player() -> AudioPlayer {
        AudioPlayer::new().unwrap()
    }

    fn load_test_audio() -> Vec<u8> {
        include_bytes!("../../tests/fixtures/test.mp3").to_vec()
    }

    #[tokio::test]
    async fn test_new_player_is_stopped() {
        let player = create_test_player();
        assert!(!player.is_playing().await);
        assert_eq!(player.get_position().await, 0.0);
    }

    #[tokio::test]
    async fn test_play_starts_playback() {
        let player = create_test_player();
        let audio = load_test_audio();

        player.play_from_memory(Uuid::new_v4(), &audio).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert!(player.is_playing().await);
    }

    #[tokio::test]
    async fn test_pause_stops_position_advance() {
        let player = create_test_player();
        let audio = load_test_audio();

        player.play_from_memory(Uuid::new_v4(), &audio).await.unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;

        player.pause().await;
        let pos1 = player.get_position().await;

        tokio::time::sleep(Duration::from_millis(500)).await;
        let pos2 = player.get_position().await;

        assert!((pos1 - pos2).abs() < 0.1, "Position changed while paused");
    }

    #[tokio::test]
    async fn test_resume_continues_playback() {
        let player = create_test_player();
        let audio = load_test_audio();

        player.play_from_memory(Uuid::new_v4(), &audio).await.unwrap();
        player.pause().await;
        let pos_paused = player.get_position().await;

        player.play().await;
        tokio::time::sleep(Duration::from_millis(500)).await;

        let pos_resumed = player.get_position().await;
        assert!(pos_resumed > pos_paused);
    }

    #[tokio::test]
    async fn test_seek_forward_advances_position() {
        let player = create_test_player();
        let audio = load_test_audio();

        player.play_from_memory(Uuid::new_v4(), &audio).await.unwrap();
        let pos1 = player.get_position().await;

        player.seek_forward(Duration::from_secs(10)).await.unwrap();
        let pos2 = player.get_position().await;

        assert!(pos2 >= pos1 + 9.0, "Expected +10s, got {} -> {}", pos1, pos2);
    }

    #[tokio::test]
    async fn test_seek_backward_clamps_at_zero() {
        let player = create_test_player();
        let audio = load_test_audio();

        player.play_from_memory(Uuid::new_v4(), &audio).await.unwrap();
        player.seek_backward(Duration::from_secs(100)).await.unwrap();

        let pos = player.get_position().await;
        assert!(pos >= 0.0, "Position went negative: {}", pos);
        assert!(pos < 1.0, "Position should be near zero");
    }

    #[tokio::test]
    async fn test_stop_clears_episode() {
        let player = create_test_player();
        let audio = load_test_audio();
        let episode_id = Uuid::new_v4();

        player.play_from_memory(episode_id, &audio).await.unwrap();
        assert_eq!(player.get_current_episode().await, Some(episode_id));

        player.stop().await;
        assert_eq!(player.get_current_episode().await, None);
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
    async fn test_speed_changes_affect_playback() {
        let player = create_test_player();
        let audio = load_test_audio();

        player.play_from_memory(Uuid::new_v4(), &audio).await.unwrap();
        player.set_speed(2.0).await;

        assert_eq!(player.get_speed().await, 2.0);
    }

    #[tokio::test]
    async fn test_play_while_playing_stops_previous() {
        let player = create_test_player();
        let audio = load_test_audio();

        let ep1 = Uuid::new_v4();
        let ep2 = Uuid::new_v4();

        player.play_from_memory(ep1, &audio).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        player.play_from_memory(ep2, &audio).await.unwrap();
        assert_eq!(player.get_current_episode().await, Some(ep2));
    }

    // Add 5 more tests...
}
```

**Pass Criteria**:
- ✅ All tests pass: `cargo test audio::player::tests`
- ✅ Coverage >80%: `cargo tarpaulin --packages podcast-tui --lib -- audio::player`
- ✅ Include test audio file: `tests/fixtures/test.mp3` (short clip)

**Time Estimate**: 6-8 hours

---

### 2.2 Database Tests

**Target**: 70% coverage, 25+ tests

**Implementation** (`src/storage/db.rs`):
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    async fn create_test_db() -> (Database, TempDir) {
        let temp_dir = tempfile::tempdir().unwrap();
        let db_path = temp_dir.path().join("test.db");
        let db = Database::new(&db_path).await.unwrap();
        (db, temp_dir)
    }

    fn create_test_subscription() -> Subscription {
        Subscription {
            id: Uuid::new_v4(),
            title: "Test Podcast".to_string(),
            rss_url: "https://example.com/feed.xml".to_string(),
            priority: SubscriptionPriority::Medium,
            created_at: Utc::now(),
            last_refreshed: Utc::now(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_insert_and_get_subscription() {
        let (db, _temp) = create_test_db().await;
        let sub = create_test_subscription();

        db.insert_subscription(&sub).await.unwrap();
        let retrieved = db.get_subscription(sub.id).await.unwrap();

        assert_eq!(retrieved.title, sub.title);
        assert_eq!(retrieved.rss_url, sub.rss_url);
    }

    #[tokio::test]
    async fn test_unique_constraint_on_rss_url() {
        let (db, _temp) = create_test_db().await;
        let sub1 = create_test_subscription();

        db.insert_subscription(&sub1).await.unwrap();

        let mut sub2 = create_test_subscription();
        sub2.id = Uuid::new_v4(); // Different ID
        // Same RSS URL

        let result = db.insert_subscription(&sub2).await;
        assert!(result.is_err(), "Should fail on duplicate RSS URL");
    }

    #[tokio::test]
    async fn test_cascade_delete_episodes() {
        let (db, _temp) = create_test_db().await;
        let sub = create_test_subscription();
        db.insert_subscription(&sub).await.unwrap();

        let episode = Episode {
            id: Uuid::new_v4(),
            subscription_id: sub.id,
            title: "Test Episode".to_string(),
            // ... other fields
        };
        db.insert_episode(&episode).await.unwrap();

        // Delete subscription
        db.delete_subscription(sub.id).await.unwrap();

        // Episode should be gone too
        let result = db.get_episode(episode.id).await;
        assert!(result.is_err(), "Episode should be cascade deleted");
    }

    #[tokio::test]
    async fn test_get_episodes_ordered_by_date() {
        let (db, _temp) = create_test_db().await;
        let sub = create_test_subscription();
        db.insert_subscription(&sub).await.unwrap();

        // Insert episodes in random order
        let ep1 = Episode {
            published_at: Utc::now() - chrono::Duration::days(3),
            // ...
        };
        let ep2 = Episode {
            published_at: Utc::now() - chrono::Duration::days(1),
            // ...
        };
        let ep3 = Episode {
            published_at: Utc::now() - chrono::Duration::days(2),
            // ...
        };

        db.insert_episode(&ep3).await.unwrap();
        db.insert_episode(&ep1).await.unwrap();
        db.insert_episode(&ep2).await.unwrap();

        let episodes = db.get_episodes_for_subscription(sub.id).await.unwrap();

        // Should be newest first
        assert_eq!(episodes[0].id, ep2.id);
        assert_eq!(episodes[1].id, ep3.id);
        assert_eq!(episodes[2].id, ep1.id);
    }

    // Add 20 more tests covering:
    // - Update operations
    // - Search/filter queries
    // - Queue operations
    // - Playback state persistence
    // - Transaction rollback
    // - Concurrent access
    // - Migration edge cases
}
```

**Pass Criteria**:
- ✅ All tests pass: `cargo test storage::db::tests`
- ✅ Coverage >70%: `cargo tarpaulin -- storage`
- ✅ Tests use real SQLite (temp files)

**Time Estimate**: 8-10 hours

---

### 2.3 Integration Tests

**Target**: 10+ end-to-end scenarios

**Setup** (`tests/common/mod.rs`):
```rust
use podcast_tui::*;
use std::path::PathBuf;
use tempfile::TempDir;

pub struct TestApp {
    pub app: App,
    pub temp_dir: TempDir,
}

impl TestApp {
    pub async fn new() -> Self {
        let temp_dir = tempfile::tempdir().unwrap();
        let config_path = temp_dir.path().join("config.json");
        let db_path = temp_dir.path().join("podcast.db");

        let config = Config {
            db_path,
            log_level: "debug".to_string(),
            // ... defaults
        };

        let app = App::new(config).await.unwrap();

        Self { app, temp_dir }
    }

    pub async fn subscribe(&mut self, url: &str) -> Result<Uuid> {
        // Helper to subscribe
    }

    pub fn get_test_audio_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/test.mp3")
    }
}
```

**Tests** (`tests/integration/playback.rs`):
```rust
mod common;
use common::TestApp;

#[tokio::test]
async fn test_subscribe_refresh_play_flow() {
    let mut test_app = TestApp::new().await;

    // 1. Subscribe to feed
    let sub_id = test_app.subscribe("https://example.com/feed.xml")
        .await
        .expect("Failed to subscribe");

    // 2. Refresh feed (mock server or test feed)
    test_app.app.refresh_feeds().await
        .expect("Failed to refresh");

    // 3. Get episodes
    let episodes = test_app.app.state.db
        .get_episodes_for_subscription(sub_id)
        .await
        .expect("Failed to get episodes");

    assert!(!episodes.is_empty(), "No episodes after refresh");

    // 4. Play first episode
    test_app.app.state.play_episode(episodes[0].clone())
        .await
        .expect("Failed to play episode");

    // 5. Verify playback started
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(test_app.app.state.is_playing);

    // 6. Test seeking
    test_app.app.state.audio_player
        .seek_forward(Duration::from_secs(10))
        .await
        .expect("Failed to seek");

    let pos = test_app.app.state.audio_player.get_position().await;
    assert!(pos > 9.0 && pos < 11.0, "Seek position incorrect: {}", pos);
}

#[tokio::test]
async fn test_queue_auto_advance() {
    let mut test_app = TestApp::new().await;

    // Create 3 short test episodes (2 seconds each)
    let ep1 = create_short_test_episode();
    let ep2 = create_short_test_episode();
    let ep3 = create_short_test_episode();

    // Add to queue
    test_app.app.state.queue_manager.add_to_queue(ep1.id, QueuePriority::Medium).await.unwrap();
    test_app.app.state.queue_manager.add_to_queue(ep2.id, QueuePriority::Medium).await.unwrap();
    test_app.app.state.queue_manager.add_to_queue(ep3.id, QueuePriority::Medium).await.unwrap();

    // Play first
    test_app.app.state.play_episode(ep1).await.unwrap();

    // Wait for first to complete and second to start
    tokio::time::sleep(Duration::from_secs(3)).await;

    // Verify second is playing
    let current = test_app.app.state.audio_player.get_current_episode().await;
    assert_eq!(current, Some(ep2.id), "Queue didn't auto-advance");
}

// Add tests for:
// - OPML import/export
// - Download management
// - Search and subscribe
// - Playback state persistence
// - Error recovery
// - Concurrent operations
```

**Pass Criteria**:
- ✅ All integration tests pass: `cargo test --test integration`
- ✅ Tests use real components (no mocks except external APIs)
- ✅ End-to-end timing: Full flow <5 seconds

**Time Estimate**: 8-10 hours

---

### 2.4 Property-Based Tests

**Implementation**:
```toml
[dev-dependencies]
proptest = "1.0"
```

```rust
// src/audio/player.rs
#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn seek_never_negative(seek_back_secs in 0u64..10000) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let player = AudioPlayer::new().unwrap();
                let audio = include_bytes!("../../tests/fixtures/test.mp3");
                player.play_from_memory(Uuid::new_v4(), audio).await.unwrap();

                player.seek_backward(Duration::from_secs(seek_back_secs)).await.unwrap();
                let pos = player.get_position().await;

                prop_assert!(pos >= 0.0);
            });
        }

        #[test]
        fn volume_always_clamped(vol in -1000.0f32..1000.0f32) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let player = AudioPlayer::new().unwrap();
                player.set_volume(vol).await;
                let actual = player.get_volume().await;
                prop_assert!(actual >= 0.0 && actual <= 1.0);
            });
        }

        #[test]
        fn speed_always_positive(speed in -100.0f32..100.0f32) {
            let rt = tokio::runtime::Runtime::new().unwrap();
            rt.block_on(async {
                let player = AudioPlayer::new().unwrap();
                player.set_speed(speed).await;
                let actual = player.get_speed().await;
                prop_assert!(actual > 0.0);
            });
        }
    }
}
```

**Pass Criteria**:
- ✅ Run 1000 iterations: `cargo test --release -- --ignored proptest`
- ✅ No panics or failures
- ✅ 10+ property tests across modules

**Time Estimate**: 4-5 hours

---

## Phase 3: PERFORMANCE OPTIMIZATION

### 3.1 Fix Streaming Lock Contention

**Problem**: Acquires write lock on every chunk (7,500+ times for 60MB file).

**Solution**: Use channels instead of locked buffers.

**Implementation** (`src/audio/stream.rs`):
```rust
pub struct StreamState {
    pub episode_id: Uuid,
    pub content_length: Option<u64>,
    bytes_loaded: Arc<AtomicU64>,
    complete: Arc<AtomicBool>,

    // Replace: buffer: Arc<RwLock<Vec<u8>>>
    // With: Channel-based streaming
    chunk_rx: Arc<Mutex<mpsc::Receiver<Vec<u8>>>>,
}

impl AudioStreamer {
    pub async fn stream_episode(&self, episode_id: Uuid, url: &str) -> Result<StreamState> {
        let (tx, rx) = mpsc::channel(100);  // Buffer 100 chunks (~800KB)

        let response = self.client.get(url).send().await?;
        let content_length = response.content_length();

        let state = StreamState {
            episode_id,
            content_length,
            bytes_loaded: Arc::new(AtomicU64::new(0)),
            complete: Arc::new(AtomicBool::new(false)),
            chunk_rx: Arc::new(Mutex::new(rx)),
        };

        let bytes_loaded = state.bytes_loaded.clone();
        let complete = state.complete.clone();

        // Spawn background task
        tokio::spawn(async move {
            let mut stream = response.bytes_stream();
            let mut total = 0u64;

            while let Some(chunk_result) = stream.next().await {
                match chunk_result {
                    Ok(chunk) => {
                        total += chunk.len() as u64;
                        bytes_loaded.store(total, Ordering::Relaxed);

                        if tx.send(chunk.to_vec()).await.is_err() {
                            break; // Receiver dropped
                        }
                    }
                    Err(e) => {
                        tracing::error!("Stream error: {}", e);
                        break;
                    }
                }
            }

            complete.store(true, Ordering::Relaxed);
        });

        Ok(state)
    }
}
```

**Pass Criteria**:
- ✅ Flamegraph shows <2% time in synchronization
  ```bash
  cargo install flamegraph
  cargo flamegraph --test streaming_bench
  # Open flamegraph.svg, verify no hot spots in lock/unlock
  ```
- ✅ Benchmark: Stream 100MB in <10s on 100Mbps connection
- ✅ Memory: Stays bounded (buffer size × chunk size)

**Time Estimate**: 3-4 hours

---

### 3.2 Optimize AppState Access

**Problem**: Vectors cloned on every access.

**Solution**: Use `Arc<Vec<T>>` for cheap clones.

**Implementation** (`src/app/state.rs`):
```rust
pub struct AppState {
    // Before:
    // pub subscriptions: Vec<Subscription>,
    // pub episodes: Vec<Episode>,

    // After:
    pub subscriptions: Arc<Vec<Subscription>>,
    pub episodes: Arc<Vec<Episode>>,
}

impl AppState {
    pub async fn load_subscriptions(&mut self) -> Result<()> {
        let subs = self.db.get_all_subscriptions().await?;
        self.subscriptions = Arc::new(subs);
        Ok(())
    }

    // Callers get Arc clone (cheap - just pointer copy)
    pub fn get_subscriptions(&self) -> Arc<Vec<Subscription>> {
        Arc::clone(&self.subscriptions)
    }
}
```

**Pass Criteria**:
- ✅ Benchmark: 1000 accesses with 1000 subscriptions <1ms
  ```rust
  #[bench]
  fn bench_subscription_access(b: &mut Bencher) {
      let state = create_state_with_1000_subs();
      b.iter(|| {
          let _ = state.get_subscriptions();
      });
  }
  ```

**Time Estimate**: 2 hours

---

### 3.3 Optimize Position Tracking

**Problem**: get_position() acquires 2 locks, called 60+ times/sec.

**Solution**: Use atomics in AudioState (already in threaded design).

**Implementation**: Built into Phase 1.1 refactor.

**Pass Criteria**:
- ✅ get_position() uses atomics (no locks)
- ✅ Benchmark: 1M calls <10ms

**Time Estimate**: Included in 1.1

---

## Phase 4: CODE QUALITY

### 4.1 Split Large Files

**Target**: No file >300 lines

**Implementation**:
```bash
# Split storage/db.rs (475 lines)
src/storage/
  ├── db.rs           # Connection, migrations (100 lines)
  ├── subscriptions.rs  # Subscription queries (100 lines)
  ├── episodes.rs       # Episode queries (100 lines)
  ├── queue.rs          # Queue queries (80 lines)
  ├── playback.rs       # Playback state (80 lines)
  └── mod.rs           # Re-exports
```

**Pass Criteria**:
- ✅ All files <300 lines: `find src -name "*.rs" -exec wc -l {} + | awk '$1 > 300'`
  (Should be empty)
- ✅ All tests still pass

**Time Estimate**: 3-4 hours

---

### 4.2 Implement Typed Errors

**Problem**: `anyhow::Result` everywhere - can't pattern match.

**Solution**: Use `thiserror`.

**Implementation** (`src/error.rs`):
```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PodcastError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Audio error: {0}")]
    Audio(String),

    #[error("Failed to fetch feed from {url}: {source}")]
    FeedFetch {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    #[error("Failed to parse feed: {0}")]
    FeedParse(String),

    #[error("Episode not found: {0}")]
    EpisodeNotFound(Uuid),

    #[error("Subscription not found: {0}")]
    SubscriptionNotFound(Uuid),

    #[error("Invalid state transition")]
    InvalidStateTransition,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Channel send error")]
    ChannelSend,

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, PodcastError>;

impl From<anyhow::Error> for PodcastError {
    fn from(e: anyhow::Error) -> Self {
        PodcastError::Other(e.to_string())
    }
}
```

**Files Modified**:
- All modules: Replace `anyhow::Result` with `crate::Result`
- Error handling: Use pattern matching where appropriate

**Pass Criteria**:
- ✅ Can match on errors: `match err { PodcastError::Audio(_) => ... }`
- ✅ Zero `anyhow::Result` in public APIs: `rg "anyhow::Result" src --type rust`
- ✅ All tests pass after migration

**Time Estimate**: 4-5 hours

---

### 4.3 Fix All Clippy Warnings

**Target**: Zero warnings

**Implementation**:
```rust
// Fix from_str warnings - implement FromStr trait
use std::str::FromStr;

impl FromStr for DownloadStatus {
    type Err = PodcastError;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "Downloading" => Ok(Self::Downloading),
            "Downloaded" => Ok(Self::Downloaded),
            "NotDownloaded" | _ => Ok(Self::NotDownloaded),
        }
    }
}

// Remove old from_str methods
```

**Pass Criteria**:
- ✅ `cargo clippy --all-targets --all-features` → 0 warnings
- ✅ CI fails on warnings: `cargo clippy -- -D warnings`

**Time Estimate**: 2-3 hours

---

### 4.4 Add Documentation

**Requirements**: All public APIs documented.

**Implementation**:
```rust
//! Audio playback module.
//!
//! Provides a thread-safe audio player with seeking, speed control,
//! and volume management. The player runs in a dedicated thread to
//! avoid blocking async operations.
//!
//! # Example
//!
//! ```no_run
//! use podcast_tui::audio::AudioPlayer;
//! use uuid::Uuid;
//!
//! #[tokio::main]
//! async fn main() -> Result<(), Box<dyn std::error::Error>> {
//!     let player = AudioPlayer::new()?;
//!     let audio_data = std::fs::read("episode.mp3")?;
//!
//!     player.play_from_memory(Uuid::new_v4(), &audio_data).await?;
//!     player.seek_forward(std::time::Duration::from_secs(30)).await?;
//!
//!     Ok(())
//! }
//! ```

/// Play audio from an in-memory buffer.
///
/// This method stops any currently playing audio and starts playback
/// of the provided audio data.
///
/// # Arguments
///
/// * `episode_id` - Unique identifier for the episode
/// * `audio_data` - Raw audio data in a supported format (MP3, AAC, FLAC, Ogg Vorbis)
///
/// # Errors
///
/// Returns [`PodcastError::Audio`] if:
/// - The audio format is unsupported or invalid
/// - The audio device is unavailable
/// - The audio thread has panicked
///
/// # Example
///
/// ```no_run
/// # use podcast_tui::audio::AudioPlayer;
/// # use uuid::Uuid;
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let player = AudioPlayer::new()?;
/// let audio = std::fs::read("episode.mp3")?;
/// player.play_from_memory(Uuid::new_v4(), &audio).await?;
/// # Ok(())
/// # }
/// ```
pub async fn play_from_memory(&self, episode_id: Uuid, audio_data: &[u8]) -> Result<()>
```

**Files Modified**: All modules

**Pass Criteria**:
- ✅ `cargo doc --open` builds without warnings
- ✅ `cargo rustdoc -- -D missing_docs` passes
- ✅ README.md has quick start guide

**Time Estimate**: 6-8 hours

---

## Phase 5: MISSING FEATURES

### 5.1 Queue Auto-Advance

**Implementation** (`src/audio/player.rs`):
```rust
pub struct AudioPlayer {
    command_tx: mpsc::Sender<AudioCommand>,
    state: Arc<RwLock<AudioState>>,
    on_complete_callback: Arc<RwLock<Option<Box<dyn Fn(Uuid) + Send + Sync>>>>,
}

impl AudioPlayer {
    pub async fn set_on_complete(&self, callback: Box<dyn Fn(Uuid) + Send + Sync>) {
        let mut cb = self.on_complete_callback.write().await;
        *cb = Some(callback);
    }
}

// In audio thread:
fn audio_thread(..., on_complete: Arc<RwLock<Option<...>>>) {
    // Detect completion
    if sink.empty() && current_episode.is_some() {
        let episode_id = current_episode.unwrap();

        if let Some(cb) = &*on_complete.blocking_read() {
            cb(episode_id);
        }
    }
}
```

```rust
// src/app/state.rs
impl AppState {
    pub fn new(...) -> Self {
        let queue = queue_manager.clone();
        let db = db.clone();

        audio_player.set_on_complete(Box::new(move |episode_id| {
            let queue = queue.clone();
            let db = db.clone();

            tokio::spawn(async move {
                // Mark episode as played
                db.mark_episode_played(episode_id).await;

                // Play next in queue
                if let Ok(Some(next)) = queue.get_next().await {
                    // Trigger play
                }
            });
        }));

        Self { ... }
    }
}
```

**Pass Criteria**:
- ✅ Integration test: Queue 3 episodes, verify all play sequentially
- ✅ Completion detection within 500ms

**Time Estimate**: 4-5 hours

---

### 5.2 OPML Import/Export

**Implementation** (`src/app/mod.rs`):
```rust
impl App {
    pub async fn import_opml(&mut self, path: &Path) -> Result<usize> {
        let content = tokio::fs::read_to_string(path).await?;
        let outlines = OpmlParser::parse(&content)?;

        let mut count = 0;
        for outline in outlines {
            if let Some(xml_url) = outline.xml_url {
                match self.subscribe_to_feed(&xml_url).await {
                    Ok(_) => count += 1,
                    Err(e) => tracing::warn!("Failed to subscribe to {}: {}", xml_url, e),
                }
            }
        }

        Ok(count)
    }

    pub async fn export_opml(&self, path: &Path) -> Result<()> {
        let subs = self.state.db.get_all_subscriptions().await?;
        let opml = OpmlWriter::create(&subs)?;
        tokio::fs::write(path, opml).await?;
        Ok(())
    }

    async fn subscribe_to_feed(&mut self, url: &str) -> Result<Uuid> {
        // Fetch feed
        let content = self.feed_fetcher.fetch_feed(url).await?;
        let channel = FeedParser::parse_channel(&content)?;

        // Create subscription
        let sub = Subscription::from_channel(channel, url);
        self.state.db.insert_subscription(&sub).await?;

        Ok(sub.id)
    }
}
```

**Pass Criteria**:
- ✅ Import real PocketCasts export
- ✅ Export → import round-trip preserves all data
- ✅ UI integration: Menu item works

**Time Estimate**: 3-4 hours

---

### 5.3 Search Integration

**Implementation** (`src/app/state.rs`):
```rust
impl AppState {
    pub async fn search_podcasts(&self, query: &str) -> Result<Vec<SearchResult>> {
        self.itunes_search.search(query).await
    }

    pub async fn subscribe_from_result(&mut self, result: SearchResult) -> Result<Uuid> {
        let sub = Subscription {
            id: Uuid::new_v4(),
            title: result.title,
            rss_url: result.feed_url,
            artwork_url: result.artwork_url,
            author: result.artist,
            description: result.description,
            created_at: Utc::now(),
            last_refreshed: Utc::now(),
            ..Default::default()
        };

        self.db.insert_subscription(&sub).await?;
        Ok(sub.id)
    }
}
```

```rust
// src/ui/components/search.rs - NEW
pub fn render_search_view(f: &mut Frame, state: &AppState, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),  // Search input
            Constraint::Min(0),     // Results
        ])
        .split(area);

    // Render search input box
    // Render results list
    // Handle selection
}
```

**Pass Criteria**:
- ✅ Search "rust programming" returns results
- ✅ Subscribe from results works
- ✅ Integration test: search → subscribe → refresh → play

**Time Estimate**: 4-5 hours

---

### 5.4 Download Management

**Implementation** (`src/download/manager.rs`):
```rust
impl DownloadManager {
    pub async fn download_episode(&self, episode: &mut Episode) -> Result<()> {
        let download_dir = self.config.download_path.clone();
        std::fs::create_dir_all(&download_dir)?;

        // Update status
        episode.download_status = DownloadStatus::Downloading;
        self.db.update_episode(episode).await?;

        // Stream to file
        let response = self.client.get(&episode.url).send().await?;
        let total_size = response.content_length();

        let filename = format!("{}.mp3", episode.id);
        let file_path = download_dir.join(&filename);
        let mut file = tokio::fs::File::create(&file_path).await?;

        let mut downloaded = 0u64;
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;

            downloaded += chunk.len() as u64;

            // Update progress
            if let Some(total) = total_size {
                let progress = (downloaded as f64 / total as f64) * 100.0;
                self.update_progress(episode.id, progress).await;
            }
        }

        // Update episode
        episode.download_status = DownloadStatus::Downloaded;
        episode.local_path = Some(file_path.to_string_lossy().to_string());
        self.db.update_episode(episode).await?;

        Ok(())
    }

    async fn update_progress(&self, episode_id: Uuid, progress: f64) {
        let mut progress_map = self.progress.write().await;
        progress_map.insert(episode_id, progress);
    }

    pub async fn get_progress(&self, episode_id: Uuid) -> Option<f64> {
        self.progress.read().await.get(&episode_id).copied()
    }
}
```

**Pass Criteria**:
- ✅ Download episode, file appears in filesystem
- ✅ Progress bar updates
- ✅ Can play downloaded episode offline
- ✅ Database updated with local_path

**Time Estimate**: 5-6 hours

---

### 5.5 Keyboard Shortcuts

**Implementation** (`src/app/mod.rs`):
```rust
impl App {
    async fn handle_key(&mut self, key: KeyEvent) -> Result<bool> {
        use KeyCode::*;

        match (key.code, key.modifiers) {
            (Char('q'), _) | (Esc, _) => return Ok(true), // Quit
            (Char(' '), _) => self.state.toggle_playback().await?,
            (Char('n'), _) => self.play_next().await?,
            (Char('p'), _) => self.play_previous().await?,
            (Right, _) => self.state.audio_player.seek_forward(Duration::from_secs(30)).await?,
            (Left, _) => self.state.audio_player.seek_backward(Duration::from_secs(10)).await?,
            (Char('j'), _) | (Down, _) => self.state.next_item(),
            (Char('k'), _) | (Up, _) => self.state.previous_item(),
            (Enter, _) => self.state.select_item().await?,
            (Char('r'), _) => self.refresh_feeds().await?,
            (Char('s'), _) => self.state.set_view(View::Search),
            (Char('d'), _) => self.download_current_episode().await?,
            (Char('+'), _) | (Char('='), _) => self.adjust_speed(0.25).await?,
            (Char('-'), _) => self.adjust_speed(-0.25).await?,
            (Char('?'), _) => self.state.set_view(View::Help),
            _ => {}
        }
        Ok(false)
    }
}
```

**Pass Criteria**:
- ✅ All shortcuts listed in README
- ✅ Help screen (?) shows shortcuts
- ✅ Manual test: Navigate with keyboard only

**Time Estimate**: 3-4 hours

---

### 5.6 Artwork Rendering

**Implementation** (`src/artwork/renderer.rs`):
```toml
[dependencies]
ratatui-image = "1.0"
image = "0.25"
```

```rust
use ratatui_image::{picker::Picker, protocol::StatefulProtocol};
use image::ImageReader;

pub struct ArtworkRenderer {
    picker: Picker,
    protocol: Box<dyn StatefulProtocol>,
}

impl ArtworkRenderer {
    pub fn new() -> Result<Self> {
        let mut picker = Picker::from_termios()?;
        picker.guess_protocol();
        let protocol = picker.new_protocol();

        Ok(Self { picker, protocol })
    }

    pub fn render(&mut self, f: &mut Frame, area: Rect, image_path: &Path) -> Result<()> {
        if !image_path.exists() {
            self.render_placeholder(f, area);
            return Ok(());
        }

        let img = ImageReader::open(image_path)?
            .decode()
            .context("Failed to decode image")?;

        let dyn_img = ratatui_image::DynamicImage::from(img);
        let image = ratatui_image::Image::new(&dyn_img);

        f.render_widget(image, area);

        Ok(())
    }

    fn render_placeholder(&self, f: &mut Frame, area: Rect) {
        let placeholder = Paragraph::new("[No Artwork]")
            .block(Block::default().borders(Borders::ALL));
        f.render_widget(placeholder, area);
    }
}
```

**Pass Criteria**:
- ✅ Artwork displays in Kitty terminal
- ✅ Artwork displays with Sixel support
- ✅ Graceful fallback in other terminals
- ✅ Manual test across terminals

**Time Estimate**: 4-5 hours

---

## Phase 6: ARCHITECTURE

### 6.1 Event System

**Implementation** (`src/events.rs`):
```rust
use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub enum AppEvent {
    KeyPress(KeyEvent),
    PlaybackStarted(Uuid),
    PlaybackCompleted(Uuid),
    PlaybackError(String),
    PositionUpdate(Duration),
    FeedRefreshCompleted,
    DownloadProgress(Uuid, f64),
    Quit,
}

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<AppEvent>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.tx.subscribe()
    }

    pub fn emit(&self, event: AppEvent) {
        let _ = self.tx.send(event);
    }
}
```

**Pass Criteria**:
- ✅ UI updates on events, not polling
- ✅ Event latency <16ms (60 FPS)

**Time Estimate**: 6-8 hours

---

## VERIFICATION & CI

### Update CI Pipeline

```yaml
# .github/workflows/ci.yml
quality-gates:
  steps:
    - name: Tests
      run: cargo test --all-features --all-targets

    - name: Clippy (no warnings)
      run: cargo clippy --all-targets --all-features -- -D warnings

    - name: Coverage
      run: |
        cargo tarpaulin --out Xml
        coverage=$(python3 -c "import xml.etree.ElementTree as ET; print(ET.parse('cobertura.xml').getroot().get('line-rate'))")
        if (( $(echo "$coverage < 0.70" | bc -l) )); then
          echo "Coverage $coverage below 70%"
          exit 1
        fi

    - name: Docs
      run: cargo doc --all-features --no-deps

    - name: Benchmarks
      run: cargo bench --no-fail-fast
```

---

## FINAL CHECKLIST

### Critical Bugs (Phase 1)
- [ ] AudioPlayer is Send + Sync
- [ ] No clippy arc_with_non_send_sync warnings
- [ ] Seeking <100ms for any position
- [ ] Audio actually plays (manual test)
- [ ] Database uses sqlx migrations
- [ ] No unwrap() in concurrent code
- [ ] Stress test passes (100 concurrent ops)

### Testing (Phase 2)
- [ ] Total coverage >70%
- [ ] Audio player coverage >80%
- [ ] Database coverage >70%
- [ ] 10+ integration tests
- [ ] Property tests (1000 iterations)

### Performance (Phase 3)
- [ ] No lock contention in streaming
- [ ] Position tracking uses atomics
- [ ] Flamegraph clean

### Code Quality (Phase 4)
- [ ] No files >300 lines
- [ ] Zero clippy warnings
- [ ] Typed errors (no anyhow in public API)
- [ ] All public APIs documented

### Features (Phase 5)
- [ ] Queue auto-advances
- [ ] OPML import/export
- [ ] Search works
- [ ] Downloads work
- [ ] Keyboard shortcuts
- [ ] Artwork renders

### Architecture (Phase 6)
- [ ] Event-driven UI
- [ ] Proper state machine

---

## TIME ESTIMATE

| Phase | Hours | Days (6h/day) |
|-------|-------|---------------|
| Phase 1: Critical Bugs | 25-31 | 4-5 |
| Phase 2: Testing | 26-33 | 4-6 |
| Phase 3: Performance | 5-6 | 1 |
| Phase 4: Code Quality | 15-20 | 2-3 |
| Phase 5: Features | 23-29 | 4-5 |
| Phase 6: Architecture | 6-8 | 1-2 |
| **TOTAL** | **100-127** | **17-21** |

---

## SUCCESS METRICS

### Must Have
- ✅ Zero critical bugs
- ✅ Audio plays and seeks correctly
- ✅ Test coverage >70%
- ✅ Zero clippy warnings

### Should Have
- ✅ All features implemented
- ✅ Performance benchmarks met
- ✅ Documentation complete

### Nice to Have
- ✅ Event-driven architecture
- ✅ Property-based tests
- ✅ Artwork rendering
