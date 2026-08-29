CREATE TABLE IF NOT EXISTS data_purge_audits (
  id TEXT PRIMARY KEY NOT NULL,
  category TEXT NOT NULL,
  preview_hash TEXT NOT NULL,
  impact_json TEXT NOT NULL,
  file_cleanup_json TEXT NOT NULL DEFAULT '[]',
  created_at TEXT NOT NULL
) STRICT;

CREATE INDEX IF NOT EXISTS data_purge_audits_created_idx
  ON data_purge_audits(created_at DESC);
