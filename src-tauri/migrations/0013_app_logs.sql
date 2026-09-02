-- Activity log for everything that happens before a scrape_run exists. scrape_run_events can
-- only describe test/scrape runs on an already-saved source (scrape_runs.mode is CHECK'd to
-- ('test','scrape')), so detection failures, worker crashes and robots overrides — exactly the
-- events a user needs to see when adding a source fails — had nowhere to go.
CREATE TABLE IF NOT EXISTS app_logs (
  id TEXT PRIMARY KEY NOT NULL, at TEXT NOT NULL, level TEXT NOT NULL CHECK(level IN ('info','warning','error')),
  source_id TEXT, source_name TEXT, action TEXT NOT NULL, code TEXT, message TEXT NOT NULL,
  detail_json TEXT NOT NULL DEFAULT '{}'
) STRICT;
CREATE INDEX IF NOT EXISTS app_logs_at ON app_logs(at DESC);
