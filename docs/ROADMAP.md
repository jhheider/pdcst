# pdcst roadmap

A fast, keyboard-driven, single-binary **terminal podcast player**, pure Rust,
built for one user (Jacob) to actually use every day. This is the canonical plan;
it is written to survive a context reset, so it is deliberately complete.

## The deal (read this first)

pdcst exists to reproduce the one PocketCasts feature that made it
indispensable on a commute: **the auto-managed Up Next queue.** Everything else
is table stakes. Concretely, the queue must:

1. **Auto-fill from subscriptions at publish time.** When a refresh finds a new
   episode, that subscription either **pushes** (appends to the end) or
   **unshifts** (prepends to the top) the episode onto the queue. Push vs unshift
   is configurable **per subscription**, with a global default.
2. **Keep filled to a programmable max depth.** The queue tops itself up to N
   episodes automatically; the user never hand-fills it. As episodes finish and
   drop off, and as new ones publish, it stays full with zero manual work.
3. **Be a smart queue: no two adjacent episodes of the same podcast.** When
   auto-filling, interleave so you do not get a run of one show back to back.
4. **Never clobber the current item.** The currently-playing episode is
   protected: auto-fill and re-ordering happen around it, never over it.
5. **Track listen state for every episode.** Unplayed / in-progress (+ resume
   position) / played, persisted and shown everywhere. Auto-add only unplayed
   episodes; completing an episode marks it played and advances the queue.

That is the product. A player that nails this beats hullcaster (the existing
Rust competitor) for Jacob because it is the thing he actually missed. Sync,
discovery, and a settings-heavy UI are explicitly NOT the point (sync is
impossible in a local app anyway). The edge is: mine, fast, self-contained,
does not crash, and keeps my queue full.

## Where we are now (2026-07-12)

Backend is genuinely solid and tested; the UI is wired but has real gaps; the
auto-queue does not exist yet (only a basic advance-on-complete). A three-lens
audit (code / product / TUI-UX) is summarized at the bottom.

**Merged to `main`:**
- **Workspace**: `crates/podcast-tui` (binary) + `crates/wsola` (library),
  independently versioned, deps + metadata hoisted to `[workspace.*]`.
- **`wsola`**: a real, documented, thiserror, property-tested pure-Rust WSOLA
  time-stretch library (pitch-preserved tempo). Its own crate; will publish
  independently. Brief: jhheider/briefs `pure-rust-time-stretch`.
- **Deps maximized**: ratatui 0.30, crossterm 0.29, reqwest 0.13, sqlx 0.9,
  quick-xml 0.41, toml 1.1, dirs 6, rodio 0.22. Dropped unused `image` +
  `ratatui-image`. TLS is rustls + **ring** only (no openssl/native-tls/aws-lc).
- **Reachable onboarding**: `--import` / `--export` OPML CLI flags (the app
  previously had no way to add a podcast). Verified against a real 17-feed
  Pocket Casts OPML.
- **CI**: thin `jhheider/rust-ci@v1` callers (ci/style/audit) on the self-hosted
  `studio-pdcst` runner (private repo). The runner image carries `libasound2-dev`
  + `pkg-config` for rodio; see `jhheider/gha-runner` `images/pdcst`. Toolchain
  is rustc 1.97.

**Open PR (`audio/async-rewrite`, PR #4):**
- **Stage 1 - rodio 0.22 migration**: `DeviceSinkBuilder`/`MixerDeviceSink` +
  `Player`; real `Player::try_seek` (deleted the O(n) re-decode seek and the
  buffer it needed); position from the player. rodio was the last held dep.
- **Stage 2 - pitch-corrected speed**: `WsolaSource<S>` - a generic rodio
  `Source` that time-stretches its inner source, reads tempo lock-free from a
  shared `AtomicU32` (live speed changes), tracks position in **source time**
  (not stretched output time) in a shared `AtomicU64`, and resets on seek. Speed
  now flows through wsola, not rodio's pitch-shifting `set_speed`.

**NOT yet done:** the whole UX layer (Phase B) and the auto-queue (Phase C).
De-freeze and resume (Phase A Stages 3-4) are now done; the last Phase A item is
Jacob's real-audio validation.

## Phases

Ordering rationale: finish the audio path (A) so daily listening works; make the
app usable at all (B) - you currently cannot subscribe from inside it; then build
the deal (C) on top of a working queue + listen state; then keep it fed (D);
then ship it (E). Listen-state tracking (part of the deal) starts in A/B because
it is foundational.

### Phase A - finish the audio rewrite (in flight on PR #4)

- [x] Stage 1: rodio 0.22 migration.
- [x] Stage 2: wsola pitch-corrected speed.
- [x] **Stage 3 - de-freeze the UI.** Done. `AppState::play_episode` now spawns
      the fetch (`load_and_play`) as a tokio task and returns immediately, so the
      event loop never blocks on the download; it shows a persistent "Loading..."
      status that `PlaybackStarted` clears (a failed fetch publishes
      `PlaybackError` -> error modal). The `sleep(2s)` status-clear pattern is
      gone: status messages carry an expiry (`StatusMessage`), cleared at render
      time on the 50ms tick via `AppState::expire_status`. The auto-advance task
      in `app/mod.rs` now shares the single play path (`state::load_and_play`), so
      it streams to disk exactly like a manual play.
- [x] **Stage 3b - progressive stream-to-disk.** Done (resolves the streaming
      decision below). `audio/stream.rs` is now a `DiskStream` that downloads an
      episode to a temp file in the background while a `GrowingFile` reader feeds
      the decoder off the same file, blocking only when a read runs ahead of the
      downloaded frontier (real EOF on completion, error on failure). Playback
      starts after a `PREBUFFER_BYTES` (256 KiB) prebuffer - soonest start, no UI
      block, nothing buffered wholesale in memory. Downloaded episodes play
      straight from their file (`AudioPlayer::play_from_file`); remote ones use
      `play_stream`; both share one generic `start_playback<R: Read + Seek>`. Temp
      files live under `config.stream_cache_dir()` and are purged as episodes
      change. Caveat (Jacob's ears): a resume seek into a not-yet-downloaded
      region waits for the download to reach it, and a mid-stream download failure
      currently ends the episode like a normal completion.
- [x] **Stage 4 - resume.** Done. `AppState::save_progress` persists the
      per-episode position AND the singleton `playback_state` (episode, rate,
      volume) on the 1s `PlaybackPosition` tick, on pause, and on quit.
      `play_episode` reads `episode.playback_position_seconds` and passes it as
      the `start` arg (through to `WsolaSource`/`try_seek`).
      `restore_playback_state` runs on launch: it reloads the last episode
      (shown, not auto-played) and restores rate/volume; pressing play resumes
      from the saved position. First slice of per-episode listen state.
- [x] **Stage 3c - disk retention.** Done. `retention::RetentionManager` keeps
      on-disk audio bounded. Config knobs (`delete_on_finish` default true,
      `max_cache_episodes` default 50, `max_cache_megabytes` default 4096; a cap
      of 0 = unlimited). Delete-on-finish removes a finished episode's download in
      the completion path; the size caps evict least-recently-played downloads
      first (never the currently-playing one) and reconcile rows whose file
      vanished. Enforced at startup (which also `stream::purge_all`s stale stream
      temp files) and re-swept every 6h so a long-open session cannot drift over.
- [ ] **Real-audio validation (Jacob's ears).** The DSP is unit/property tested,
      but CI cannot hear the live path. Once PR #4 merges: run it, import the
      OPML, play an episode, confirm 1.5x sounds sped-up (not chipmunk), seek
      works, resume works.

### Phase B - make it usable (the UX pass)

Reference: PocketCasts for Mac is installed (Electron, DOM-inspectable) - mine it
for the interaction model and keybindings. Informed by the `product-designer` and
`tui-ux` agents (2026-07-12), which converged: the Phase B bar is not "usable app"
in the abstract, it is "the Up Next queue is inspectable, operable, and
trustworthy, and I can never fall into a broken input state." Sequenced into PRs,
tier-1 (the gate into Phase C) first.

**PR 1 - input routing + view model (done, this PR):**
- [x] **Text-entry no longer collides with global keys.** Quit is a
      `should_quit` flag set in `handle_key_event` (not a raw `q` match in the run
      loop), so a literal `q` while typing no longer quits the app; Ctrl-C is the
      always-available hard quit. The search gate routes every printable key
      (digits and `q` included) into the box; only Esc/Enter escape.
- [x] **Unified view model.** Numbers 1-4 = Subscriptions/Queue/Search/Settings;
      Tab/Shift-Tab cycle exactly those four (`next_top_view`/`prev_top_view`);
      Episodes is a drill-down (Enter opens, Esc backs out), not a tab. Help modal
      and footer reconciled to match.
- [x] **List navigation works in every view.** `next_item`/`goto_bottom`/
      `page_down` use one `max_index()` over the current view's list, fixing the
      `_ => 0` no-ops in Queue/Search.

**PR 2 - scrolling + display (done):**
- [x] **List scrolling.** `ListState` + `render_stateful_widget` is now the one
      selection+scroll mechanism (`render` takes `&mut AppState`; `sync_list_selection`
      clamps + points it each frame; reset in `set_view`). Hand-rolled
      `bg(DarkGray)` highlight replaced with `Modifier::REVERSED` (theme-independent).
      The dead `scroll_offset` field is gone.
- [x] **State visibility.** Now-playing `>` marker in Episodes/Queue;
      unplayed/in-progress/played markers (` `/`~`/`x`) per row; elapsed/duration
      (`12:34 / 45:00`) in the playback bar; persistent "Up Next: N" in the footer
      (queue kept fresh on any `QueueUpdated` + at startup).
- [x] **Panic-safe terminal restore** via a `TerminalGuard` Drop; dropped mouse
      capture; first-run guidance on an empty Subscriptions view. A `TestBackend`
      render smoke test now covers every view (empty, populated, modals).
- [ ] NO_COLOR (strip all fg/bg) deferred; `REVERSED` already makes the selection
      theme-independent, which was the load-bearing part.

**PR 3 - queue + search operability (done):**
- [x] **Subscribe from the running app.** A `SearchFocus` (Input vs Results):
      typing runs in the box, Enter runs the query and moves focus to the results
      list, Enter there calls `subscribe_from_search_result`; Esc steps back to
      the box then exits. The active pane is border-highlighted. Closes the #1 gap.
- [x] **Operable queue.** `select_item` Queue arm plays the selected item; `x` in
      the Queue view removes it (`remove_selected_from_queue`). Reorder deferred
      (smart-interleave is the Phase C ordering story).
- [x] **Skip fixed.** `play_next_in_queue` now drops the current episode (the
      queue head) before advancing, so `n` skips instead of replaying it. The
      completion path already removed+advanced; folding both into one
      `QueueManager` method is left for the Phase C queue rework.
- [x] **Un-freeze refresh + search (PR 4).** `refresh_selected_subscription` /
      `refresh_all_subscriptions` now spawn off the event loop (the refresher's
      FeedRefresh* events drive the reload in `handle_state_event`:
      `FeedRefreshCompleted` reloads subscriptions + the viewed episode list and
      reports "Refreshed: N new"). Search likewise: `start_search` spawns and
      publishes a new `SearchCompleted { results }` / `SearchFailed` event that
      the handler applies (results + focus). No network call blocks the UI now.

Tests: `queue_ops` integration tests cover the skip and remove behavior; the
render smoke tests now also cover the search view. Shared `tests/common` builds a
wired `AppState` over a temp DB.

**Deferred / cut** (agents agreed): `Modal::Confirm` for delete (streaming-first,
`delete_on_finish` already reclaims; ceremony for a single user); download-progress
UI; manual queue reorder (smart-interleave is the ordering story); adopting the
dead `PlaybackPanel`/alt keymap (do the subtraction, skip the adoption).
- [ ] **Delete the dead subsystems** (subtraction only): 30-variant `PodcastError`
      (app uses `anyhow`), `AppEvent`/`from_key_event` (second unused keymap),
      `ui/components/*`. Its own no-behavior-change PR.
- [ ] Add a `TestBackend` smoke test per view once `ListState` lands, targeting
      the input-routing/queue-operability paths where showstoppers cluster.

### Phase C - the auto-queue (THE DEAL)

Build the five requirements from the top of this doc. This is the reason the app
exists; do not treat it as a nice-to-have.

Design sketch (confirm details as you build):
- **Schema.** A `queue` table (position-ordered, references episode); per-episode
  listen state on `episodes` (played, position_seconds already exist; add
  in-progress/added-to-queue as needed); per-subscription auto-add config
  (`auto_add: none|top|bottom`) + global settings (`queue_max_depth`, global
  default auto-add, smart-shuffle on/off).
- **Publish-time hook.** When `FeedRefresher` ingests a new, unplayed episode for
  a sub with auto-add enabled, enqueue it (push or unshift per the sub's rule),
  respecting `queue_max_depth` (do not exceed it) and never displacing the
  current item.
- **Smart interleave.** When inserting (or on a re-balance pass), avoid two
  adjacent episodes of the same podcast: insert at the nearest legal position, or
  reorder the not-yet-current tail. Keep the algorithm simple and deterministic;
  the current item is fixed.
- **Completion -> advance.** On `PlaybackCompleted`: mark played, remove from
  queue, advance to the next (a basic version already exists in `app/mod.rs`;
  fold it into the queue manager and make it respect listen state).
- **Listen state everywhere.** Episode lists show unplayed/in-progress/played;
  auto-add only unplayed; resume uses the saved position (Phase A Stage 4).
- Depends on: a working editable queue (Phase B), resume/listen-state (Phase A),
  and refresh (Phase D) as the trigger source.

### Phase D - refresh scheduling

The auto-queue is only as good as its trigger. Add background feed refresh:
on-launch refresh, then periodic (programmable interval), concurrency-bounded
(`FeedRefresher` already does semaphore-bounded concurrent refresh). New episodes
found here are what feed Phase C. Surface refresh state in the UI (Phase B's
feedback work).

### Phase E - distribution and static linking

Make it a portable single binary and ship it.

- [ ] **Static sqlite (easy win).** Switch the sqlx feature from `sqlite` to
      `sqlite-bundled` - compiles the SQLite amalgamation and links it
      statically, so there is no system `libsqlite3` dependency. This is the one
      accepted C compile (a single well-contained amalgamation, universally
      used); it is what pkgx does for portability. One-line change.
- [ ] **Static ALSA: blocked, document the reality.** `alsa-sys` 0.4 hardcodes
      `pkg_config...statik(false)`, and ALSA `dlopen`s its plugins at runtime, so
      a clean static `libasound` is not feasible without forking alsa-sys - and
      even then the plugin story is fragile. The honest portable-Linux path is
      **musl + bundled sqlite + dynamic libasound** (one documented runtime dep,
      `apt install libasound2` / present on any desktop). Revisit only if the
      engine ever changes. On macOS this is moot (CoreAudio, no ALSA).
- [ ] **Release workflow.** A `jhheider/rust-ci` `release.yml` caller (bin-name,
      target matrix, optional Homebrew tap), like edikt/penknife. Ties into the
      musl/static story above.
- [ ] **Version reset.** `podcast-tui` is at `1.0.0`, which overclaims (the audit
      put it at ~35% of a daily-usable player). Reset to `0.x` before any real
      release. `wsola` versions independently (starts 0.1.0, publishes on its own
      cadence).

## Cross-cutting constraints (the ethos)

- **Pure Rust, lean deps, no C build systems** - except the SQLite amalgamation
  (Phase E) as a deliberate, contained exception for portability.
- **TLS: rustls + ring only.** Never openssl, native-tls, or aws-lc. reqwest is
  `default-features=false` + `rustls-no-provider` + `webpki-roots`, with ring
  installed once at startup via `podcast_tui::ensure_crypto_provider()` (called
  at every reqwest client site so tests, which never run `main`, work).
- **Design bar: a keyboard-first TUI** in the class of lazygit/k9s/newsboat.
  Reviewed with the user-level `tui-ux` agent.
- **CI on the self-hosted runner** (private repo); keep it green under
  fmt + clippy `-D warnings` + tests + the no-em-dash style gate. Match the local
  toolchain to the runner (1.97) so lints agree.

## Key technical reference (for a cold start)

- **Layout**: Cargo workspace. `crates/podcast-tui` (bin), `crates/wsola` (lib).
  Deps hoisted in root `Cargo.toml` `[workspace.dependencies]`.
- **Audio** (`crates/podcast-tui/src/audio/`):
  - `player/mod.rs`: `AudioPlayer` (Send+Sync handle) -> `mpsc` -> a dedicated
    std thread that owns the `!Send` `MixerDeviceSink` and current `Player`.
    State via atomics; changes published on the `EventBus`. rodio 0.22:
    `DeviceSinkBuilder::open_default_sink()`, `Player::connect_new(mixer)`,
    `player.try_seek`, `player.get_pos`, `player.empty`. Speed does NOT use
    `player.set_speed` (that pitch-shifts) - it goes through wsola.
  - `wsola_source.rs`: `WsolaSource<S: Source>` wraps the decoder; tempo from a
    shared `Arc<AtomicU32>` (f32 bits), source-time position to
    `Arc<AtomicU64>`. `try_seek` -> inner seek + `TimeStretch::reset` + reposition.
  - `stream.rs`: progressive stream-to-disk. `AudioStreamer::open_stream` starts
    a `DiskStream` (background download to a temp file under
    `config.stream_cache_dir()`), waits for a 256 KiB prebuffer, and returns a
    `GrowingFile` - a blocking, seekable reader that waits when it runs ahead of
    the downloaded frontier. Feeds `AudioPlayer::play_stream`; downloaded
    episodes use `play_from_file`. `AppState::load_and_play` picks between them.
- **wsola API**: `TimeStretch::new(sr, ch)` / `with_config`, `set_tempo`,
  `push(&[f32])`, `pull(max) -> Vec<f32>`, `flush`, `reset`; free
  `stretch(samples, sr, ch, tempo)`. Interleaved f32. Streaming output is
  bit-identical to one-shot (property-tested).
- **DB** (`storage/`): sqlx + sqlite, migrations in
  `crates/podcast-tui/migrations` (`migrate!("./migrations")`). Tables:
  subscriptions, episodes (has `playback_position_seconds`, `played`),
  `playback_state` (singleton id=1), queue. `playback.rs` has the
  built-but-unwired resume methods.
- **Events**: `app/events.rs` `EventBus` (tokio broadcast) + `StateEvent`. The
  run loop (`app/mod.rs`) is `tokio::select!` over the bus + a ~50ms key poll -
  event-driven, not a busy poll (the old EVENT_ARCHITECTURE doc was wrong).
- **Onboarding**: `--import <opml>` / `--export <opml>` in `main.rs`.
- **CI/runner**: `jhheider/gha-runner` service `pdcst` builds `images/pdcst`
  (stock runner + libasound2-dev + pkg-config); `just up` registers it.

## Open decisions

- ~~Progressive streaming vs download-then-play.~~ RESOLVED 2026-07-12:
  progressive stream-to-disk (play after a 256 KiB prebuffer while the rest
  downloads). See Phase A Stage 3b.
- Auto-add default: push or unshift? Per-sub UI shape.
- Smart-interleave algorithm: insert-at-nearest-legal vs periodic re-balance.
- Whether to pursue a musl fully-static Linux build now or after the product is
  proven.

## The audit (three-lens), for reference

A code-critical, a product, and a TUI-UX review of the pre-work state agreed:
this is not a shell and not near-done - it is well-built vertical slices that
were never wired together, with the wiring layer (untested) holding every
showstopper. Genuinely done toward a daily-usable player was ~35%. Verdict:
**finish, rescoped** to "a fast local keyboard player I actually use" - and the
user's directive since then makes the **auto-queue the centerpiece of that
goal**, not an afterthought. Cut for good: sync (impossible locally), in-terminal
artwork rendering (the rabbit hole - the renderer stays a stub), discovery, DSP
effects beyond pitch-corrected speed, and the word "parity."
