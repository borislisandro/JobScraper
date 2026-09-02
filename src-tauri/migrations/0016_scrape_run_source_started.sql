-- Warm update preflight asks for the newest completed baseline for every source. Keep that
-- lookup bounded as scrape history grows; this is an additive, rollback-safe index only.
CREATE INDEX IF NOT EXISTS scrape_runs_source_started_idx
ON scrape_runs(source_id, started_at DESC);
