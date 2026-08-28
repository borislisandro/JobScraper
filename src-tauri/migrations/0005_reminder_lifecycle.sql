-- `delivered` is distinct from `missed`: scheduler delivery was confirmed by
-- reminder-only startup, whereas missed is shown once during later reconcile.
ALTER TABLE reminders RENAME TO reminders_v4;
CREATE TABLE reminders (
  id TEXT PRIMARY KEY NOT NULL,
  application_id TEXT NOT NULL REFERENCES applications(id) ON DELETE CASCADE,
  reminder_type TEXT NOT NULL CHECK(reminder_type IN ('ghosted','interview')),
  entity_id TEXT,
  due_at TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('pending','delivered','cancelled','missed')),
  os_task_id TEXT,
  last_reconciled_at TEXT,
  last_error TEXT,
  created_at TEXT NOT NULL
) STRICT;
INSERT INTO reminders(id,application_id,reminder_type,entity_id,due_at,status,os_task_id,last_reconciled_at,last_error,created_at)
SELECT id,application_id,reminder_type,entity_id,due_at,
  CASE WHEN status='sent' THEN 'delivered' ELSE status END,
  os_task_id,last_reconciled_at,last_error,created_at
FROM reminders_v4;
DROP TABLE reminders_v4;
CREATE INDEX IF NOT EXISTS reminders_pending_due_idx ON reminders(status,due_at);
CREATE INDEX IF NOT EXISTS reminders_entity_idx ON reminders(entity_id,status);

ALTER TABLE interviews ADD COLUMN updated_at TEXT;
UPDATE interviews SET updated_at=created_at WHERE updated_at IS NULL;
