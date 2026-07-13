-- Polish pass: record the most recent refresh failure per subscription so the
-- UI can surface *why* a feed is not updating (a dead URL, a non-standard XML
-- parse) right in its row, instead of only in a transient status message that
-- scrolls away. NULL means the last refresh succeeded (or none has run yet).
ALTER TABLE subscriptions ADD COLUMN last_error TEXT;
