CREATE TABLE IF NOT EXISTS application_attempts (
  id TEXT PRIMARY KEY NOT NULL,
  application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
  job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE RESTRICT,
  opened_at TEXT NOT NULL,
  original_stage TEXT NOT NULL,
  focus_cycle INTEGER NOT NULL DEFAULT 0,
  resolved_at TEXT,
  resolution TEXT CHECK(resolution IN ('yes','no','not_yet'))
) STRICT;
CREATE INDEX IF NOT EXISTS application_attempts_open_idx ON application_attempts(application_id,resolved_at,opened_at DESC);
ALTER TABLE reminders ADD COLUMN entity_id TEXT;
ALTER TABLE reminders ADD COLUMN os_task_id TEXT;
ALTER TABLE reminders ADD COLUMN last_reconciled_at TEXT;
ALTER TABLE reminders ADD COLUMN last_error TEXT;
