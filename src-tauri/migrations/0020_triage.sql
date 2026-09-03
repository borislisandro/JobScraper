-- Two things the Jobs page could not say, on a list that only ever grows.
--
-- dismissed_at: "not for me", decided by the person reading it. Kept as a timestamp rather than a
-- flag so the list can be ordered by when it was set and so undoing is a single UPDATE to NULL.
-- Dismissing never deletes: the row stays scraped, searchable and countable, it simply leaves the
-- default view.
ALTER TABLE jobs ADD COLUMN dismissed_at TEXT;
CREATE INDEX IF NOT EXISTS jobs_dismissed ON jobs(dismissed_at);
-- first_seen_at: when this listing first arrived. created_at already means that, but it is also
-- what the reconcile and repair paths touch, so "new since I last looked" needs a column nothing
-- else writes. Existing rows inherit the date they were created.
ALTER TABLE jobs ADD COLUMN first_seen_at TEXT;
UPDATE jobs SET first_seen_at=created_at WHERE first_seen_at IS NULL;
CREATE INDEX IF NOT EXISTS jobs_first_seen ON jobs(first_seen_at DESC);
