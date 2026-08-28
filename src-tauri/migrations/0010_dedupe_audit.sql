CREATE TABLE IF NOT EXISTS job_dedupe_events (
  id TEXT PRIMARY KEY NOT NULL,
  canonical_job_id TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
  method TEXT NOT NULL,
  evidence_json TEXT NOT NULL,
  created_at TEXT NOT NULL
) STRICT;
CREATE TABLE IF NOT EXISTS duplicate_merge_conflicts (
  id TEXT PRIMARY KEY NOT NULL,
  audit_id TEXT NOT NULL REFERENCES duplicate_merge_audits(id) ON DELETE CASCADE,
  conflict_type TEXT NOT NULL CHECK(conflict_type IN ('match_result','review_decision')),
  persona_id TEXT NOT NULL,
  canonical_snapshot_json TEXT NOT NULL,
  merged_snapshot_json TEXT NOT NULL,
  resolution TEXT NOT NULL DEFAULT 'preserved_separately',
  created_at TEXT NOT NULL,
  resolved_at TEXT
) STRICT;
