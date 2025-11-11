-- Create subscriptions table
CREATE TABLE IF NOT EXISTS subscriptions (
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

-- Create episodes table
CREATE TABLE IF NOT EXISTS episodes (
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

CREATE INDEX IF NOT EXISTS idx_episodes_subscription ON episodes(subscription_id);
CREATE INDEX IF NOT EXISTS idx_episodes_published ON episodes(published_at DESC);
CREATE INDEX IF NOT EXISTS idx_episodes_played ON episodes(played);

-- Create queue table
CREATE TABLE IF NOT EXISTS queue_items (
    id TEXT PRIMARY KEY,
    episode_id TEXT NOT NULL,
    position INTEGER NOT NULL,
    priority TEXT NOT NULL DEFAULT 'Medium',
    added_at DATETIME NOT NULL,
    FOREIGN KEY (episode_id) REFERENCES episodes(id) ON DELETE CASCADE,
    UNIQUE(episode_id)
);

CREATE INDEX IF NOT EXISTS idx_queue_position ON queue_items(position);

-- Create playback state table
CREATE TABLE IF NOT EXISTS playback_state (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    current_episode_id TEXT,
    position_seconds REAL NOT NULL DEFAULT 0,
    playback_rate REAL NOT NULL DEFAULT 1.0,
    volume REAL NOT NULL DEFAULT 1.0,
    status TEXT NOT NULL DEFAULT 'Stopped',
    updated_at DATETIME NOT NULL,
    last_position_save DATETIME,
    FOREIGN KEY (current_episode_id) REFERENCES episodes(id) ON DELETE SET NULL
);

-- Initialize playback state
INSERT OR IGNORE INTO playback_state (id, position_seconds, playback_rate, volume, status, updated_at)
VALUES (1, 0, 1.0, 1.0, 'Stopped', CURRENT_TIMESTAMP);

-- Create config table
CREATE TABLE IF NOT EXISTS config (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    data TEXT NOT NULL
);
