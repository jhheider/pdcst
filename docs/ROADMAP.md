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

## Where we are now (2026-07-13)

Feature-complete and shipped. Every phase below (A-E) is done; the repo is
public, on hosted CI, releasing tagged binaries + a Homebrew tap. Latest release
**v0.3.2**. The historical phase log is kept below for a cold start; this is the
current-state summary.

- **The core loop closes end to end**, validated in real daily use: subscribe
  (in-app iTunes search or `--import` OPML) -> hands-off background refresh
  (`feed/scheduler.rs`) -> auto-managed Up Next -> play with pitch-corrected
  speed, seek, and cross-session resume (survives quit *and* SIGTERM/SIGHUP).
- **The deal (auto-queue) is built and tested** (Phase C): auto-fill at publish
  time, per-sub push/unshift, programmable max depth, smart interleave,
  never-clobber-current, listen-state tracking.
- **`wsola`** is its own crate, **published to crates.io** (0.1.0), consumed by
  pdcst via a rodio `Source`; re-confirmed perfect in real playback (1.5x, no
  chipmunk). Brief: jhheider/briefs `pure-rust-time-stretch`.
- **Polish + reliability passes** (v0.3.0-v0.3.2): two-pane ranger-style Library,
  episode cards, subscription counts, honest per-feed errors + Atom fallback +
  feed recovery (`f`), abrupt-exit save, and **feed-text normalization** (HTML
  entities + ZWJ emoji stripped at ingest, v0.3.2 - brief
  `pdcst-emoji-ghost-glyphs`).
- **Distribution** (Phase E): public repo, hosted CI (fmt + clippy `-D warnings`
  + tests + the no-em-dash style gate), and a `jhheider/rust-ci` `release.yml`
  caller that on `release: published` builds `x86_64-unknown-linux-gnu` +
  `aarch64`/`x86_64-apple-darwin` binaries, publishes `wsola` (skipping
  already-published versions; pdcst itself stays unpublished), and refreshes
  `Formula/pdcst.rb` in `jhheider/homebrew-tap`.

## What's left (product review 2026-07-13)

A product-designer pass over the shipped app confirmed the thesis is met and the
loop closes, but flagged **two real gaps plus a subtraction PR** before the
roadmap is honestly "done, maintenance-only." None is scope creep; each serves
the single-user daily-driver thesis. Ordered by leverage:

1. **[x] Fix false-completion on a mid-stream network drop (reliability).**
   DONE. `GrowingFile::failure()` hands the audio thread a `StreamFailure` handle
   (a cheap clone of the stream's `failed` flag) that outlives the reader once the
   decoder consumes it; the run-dry check (`audio/player/mod.rs`) now routes
   through `run_dry_event`, emitting `PlaybackError` (position kept, episode not
   marked played, `auto_advance` ignores it) instead of `PlaybackCompleted` when
   the download failed mid-stream. So a cellular blip mid-episode no longer marks
   it done and skips ahead. Unit-tested (`run_dry_event`, `failure_handle_*`).
2. **[ ] Make config reachable at a default path.** `Config::load_default`
   (`config.rs:7`) never reads a file - it always returns `Config::default()`,
   and `save_to_file` is called nowhere. So the dials that tune *the auto-queue
   itself* (`queue_max_depth`, `auto_queue_to_top_default`, `smart_interleave`,
   `auto_refresh_interval_minutes`, retention caps) can only be changed by
   hand-authoring a TOML and passing `--config <file>` every launch; the Settings
   view shows them read-only. Fix: write a commented default TOML on first run
   (and/or `--print-config`) and have `load_default` read it. Keep it a plain
   file - **no in-app editor** (that re-imports the settings-heavy UI that was
   correctly cut).
3. **[x] Subtraction PR (no behavior change).** DONE.
   - Removed the whole **`artwork/` subsystem (~518 lines)**: `ArtworkManager`,
     cache/fetcher/protocol/renderer, its construction + `load_cache_from_disk()`
     startup I/O, and the `artwork_manager` fields on `Services`/`AppState`. The
     inert `Subscription.artwork_url`/`artwork_path` *data* columns are kept (they
     do no I/O, and dropping DB columns would be a schema change, not "no behavior
     change") - only the live-but-dead subsystem is gone.
   - Deleted the **dead/misleading config knobs**: `keybindings`/`KeyBindings`,
     `theme`/`Theme`, `show_artwork`, `artwork_protocol`/`ArtworkProtocol`,
     `artwork_dir`, `trim_silence`, and the unused-and-inverted
     `skip_forward_seconds`/`skip_backward_seconds` (input.rs hardcodes 10s/30s).
   - The originally-listed 30-variant `PodcastError`, second
     `AppEvent`/`from_key_event` keymap, and `ui/components/*` were **already
     gone** (removed in an earlier pass); nothing left to cut there.

**Verdict:** one or two real gaps (items 1-2), then the subtraction PR (item 3),
after which the roadmap is genuinely done for its purpose. Explicitly still cut:
config-editing UI, artwork rendering, manual queue reorder, download-progress UI,
sync, discovery, themes.

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
      region waits for the download to reach it. (A mid-stream download failure
      used to end the episode like a normal completion; fixed - see "What's left"
      item 1, it now surfaces as a `PlaybackError` with the position kept.)
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

- [x] **PR 1 - schema + config.** Per-subscription `auto_queue_to_top` column
      (migration `20250712000001`) + `Subscription` field; global `Config`
      settings `queue_max_depth` (20), `auto_queue_to_top_default`,
      `smart_interleave`. Per-episode listen state (played, position) already
      exists from Phase A; the in-progress marker shipped in Phase B PR 2.
- [x] **PR 2 - publish-time enqueue hook + smart interleave.** `refresh_feed`
      now detects genuinely-new episodes (via `Database::episode_exists`, which
      also fixes the inflated `new_episodes` count) and, for an `auto_queue`
      feed, calls `QueueManager::auto_enqueue`. That pushes/unshifts per the
      sub's rule, respects `queue_max_depth` (skips when full), never displaces
      the current item (reads `playback_state` to protect the head, inserting via
      the new `insert_into_queue_at`), and smart-interleaves via the pure
      `nearest_legal_position` (no two adjacent episodes of the same podcast).
      Unit-tested (`nearest_legal_position`) + integration-tested (`auto_enqueue`
      push/unshift/max-depth/never-clobber/interleave).
- [x] **PR 3 - per-sub toggle UI + settings view.** `A` in the Subscriptions
      view cycles a feed's auto-queue off -> bottom -> top -> off
      (`cycle_selected_auto_queue` + `Database::update_subscription_auto_queue`);
      the row shows a `Qv`/`Q^` marker and the Settings view shows the global
      auto-queue config. This is what turns the feature on end-to-end (auto_queue
      defaults off). Cycle + DB round-trip tested.
- [x] **PR 4 - completion fold.** `QueueManager::advance(finished_id,
      mark_played)` is now the single path for both natural completion (mark
      played) and manual skip (`n`, which also marks played so a skip is not
      auto-re-queued) plus retry-on-failure (mark_played=false). It marks played,
      removes from the queue, and returns the next episode; the caller plays it.
      `auto_advance.rs` and `play_next_in_queue` both call it. Behaviour-preserving
      refactor + a first unit test on the completion/skip logic.

**Phase C is complete.** The five requirements of "the deal" are met: auto-fill
at publish time (PR 2), programmable max depth (PR 2), smart interleave (PR 2),
never-clobber-current (PR 2), and listen-state tracking (Phase A + B markers,
auto-add-only-unplayed via `is_new`). Live validation (mark a feed with `A`,
refresh, watch Up Next fill) is Jacob's to run.
- Depends on: a working editable queue (Phase B, done), resume/listen-state
  (Phase A, done), and refresh (Phase D) as the trigger source.

### Phase D - refresh scheduling (done)

The auto-queue's trigger. `feed::spawn_auto_refresh` (in `feed/scheduler.rs`,
wired in `App::new`) refreshes every feed once ~3s after launch and then every
`config.auto_refresh_interval_minutes` (default 60; `0` disables the periodic
pass, launch refresh still runs). It reuses the semaphore-bounded
`FeedRefresher::refresh_all`, so it publishes the same `FeedRefresh*` events (UI
progress) and runs the Phase C auto-enqueue hook - so new episodes now land in
Up Next on their own, no manual `R`. The Settings view shows the interval.

- [x] on-launch + periodic background refresh, concurrency-bounded.
- [x] feeds Phase C (auto-enqueue runs inside `refresh_feed`).
- [x] refresh state surfaced (FeedRefresh events -> status; interval in Settings).
- Note: a manual `R` racing the auto-refresh on the same feed is a narrow,
  accepted race (both dedup by guid; worst case a transient double-enqueue).

### Phase E - distribution and static linking

Make it a portable single binary and ship it.

- [x] **Static sqlite - already done (upstream).** sqlx 0.9 reorganized its
      features so the `sqlite` feature now implies `sqlite-bundled`
      (`sqlx-sqlite/bundled` -> `libsqlite3-sys` with `bundled`). pdcst already
      uses `features = ["sqlite", ...]`, so it compiles the SQLite amalgamation
      and links it statically today: `otool -L` / `ldd` show no `libsqlite3`. The
      "one-line change" is obsolete. No action needed.
- [x] **Static ALSA: blocked, so ship glibc + dynamic libasound.** `alsa-sys`
      0.4 hardcodes `pkg_config...statik(false)`, and ALSA `dlopen`s its plugins
      at runtime, so a clean static `libasound` (musl or otherwise) is not
      feasible. Resolved: the Linux target is **`x86_64-unknown-linux-gnu` with
      dynamic libasound**, not musl. The build installs `libasound2-dev` +
      `pkg-config` (via rust-ci's `system-packages` input); the Homebrew formula
      `depends_on "alsa-lib"` on Linux; runtime needs `libasound2` (present on any
      desktop). macOS is moot (CoreAudio, no ALSA).
- [x] **Repo public + public runners.** Went public 2026-07-13. CI/style/audit
      reverted from the self-hosted `studio-pdcst` runner to free hosted runners
      (`ci.yml` matrix Linux + macOS with `system-packages`). Follow-up (Jacob's
      infra box): remove the `pdcst` service from `jhheider/gha-runner` and
      `just clean-stale`.
- [x] **Release workflow.** `jhheider/rust-ci` `release.yml@v1` caller on
      `release: published`: builds `pdcst` for `x86_64-unknown-linux-gnu` +
      `aarch64/x86_64-apple-darwin`, attaches the archives, **publishes only the
      `wsola` crate** to crates.io (pdcst stays unpublished; the action skips
      already-published versions), and refreshes `Formula/pdcst.rb` in
      `jhheider/homebrew-tap`. Needed a new rust-ci input (`system-packages`,
      v1.5.0) so the Linux build can apt-install the ALSA headers.
- [x] **Version.** Reset to `0.2.0` earlier; `0.3.0` for the polish pass; `0.3.1`
      adds `--version` (clap) + the distribution wiring. `wsola` stays `0.1.0`,
      published independently. Identity unified to `pdcst` throughout.

## Product fitness (product-designer + tui-ux agents, 2026-07-12)

Both agents re-reviewed the feature-complete app. Verdict: coherent daily-driver
for its owner; the core loop closes end to end. Fixes landed this pass:

- [x] **Subscribe now fetches episodes.** `subscribe_from_search_result` spawns a
      `refresh_one`, so a fresh feed is not empty until the next scheduled
      refresh. (Was the #1 leverage bug - a stranger's first action produced a
      blank screen.)
- [x] **Empty Episodes view has guidance** ("press r to refresh").
- [x] **Actions no longer lie.** `a`/`d`/`x`/`s` return whether they acted; the
      status only shows on real work (pressing `a` on a subscription row no
      longer says "Added to queue").
- [x] **Unsubscribe** (`u` in Subscriptions; cascade-deletes episodes).
- [x] **Resize repaints** (the run loop drains all events per tick, handling
      `Event::Resize`; also fixes key-burst lag).
- [x] **Unmute restores the prior level** (not a hardcoded 0.5).
- [x] **README corrected** (it claimed the auto-queue/resume were "coming").
- [x] **Live smoke test** committed at `scripts/smoke.exp` (drives the TUI in a
      pty via `expect`; manual, needs a terminal).

- [x] **Auto-queue control surfaced.** A contextual footer shows the current
      view's actions, so `[A] Auto-queue` is visible in the Subscriptions view
      without opening Help; the first-run empty state also teaches "press 'A' on
      a feed to auto-fill Up Next." (Decided: personal project, so the toggle
      stays opt-in rather than defaulted on.)

Remaining for a public release (Jacob's calls, not built):

- Version reset, name unification, release workflow (above).
- Config discoverability: `Config` only loads from `--config <file>`; add
  `--print-config` or write a commented default on first run.
- **Public-vs-personal: decided - personal.** Ships as a personal tool others
  can build ("runs well for me, PRs welcome, no support"). The `wsola` crate is
  **published independently on crates.io** (0.1.0; generally useful and
  standalone).

Cosmetic papercuts (deferred): emoji in headers (cross-terminal width),
per-view footer hints, search-box arrow keys seek instead of editing.
(`Modal::Confirm` is no longer dead - the polish pass below wired it for the
feed-recovery prompt.)

## Polish pass (2026-07-12): the surrounding info/reliability layer

A follow-up pass after daily use against the fixture OPML found the core
(playback, WSOLA, queueing) solid but the surrounding information/reliability
layer thin. Brief: jhheider/briefs `pdcst-polish-pass`. All six findings fixed
(fmt + clippy `-D warnings` + 132 tests green):

- [x] **Feed refresh errors are honest + more feeds parse.** `FeedParser::
      parse_episodes` tries strict RSS 2.0, then falls back to Atom
      (`atom_syndication`, which reuses the existing `quick-xml 0.41` - no
      openssl/aws-lc). A `last_error` column (migration `20250712000002`) records
      the most recent failure per feed (set on failure, cleared on success); the
      subscription row shows a red `!` marker + the reason. The per-failure error
      *modal* is gone (a bulk refresh of dead URLs no longer spams dialogs).
      **Feed recovery**: `f` on a subscription searches iTunes by title for an
      up-to-date feed URL and, if it finds a different *title-matching* one,
      prompts (the now-wired `Modal::Confirm` + a `PendingAction`) to re-point the
      feed. Confirmed against the fixture: `Constitutionally Speaking`'s retired
      custom domain recovers to its live simplecast feed. When there's no
      confident title match, the Search view becomes a **picker** over the
      candidates (metadata cards: artist / genre / episode count / feed host) so
      the user chooses one to re-point; only a flat "none" if the search is empty.
- [x] **Episode cards.** The episode pane renders 2-3 line cards: markers +
      title, then relative date + duration + a `queued` badge (a per-render
      `HashSet` of queued ids, no schema change), then a tag-stripped description
      snippet. All from fields already in the model.
- [x] **Resume survives abrupt exit.** The 1s autosave tick already covered
      active playback; this adds a `SIGTERM`/`SIGHUP` handler (Unix) that runs the
      same save-then-break as the quit key, so `kill` / a closed terminal / an SSH
      drop still checkpoints. (Ctrl-C is a key event under raw mode, already saved.)
- [x] **Subscription counts.** `get_all_subscriptions` LEFT JOINs episodes for
      `episode_count`, `unplayed_count`, and `latest_episode_at` (transient
      `#[serde(skip)]` fields); the row shows `N new | M eps | <age>`.
- [x] **Panes, not tabs.** Subscriptions and Episodes are now the two panes of a
      single **Library** (`1`): left pane live-previews the highlighted feed into
      the right pane, each pane has its own cursor + scroll + focus border, and
      `l`/`Enter` and `h`/`Esc` cross between them (Left/Right stay seek). Backing
      out of a feed returns to the same subscription.
- [x] **No stray rodio warning on quit.** `log_on_drop(false)` on the device sink.

- [x] **WSOLA re-confirmed perfect in real use** (no action - closes the last
      open item in the `pure-rust-time-stretch` brief; every gap this pass was in
      the UI, never the DSP).

## Cross-cutting constraints (the ethos)

- **Pure Rust, lean deps, no C build systems** - except the SQLite amalgamation
  (Phase E) as a deliberate, contained exception for portability.
- **TLS: rustls + ring only.** Never openssl, native-tls, or aws-lc. reqwest is
  `default-features=false` + `rustls-no-provider` + `webpki-roots`, with ring
  installed once at startup via `pdcst::ensure_crypto_provider()` (called
  at every reqwest client site so tests, which never run `main`, work).
- **Design bar: a keyboard-first TUI** in the class of lazygit/k9s/newsboat.
  Reviewed with the user-level `tui-ux` agent.
- **CI on the self-hosted runner** (private repo); keep it green under
  fmt + clippy `-D warnings` + tests + the no-em-dash style gate. Match the local
  toolchain to the runner (1.97) so lints agree.

## Key technical reference (for a cold start)

- **Layout**: Cargo workspace. `crates/pdcst` (bin), `crates/wsola` (lib).
  Deps hoisted in root `Cargo.toml` `[workspace.dependencies]`.
- **Audio** (`crates/pdcst/src/audio/`):
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
  `crates/pdcst/migrations` (`migrate!("./migrations")`). Tables:
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
- ~~Auto-add default: push or unshift? Per-sub UI shape.~~ RESOLVED (Phase C
  PR3): per-sub `A` cycles off -> bottom -> top, defaulting off; global
  `auto_queue_to_top_default` in `Config`.
- ~~Smart-interleave algorithm: insert-at-nearest-legal vs periodic re-balance.~~
  RESOLVED (Phase C PR2): insert-at-nearest-legal via the pure
  `nearest_legal_position`.
- Whether to pursue a musl fully-static Linux build now or after the product is
  proven. (Still open; Phase E ships glibc + dynamic libasound, which is fine.)

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
