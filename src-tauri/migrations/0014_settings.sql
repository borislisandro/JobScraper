-- Migration 0012 dropped `settings` because a traffic audit found nothing read or wrote it.
-- The one-time robots.txt acknowledgement is the first real consumer: it is a user decision that
-- must outlive a restart and apply to every source, so it needs a home that is backed up and
-- exported like the rest of the user's data.
CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL, updated_at TEXT NOT NULL) STRICT;
