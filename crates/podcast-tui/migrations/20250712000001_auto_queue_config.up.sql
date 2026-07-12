-- Phase C: per-subscription auto-add direction. `auto_queue` (already present)
-- is the on/off switch; this says whether new episodes go to the top (unshift)
-- or the bottom (push) of the queue. 0 = bottom (push), 1 = top (unshift).
ALTER TABLE subscriptions ADD COLUMN auto_queue_to_top BOOLEAN NOT NULL DEFAULT 0;
