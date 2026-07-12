# pdcst

A fast, keyboard-driven terminal podcast player, in pure Rust. Single binary,
one user, built to actually live in.

Unapologetically inspired by **the inestimable [Pocket Casts](https://pocketcasts.com/)**,
whose auto-managed Up Next queue is the thing this project exists to reproduce:
subscriptions that fill the queue for you at publish time, kept topped up to a
depth you choose, smartly interleaved so you never get a run of the same show,
never clobbering what is playing, with listen state tracked for every episode.
On a commute that queue was the whole game. That is the goal here.

Not the goal: cloud sync (impossible in a local app), discovery, or a
settings-heavy UI. The edge is simple - mine, fast, self-contained, does not
crash, and keeps my queue full.

## Status

Feature-complete for its purpose and in daily-driver shape. The whole staged
plan has landed:

- **Audio**: play/pause, real seek, **pitch-corrected** speed (1.5x with no
  chipmunk), cross-session resume, progressive stream-to-disk (starts fast, no
  wait for the full download), and bounded on-disk caching (delete-on-finish +
  size caps).
- **The auto-queue** (the point): mark a feed with `A` and new episodes fill Up
  Next automatically - pushed or unshifted per feed, capped at a max depth,
  smartly interleaved so you never get a run of one show, never clobbering what
  is playing. Listen state (unplayed / in-progress / played) is tracked
  everywhere.
- **Hands-off refresh**: on launch and on an interval, so the queue stays full
  without you touching it.
- **Keyboard UX**: scrolling lists, in-app iTunes search and subscribe, an
  operable queue (play / remove / skip), now-playing and listen-state markers.

Also here: subscribe via RSS, OPML import/export. Still to do: distribution and
packaging (a static single-binary release), and a handful of UX papercuts -
tracked in **[docs/ROADMAP.md](docs/ROADMAP.md)**, the canonical plan and a
technical map for picking the work up cold.

This is a personal project that runs well for its author; it is shared to read
and build, with no support promised.

## Layout

A Cargo workspace with two independently-versioned sibling crates:

- `crates/pdcst` - the player binary.
- `crates/wsola` - a standalone pure-Rust [WSOLA](https://en.wikipedia.org/wiki/Audio_time_stretching_and_pitch_scaling)
  time-stretch library (pitch-preserved tempo: 1.5x without the chipmunk
  effect). Born from this player's need for real podcast speed; it will publish
  on its own.

## Building

Needs stable Rust. On Linux, rodio needs ALSA at build time:

```bash
# Debian/Ubuntu
sudo apt-get install libasound2-dev pkg-config
# Fedora: alsa-lib-devel   Arch: alsa-lib
```

```bash
cargo build --release
```

## Using it

```bash
# Import your subscriptions (e.g. exported from Pocket Casts) and exit
./target/release/pdcst --import podcasts.opml

# Run it
./target/release/pdcst

# Options: --config <file>, --debug, --export <file>
```

## Development

```bash
cargo test --workspace
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all
```

CI runs the same gates (plus coverage and a plain-ASCII prose check) as thin
callers over [jhheider/rust-ci](https://github.com/jhheider/rust-ci) on a
self-hosted runner. TLS is rustls + ring only, no OpenSSL/aws-lc.

## Acknowledgments

Pocket Casts, for showing what a podcast queue should feel like. And the Rust
audio and TUI ecosystems - ratatui, crossterm, rodio, symphonia, sqlx, tokio,
reqwest - that make a single small binary like this possible.

## License

MIT. See [LICENSE](LICENSE).
