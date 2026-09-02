-- Where a listing is, as ISO country codes, decided once when it is stored rather than matched as
-- text on every query: a board writes "Lisbon" or "Taichung - Fab 16 Taiwan", and the same office
-- name exists in several countries. src-tauri/src/locations.rs does the resolving.
-- ",PT,ES," so one code can be tested with a plain substring match. Empty means "resolved, nothing
-- recognisable"; NULL means "not looked at yet" and is what the startup backfill looks for.
ALTER TABLE jobs ADD COLUMN location_countries TEXT;
CREATE INDEX IF NOT EXISTS jobs_location_countries ON jobs(location_countries);
