-- Per-subscription episode order: newest-first (default) suits news / current
-- affairs; oldest-first suits serial / narrative feeds (Revolutions, a history
-- series) where you catch up in publication order. Drives the episode-list
-- display (and hence the order you queue a backlog). Guessed at subscribe time,
-- always overridable with the `O` key.
ALTER TABLE subscriptions ADD COLUMN queue_oldest_first BOOLEAN NOT NULL DEFAULT 0;
