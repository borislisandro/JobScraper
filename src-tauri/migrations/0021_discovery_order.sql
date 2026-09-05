-- Refreshing an existing vacancy must not move it above newly discovered jobs.
-- The expression also covers captured or legacy rows without first_seen_at.
CREATE INDEX IF NOT EXISTS jobs_discovery_order_idx
    ON jobs(coalesce(first_seen_at,created_at) DESC,id);
