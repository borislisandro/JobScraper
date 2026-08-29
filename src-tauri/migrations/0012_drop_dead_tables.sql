-- Traffic audit (INSERT/UPDATE/SELECT across src-tauri/src) showed these four tables are
-- created and never meaningfully used:
--   persona_filters  - 0 ins / 0 upd / 0 read; persona filters actually live as columns on
--                       personas.
--   duplicate_groups - 1 ins / 0 upd / 0 read; dedupe review actually runs off
--                       duplicate_candidates.
--   job_aliases       - 1 ins / 1 upd / 0 domain read (only ever pulled into relational
--                       export dumps, never queried by app logic).
--   settings          - 0 ins / 0 upd / 0 read; seeded once with ghosted_threshold_days=14
--                       by migration 0002, but the ghost reminder default is a separate
--                       hardcoded 14 in db.rs and update_ghost_threshold takes an explicit
--                       per-application override. Nothing reads this table.
-- job_revisions and scrape_run_events are intentionally kept (see execution plan notes).
DROP TABLE IF EXISTS persona_filters;
DROP TABLE IF EXISTS duplicate_groups;
DROP TABLE IF EXISTS job_aliases;
DROP TABLE IF EXISTS settings;
