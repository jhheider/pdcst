-- Per-podcast playback speed: 1.0 = normal. NULL means use the global default.
ALTER TABLE subscriptions ADD COLUMN playback_rate REAL;

-- down:
-- ALTER TABLE subscriptions DROP COLUMN playback_rate;
