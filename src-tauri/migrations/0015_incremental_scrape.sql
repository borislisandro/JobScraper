-- Re-scraping cost 2.83 s per job on every Workday tenant because the worker fetched a detail
-- page for every listing it saw, whether or not that job was already stored unchanged. These
-- three columns are what let a run skip that work.
--
-- listing_hash is the job as its LISTING row described it — title, location, posted date — which
-- is the most that can be known before deciding whether the detail page is worth fetching. The
-- worker computes it, the row keeps it, and the next run compares against it. content_hash
-- already existed and covers the description; it is the second gate, on the write side.
ALTER TABLE jobs ADD COLUMN listing_hash TEXT;

-- The run counters were declared in 0001 and never written, so there was no record of how much
-- a run actually did. skipped_count is the whole point of the exercise: it is the number of jobs
-- that were recognised and left alone, and requests_count is what that saved in network calls.
ALTER TABLE scrape_runs ADD COLUMN skipped_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE scrape_runs ADD COLUMN requests_count INTEGER NOT NULL DEFAULT 0;

-- reconcile_availability runs two `id IN (SELECT job_id FROM job_occurrences WHERE run_id=?)`
-- subqueries per source per run against a table that grows by one row per job per run. Without
-- this index both scan the whole table, so every run got slower than the last one forever.
CREATE INDEX IF NOT EXISTS job_occurrences_run_idx ON job_occurrences(run_id);

-- The incremental path looks a job up by the canonical URL its listing row implies, so that
-- lookup has to be an index seek rather than a scan. jobs_canonical_idx already covers the
-- URL itself; this one serves the per-source sweep that builds a run's known-jobs map.
CREATE INDEX IF NOT EXISTS jobs_source_listing_idx ON jobs(source_id, listing_hash);
