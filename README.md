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

In active development, and honest about it: the backend (audio, downloads, feed
parsing, database) is solid and tested; the UI is wired but has real gaps; the
auto-queue - the point of the whole thing - is not built yet. Do not trust a
checklist here; trust **[docs/ROADMAP.md](docs/ROADMAP.md)**, which is the real
plan (the auto-queue is the north star) and a technical map for picking the work
up cold.

What works today: subscribe via RSS, OPML import/export, iTunes search, a manual
queue, and playback (play/pause/seek/**pitch-corrected** speed/volume) with
downloads. What is coming: the auto-queue, resume, and a proper pass over the
keyboard UX. See the roadmap for the staged plan.

## Layout

A Cargo workspace with two independently-versioned sibling crates:

- `crates/podcast-tui` - the player binary.
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
./target/release/podcast-tui --import podcasts.opml

# Run it
./target/release/podcast-tui

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
