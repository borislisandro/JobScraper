-- A copy of each title lower-cased and stripped of accents (locations::fold_text), so a search for
-- "automacao" or "automação" finds "AUTOMAÇÃO". SQLite's lower() only knows ASCII. Filled in by
-- persist_worker_job_in for new rows and backfilled at startup (backfill_folded_titles).
ALTER TABLE jobs ADD COLUMN title_folded TEXT;
