# Podcast TUI

A terminal-based podcast player with PocketCasts feature parity, built in Rust.

## Features

### Core Functionality
- ✅ **Subscription Management**: Subscribe to podcasts via RSS feeds
- ✅ **OPML Import/Export**: Migrate from other podcast apps
- ✅ **Queue Management**: Build and organize your listening queue
- ✅ **Playback Controls**: Play, pause, adjust speed and volume
- ✅ **In-Memory Streaming**: Stream episodes without caching to disk
- ✅ **Offline Downloads**: Download episodes for offline listening
- ✅ **Podcast Search**: Search for new podcasts via iTunes API
- ✅ **Position Tracking**: Resume episodes where you left off
- ✅ **Concurrent Feed Refresh**: Efficiently update multiple subscriptions

### Advanced Features
- ✅ **Artwork Support**: Terminal graphics support (Sixel, Kitty, iTerm2)
- ✅ **Configurable UI**: Themes and keybindings
- ✅ **Smart Queue Ordering**: Multiple queue sorting strategies
- ✅ **Playback Speed Control**: Variable playback speeds (0.5x to 3x)
- ✅ **Episode Filters**: Filter by played/unplayed status
- ✅ **Auto-download**: Automatically download new episodes from subscriptions

## Installation

### Prerequisites
- Rust 1.70 or later
- ALSA development libraries (Linux only):
  ```bash
  # Ubuntu/Debian
  sudo apt-get install libasound2-dev pkg-config

  # Fedora
  sudo dnf install alsa-lib-devel

  # Arch
  sudo pacman -S alsa-lib
  ```

### Build from Source
```bash
git clone https://github.com/yourusername/podcast-tui.git
cd podcast-tui
cargo build --release
./target/release/podcast-tui
```

## Usage

### Basic Commands
```bash
# Start the application
podcast-tui

# Use a custom config file
podcast-tui --config /path/to/config.toml

# Enable debug logging
podcast-tui --debug
```

### Keybindings

#### Global
- `q` - Quit application
- `1` - Go to Subscriptions view
- `2` - Go to Queue view
- `3` - Go to Search view
- `Space` - Play/Pause
- `↑/↓` - Navigate lists
- `Enter` - Select item

#### Playback
- `Space` - Play/Pause
- `[` - Decrease speed
- `]` - Increase speed
- `+` - Increase volume
- `-` - Decrease volume

## Configuration

The default configuration is stored at `~/.local/share/podcast-tui/config.toml`.

### Example Configuration
```toml
[storage]
data_dir = "~/.local/share/podcast-tui"
download_dir = "~/Downloads/Podcasts"

[playback]
default_playback_rate = 1.0
skip_forward_seconds = 30
skip_backward_seconds = 10
save_position_interval_seconds = 10

[sync]
auto_refresh_interval_minutes = 60
max_concurrent_refreshes = 5
max_concurrent_downloads = 3

[ui]
show_artwork = true

[theme]
name = "default"
```

## Architecture

### Technology Stack
- **UI**: ratatui + crossterm
- **Audio**: rodio
- **Database**: SQLite (sqlx)
- **HTTP**: reqwest
- **RSS**: rss crate
- **Async Runtime**: tokio

### Project Structure
```
podcast-tui/
├── src/
│   ├── app/           # Application state and event handling
│   ├── audio/         # Audio playback and streaming
│   ├── artwork/       # Terminal graphics for podcast art
│   ├── download/      # Episode download manager
│   ├── feed/          # RSS feed parsing and refresh
│   ├── models/        # Data models
│   ├── queue/         # Queue management
│   ├── search/        # Podcast search (iTunes API)
│   ├── storage/       # SQLite database layer
│   ├── ui/            # TUI components and rendering
│   └── utils/         # Utilities (logging, formatting)
├── migrations/        # Database migrations
└── tests/            # Integration tests
```

## Data Storage

- **Database**: `~/.local/share/podcast-tui/podcast.db`
- **Downloads**: `~/Downloads/Podcasts/`
- **Artwork Cache**: `~/.local/share/podcast-tui/artwork/`
- **Logs**: `~/.local/share/podcast-tui/logs/`

## Importing Subscriptions

Export your subscriptions from your current podcast app as OPML, then:

1. Start podcast-tui
2. Navigate to Settings
3. Select "Import OPML"
4. Choose your OPML file

## Development

### Running Tests
```bash
cargo test
```

### Code Quality
```bash
# Format code
cargo fmt

# Run linter
cargo clippy -- -D warnings

# Check for errors
cargo check
```

### Building for Release
```bash
cargo build --release
```

## Known Limitations

- **Seeking**: rodio doesn't support seeking within audio streams. Skip forward/backward is not yet implemented
- **Video Podcasts**: Only audio podcasts are supported
- **Sync**: No cloud sync between devices (local only)

## Roadmap

### Phase 4 (Polish)
- [ ] Configurable keybindings UI
- [ ] Theme editor
- [ ] Silence trimming
- [ ] Better error recovery
- [ ] Performance optimizations
- [ ] Comprehensive test coverage

### Future Enhancements
- [ ] PodcastIndex API integration
- [ ] Chapter support
- [ ] Episode notes/shownotes viewer
- [ ] Playlist support
- [ ] Cloud sync (optional)

## Contributing

Contributions are welcome! Please:

1. Fork the repository
2. Create a feature branch
3. Make your changes
4. Run tests and linters
5. Submit a pull request

## License

MIT License - see LICENSE file for details

## Acknowledgments

Built with the amazing Rust ecosystem:
- ratatui - Terminal UI framework
- rodio - Audio playback
- sqlx - Type-safe SQL
- tokio - Async runtime
- And many more excellent crates!
