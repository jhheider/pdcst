-- Mark-seen support: an episode is "new" (unacknowledged) until you play it,
-- finish it, or explicitly mark it seen. Distinct from `played` (listened), so
-- you can clear the import backlog - OPML carries no listen history, so every
-- imported episode would otherwise read as "new" forever - without pretending
-- you listened. The subscription "N new" count is SUM(is_new).
ALTER TABLE episodes ADD COLUMN is_new BOOLEAN NOT NULL DEFAULT 1;
