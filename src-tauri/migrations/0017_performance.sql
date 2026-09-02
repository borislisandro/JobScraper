-- Listings-first updates keep enough state to enrich descriptions after the cheap board read.
-- Existing rows already came from eager scrapes, so they remain complete.
ALTER TABLE jobs ADD COLUMN detail_url TEXT;
ALTER TABLE jobs ADD COLUMN description_status TEXT NOT NULL DEFAULT 'complete'
  CHECK(description_status IN ('complete','pending','failed'));
ALTER TABLE jobs ADD COLUMN description_error TEXT;
ALTER TABLE jobs ADD COLUMN description_checked_at TEXT;

-- A job can appear more than once on a board (Apple repeats positions by location), but a sighting
-- means only that the job was present in this run. Keep one row for that fact and enforce it.
DELETE FROM job_occurrences
WHERE rowid NOT IN (
  SELECT min(rowid) FROM job_occurrences GROUP BY job_id,run_id
);
CREATE UNIQUE INDEX IF NOT EXISTS job_occurrences_job_run_idx
ON job_occurrences(job_id,run_id);

-- Jobs are shown globally, not grouped by source. These indexes serve the two supported sorts,
-- while application lookup serves both the saved-only filter and the stage shown on each card.
CREATE INDEX IF NOT EXISTS jobs_updated_idx ON jobs(updated_at DESC,id);
CREATE INDEX IF NOT EXISTS jobs_posted_idx ON jobs(posted_at IS NULL,posted_at DESC,updated_at DESC,id);
CREATE INDEX IF NOT EXISTS applications_job_created_idx ON applications(job_id,created_at DESC);
CREATE INDEX IF NOT EXISTS scrape_runs_source_started_idx ON scrape_runs(source_id,started_at DESC);

-- Search uses literal substring matching because punctuation such as C++ must remain searchable.
-- The old token FTS index is never read and rebuilt itself on every job update.
DROP TRIGGER IF EXISTS jobs_au;
DROP TRIGGER IF EXISTS jobs_ad;
DROP TRIGGER IF EXISTS jobs_ai;
DROP TABLE IF EXISTS jobs_fts;
